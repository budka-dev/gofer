use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use thiserror::Error;
use tokio::sync::{RwLock, Semaphore};

use super::watcher::EmbeddingConfig;

#[derive(Error, Debug)]
pub enum EmbedderError {
    #[error("Embedding error: {0}")]
    Embedding(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, EmbedderError>;

struct Embedder {
    client: reqwest::Client,
    url: String,
    api_key: Option<String>,
    model: Option<String>,
}

impl Embedder {
    fn new(config: &EmbeddingConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| EmbedderError::Embedding(e.into()))?;

        let url = config
            .external_url
            .clone()
            .unwrap_or_else(|| "http://127.0.0.1:8080/embed/".to_string());

        Ok(Self {
            client,
            url,
            api_key: config.external_api_key.clone(),
            model: config.external_model.clone(),
        })
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let mut payload = serde_json::json!({
            "inputs": texts.clone(),
            "input": texts,
        });

        if let Some(m) = &self.model {
            payload["model"] = serde_json::Value::String(m.clone());
        }

        let mut req = self.client.post(&self.url).json(&payload);

        if let Some(key) = &self.api_key {
            req = req.header("x-api-key", key);
        }

        let resp = req.send().await.map_err(|e| EmbedderError::Embedding(e.into()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(EmbedderError::Embedding(anyhow::anyhow!(
                "External embedder error ({}): {}",
                status,
                text
            )));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| EmbedderError::Embedding(e.into()))?;

        let embeddings = if let Some(arr) = data.as_array() {
            // direct array of arrays
            parse_embeddings(arr)?
        } else if let Some(arr) = data.get("embeddings").and_then(|e| e.as_array()) {
            parse_embeddings(arr)?
        } else if let Some(arr) = data.get("data").and_then(|e| e.as_array()) {
            // OpenAI style: data[i].embedding
            let mut embs = Vec::with_capacity(arr.len());
            for item in arr {
                if let Some(emb) = item.get("embedding").and_then(|e| e.as_array()) {
                    let vec: std::result::Result<Vec<f32>, _> = emb
                        .iter()
                        .map(|v| {
                            v.as_f64()
                                .map(|f| f as f32)
                                .ok_or_else(|| anyhow::anyhow!("Invalid f32 in embedding"))
                        })
                        .collect();
                    embs.push(vec.map_err(EmbedderError::Embedding)?);
                } else {
                    return Err(EmbedderError::Embedding(anyhow::anyhow!(
                        "Invalid embedding format: missing 'embedding' field"
                    )));
                }
            }
            embs
        } else {
            return Err(EmbedderError::Embedding(anyhow::anyhow!(
                "Unknown response format"
            )));
        };

        Ok(embeddings)
    }
}

fn parse_embeddings(arr: &[serde_json::Value]) -> Result<Vec<Vec<f32>>> {
    let mut embs = Vec::with_capacity(arr.len());
    for item in arr {
        if let Some(inner) = item.as_array() {
            let vec: std::result::Result<Vec<f32>, _> = inner
                .iter()
                .map(|v| {
                    v.as_f64()
                        .map(|f| f as f32)
                        .ok_or_else(|| anyhow::anyhow!("Invalid f32 in embedding array"))
                })
                .collect();
            embs.push(vec.map_err(EmbedderError::Embedding)?);
        } else {
            return Err(EmbedderError::Embedding(anyhow::anyhow!(
                "Invalid embedding format: expected array of arrays"
            )));
        }
    }
    Ok(embs)
}

pub struct EmbedderPool {
    instances: RwLock<Vec<Arc<Embedder>>>,
    semaphore: Arc<RwLock<Arc<Semaphore>>>,
    pool_size: AtomicUsize,
    next_idx: AtomicUsize,
    model_dimension: usize,
    model_name: String,
}

impl EmbedderPool {
    #[allow(dead_code)]
    pub fn new(size: usize) -> Result<Self> {
        Self::with_config(size, &EmbeddingConfig::default())
    }

    pub fn with_config(size: usize, config: &EmbeddingConfig) -> Result<Self> {
        let size = size.clamp(1, 8);

        let model_dimension = config.dimensions.unwrap_or(1024);
        let model_name = config
            .external_model
            .clone()
            .unwrap_or_else(|| "external_model".to_string());

        tracing::info!(
            "Инициализация embedder pool с внешним API: {} инстансов, URL={:?}",
            size,
            config.external_url
        );

        let mut instances = Vec::with_capacity(size);
        for _ in 0..size {
            let embedder = Embedder::new(config)?;
            instances.push(Arc::new(embedder));
        }

        Ok(Self {
            instances: RwLock::new(instances),
            semaphore: Arc::new(RwLock::new(Arc::new(Semaphore::new(size)))),
            pool_size: AtomicUsize::new(size),
            next_idx: AtomicUsize::new(0),
            model_dimension,
            model_name,
        })
    }

    pub async fn scale_up(&self, _target_size: usize) -> Result<()> {
        Ok(())
    }

    pub async fn scale_down(&self, _target_size: usize) -> Result<()> {
        Ok(())
    }

    #[allow(dead_code)]
    pub fn current_size(&self) -> usize {
        self.pool_size.load(Ordering::Acquire)
    }

    pub async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let semaphore = {
            let guard = self.semaphore.read().await;
            guard.clone()
        };

        let _permit = semaphore
            .acquire()
            .await
            .map_err(|_| EmbedderError::Embedding(anyhow::anyhow!("Semaphore закрыт")))?;

        let pool_size = self.pool_size.load(Ordering::Acquire);
        let idx = self.next_idx.fetch_add(1, Ordering::Relaxed) % pool_size;

        let embedder = {
            let instances = self.instances.read().await;
            instances.get(idx).cloned()
        };

        let embedder = embedder.ok_or_else(|| {
            EmbedderError::Embedding(anyhow::anyhow!("Embedder инстанс {} недоступен", idx))
        })?;

        embedder.embed(texts).await
    }

    pub async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let embeddings = self.embed(vec![text.to_string()]).await?;
        Ok(embeddings.into_iter().next().unwrap_or_default())
    }

    pub fn dimension(&self) -> usize {
        self.model_dimension
    }

    pub fn cache_version_key(&self) -> String {
        format!("{}:{}", self.model_name, self.model_dimension)
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_creation() {
        let pool = EmbedderPool::new(2);
        assert!(pool.is_ok());
        let pool = pool.unwrap();
        assert_eq!(pool.current_size(), 2);
    }

    #[test]
    fn test_dimension() {
        let config = EmbeddingConfig {
            provider: "external".to_string(),
            batch_size: 32,
            pool_size: 1,
            external_url: None,
            external_api_key: None,
            external_model: None,
            dimensions: Some(1024),
        };
        let pool = EmbedderPool::with_config(1, &config).unwrap();
        assert_eq!(pool.dimension(), 1024);
    }

    #[test]
    fn test_cache_version_key() {
        let config = EmbeddingConfig {
            provider: "external".to_string(),
            batch_size: 32,
            pool_size: 1,
            external_url: None,
            external_api_key: None,
            external_model: Some("BAAI/bge-m3".to_string()),
            dimensions: Some(1024),
        };
        let pool = EmbedderPool::with_config(1, &config).unwrap();
        assert_eq!(pool.cache_version_key(), "BAAI/bge-m3:1024");
    }

    #[tokio::test]
    async fn test_embed_empty_input() {
        let pool = EmbedderPool::new(1).unwrap();
        let result = pool.embed(vec![]).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_round_robin_index() {
        let pool = EmbedderPool::new(4).unwrap();
        let pool_size = pool.current_size();

        let idx0 = pool.next_idx.fetch_add(1, Ordering::Relaxed) % pool_size;
        let idx1 = pool.next_idx.fetch_add(1, Ordering::Relaxed) % pool_size;
        let idx2 = pool.next_idx.fetch_add(1, Ordering::Relaxed) % pool_size;
        let idx3 = pool.next_idx.fetch_add(1, Ordering::Relaxed) % pool_size;
        let idx4 = pool.next_idx.fetch_add(1, Ordering::Relaxed) % pool_size;

        assert_eq!(idx0, 0);
        assert_eq!(idx1, 1);
        assert_eq!(idx2, 2);
        assert_eq!(idx3, 3);
        assert_eq!(idx4, 0);
    }
}
