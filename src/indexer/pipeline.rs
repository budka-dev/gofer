//! SEDA (Staged Event-Driven Architecture) pipeline for high-throughput indexing.
//!
//! Stages: Scanner → Parser Pool → Batcher → Embedder → Writer
//! Connected via bounded tokio::mpsc channels with backpressure.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::Result;
use smol_str::SmolStr;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use super::domains::{detect_domain, DomainConfig};
use super::embedder::EmbedderPool;
use super::parser::{CodeParser, SupportedLanguage};
use crate::daemon::state::SyncProgress;
use crate::models::{CodeChunk, ImportInfo, Symbol, SymbolReference};
use crate::storage::{LanceStorage, SqliteStorage};

// ---------------------------------------------------------------------------
// Message types between pipeline stages
// ---------------------------------------------------------------------------

/// Scanner → Parser
struct ScannedFile {
    path: String,
    content: Arc<String>,
    hash: String,
    modified: i64,
    language: SupportedLanguage,
}

/// Parser → Batcher
struct ParsedDoc {
    path: String,
    hash: String,
    modified: i64,
    language: SupportedLanguage,
    symbols: Vec<Symbol>,
    chunks: Vec<CodeChunk>,
    refs: Vec<SymbolReference>,
    imports: Vec<ImportInfo>,
    domain: SmolStr,
    tech_stack: Vec<SmolStr>,
    content: Arc<String>,
}

/// Batcher → Embedder
struct ChunkBatch {
    chunks: Vec<CodeChunk>,
    metadata: Vec<ParsedFileMetadata>,
}

/// Embedder → Writer
struct EmbeddedBatch {
    chunks: Vec<CodeChunk>,
    embeddings: Vec<Vec<f32>>,
    metadata: Vec<ParsedFileMetadata>,
}

/// File metadata without chunks — kept for writer + cross-stack collection
#[derive(Debug)]
pub struct ParsedFileMetadata {
    pub path: String,
    pub hash: String,
    pub modified: i64,
    pub language: SupportedLanguage,
    pub symbols: Vec<Symbol>,
    pub refs: Vec<SymbolReference>,
    pub imports: Vec<ImportInfo>,
    pub domain: SmolStr,
    pub tech_stack: Vec<SmolStr>,
    pub content: Arc<String>,
}

// ---------------------------------------------------------------------------
// Pipeline orchestrator
// ---------------------------------------------------------------------------

const BATCH_CHUNK_SIZE_BASE: usize = 96;
const BATCH_CHUNK_SIZE_MIN: usize = 32;
const BATCH_CHUNK_SIZE_MAX: usize = 256;
const BATCH_TIMEOUT_MS: u64 = 50;
const SQLITE_FLUSH_SIZE: usize = 100;
const BATCH_MAX_CONTENT_BYTES: usize = 512 * 1024; // 512KB max content per batch
const MAX_FILE_SIZE_BYTES: u64 = 2 * 1024 * 1024; // 2MB max file size for indexing

/// Run the full indexing pipeline. Returns collected metadata for post-pipeline
/// phases (cross-stack linking, structural fingerprinting).
///
/// Uses Arc-based sharing — no ownership transfer, no loss on error.
pub async fn run_pipeline(
    files: Vec<PathBuf>,
    sqlite: SqliteStorage,
    lance: Arc<LanceStorage>,
    embedder: Arc<EmbedderPool>,
    progress: Option<Arc<SyncProgress>>,
    cancel: CancellationToken,
) -> Result<Vec<ParsedFileMetadata>> {
    let num_workers = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(4, 8))
        .unwrap_or(4);

    tracing::info!(
        "Pipeline: {} parser workers (optimized for CPU usage)",
        num_workers
    );

    // Scale up embedder pool for indexing (4 instances for parallelism)
    if let Err(e) = embedder.scale_up(4).await {
        tracing::warn!("Failed to scale up embedder pool: {}", e);
    };

    // Pre-fetch hashes for skip-unchanged logic
    let existing_hashes = sqlite.get_all_file_hashes().await?;

    // Shared collector for post-pipeline phases
    let collected: Arc<Mutex<Vec<ParsedFileMetadata>>> = Arc::new(Mutex::new(Vec::new()));

    // Bounded channels (backpressure)
    let (scan_tx, scan_rx) = mpsc::channel::<ScannedFile>(512);
    let (parse_tx, parse_rx) = mpsc::channel::<ParsedDoc>(256);
    let (batch_tx, batch_rx) = mpsc::channel::<ChunkBatch>(64);
    let (embed_tx, embed_rx) = mpsc::channel::<EmbeddedBatch>(64);

    // Shared receiver for parser workers
    let scan_rx = Arc::new(Mutex::new(scan_rx));

    // --- Spawn stages ---

    let prog_scanner = progress.clone();
    let cancel_scanner = cancel.clone();
    let h_scanner = tokio::spawn(async move {
        scanner_stage(
            files,
            existing_hashes,
            scan_tx,
            prog_scanner,
            cancel_scanner,
        )
        .await
    });

    // Parser workers — each gets a clone of the shared receiver
    let mut h_parsers: Vec<JoinHandle<anyhow::Result<()>>> = Vec::with_capacity(num_workers);
    for _ in 0..num_workers {
        let rx_clone = scan_rx.clone();
        let tx_clone = parse_tx.clone();
        let prog_clone = progress.clone();
        let cancel_clone = cancel.clone();
        h_parsers.push(tokio::spawn(async move {
            parser_worker(rx_clone, tx_clone, prog_clone, cancel_clone).await
        }));
    }
    drop(parse_tx); // Only worker clones hold senders now

    let cancel_batcher = cancel.clone();
    let h_batcher =
        tokio::spawn(async move { batcher_stage(parse_rx, batch_tx, cancel_batcher).await });

    let prog_embedder = progress.clone();
    let sqlite_embedder = sqlite.clone();
    let embedder_clone = embedder.clone();
    let cancel_embedder = cancel.clone();
    let h_embedder: JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
        if let Err(ref e) = embedder_stage(
            embedder_clone,
            batch_rx,
            embed_tx,
            prog_embedder,
            sqlite_embedder,
            cancel_embedder,
        )
        .await
        {
            tracing::error!("Embedder stage error: {}", e);
        }
        Ok(())
    });

    let sqlite_clone = sqlite.clone();
    let collected_clone = collected.clone();
    let prog_writer = progress.clone();
    let lance_compact = lance.clone();
    let cancel_writer = cancel.clone();
    let h_writer: JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
        if let Err(ref e) = writer_stage(
            sqlite_clone,
            lance,
            embed_rx,
            collected_clone,
            prog_writer,
            cancel_writer,
        )
        .await
        {
            tracing::error!("Writer stage error: {}", e);
        }
        Ok(())
    });

    // --- Await stages in order (each drains when upstream drops sender) ---

    let scan_count = h_scanner.await??;
    tracing::info!("Scanner: {} files sent to pipeline", scan_count);

    for (i, h) in h_parsers.into_iter().enumerate() {
        if let Err(e) = h.await? {
            tracing::error!("Parser worker {} error: {}", i, e);
        }
    }

    if let Err(e) = h_batcher.await? {
        tracing::error!("Batcher error: {}", e);
    }

    // Embedder — no ownership recovery needed (Arc-shared)
    h_embedder.await??;

    // Writer — no ownership recovery needed (Arc-shared)
    h_writer.await??;

    let metadata = Arc::try_unwrap(collected)
        .expect("all stage handles joined")
        .into_inner();

    tracing::info!("Pipeline complete: {} files processed", metadata.len());

    // Post-pipeline: compact LanceDB fragments to prevent read amplification
    if !metadata.is_empty() {
        if let Err(e) = lance_compact.compact().await {
            tracing::warn!("LanceDB compaction failed (non-fatal): {}", e);
        }
    }

    // Cache maintenance: evict old entries to prevent unbounded growth
    // Keep max 100k entries (~150MB) and entries younger than 30 days
    const CACHE_MAX_ENTRIES: i64 = 100_000;
    const CACHE_MAX_AGE_DAYS: i64 = 30;

    if let Err(e) = sqlite.evict_chunk_cache_by_age(CACHE_MAX_AGE_DAYS).await {
        tracing::debug!("Cache age eviction failed (non-fatal): {}", e);
    }
    if let Err(e) = sqlite.evict_chunk_cache_to_limit(CACHE_MAX_ENTRIES).await {
        tracing::debug!("Cache limit eviction failed (non-fatal): {}", e);
    }

    // Scale down embedder pool to save memory (1 instance for queries)
    if let Err(e) = embedder.scale_down(1).await {
        tracing::warn!("Failed to scale down embedder pool: {}", e);
    }

    Ok(metadata)
}

// ---------------------------------------------------------------------------
// Stage 1: Scanner — I/O bound file discovery
// ---------------------------------------------------------------------------

async fn scanner_stage(
    files: Vec<PathBuf>,
    existing_hashes: HashMap<String, (String, i64)>,
    tx: mpsc::Sender<ScannedFile>,
    progress: Option<Arc<SyncProgress>>,
    cancel: CancellationToken,
) -> anyhow::Result<usize> {
    let total = files.len();
    tracing::info!("Scanner: found {} files", total);

    if let Some(ref p) = progress {
        p.files_total.store(total, Ordering::Relaxed);
    }

    let mut sent = 0usize;
    let mut skipped_unchanged = 0usize;
    let mut skipped_unsupported = 0usize;
    let mut skipped_too_large = 0usize;
    let mut skipped_errors = 0usize;

    for path in files {
        if cancel.is_cancelled() {
            tracing::info!("Scanner: cancelled by user");
            break;
        }

        let path_str = path.to_string_lossy().to_string();

        // Detect language early to skip unsupported files
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let Some(language) = SupportedLanguage::from_extension(ext) else {
            skipped_unsupported += 1;
            continue;
        };

        // Modification time and file size check (cheap stat() call)
        let metadata = match tokio::fs::metadata(&path).await {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!("Scanner: cannot stat {:?}: {}", path, e);
                skipped_errors += 1;
                continue;
            }
        };

        // Skip files that are too large
        if metadata.len() > MAX_FILE_SIZE_BYTES {
            tracing::debug!(
                "Scanner: skipping {:?} ({}MB > {}MB limit)",
                path,
                metadata.len() / (1024 * 1024),
                MAX_FILE_SIZE_BYTES / (1024 * 1024)
            );
            skipped_too_large += 1;
            continue;
        }

        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        // Fast path: skip if mtime unchanged (avoids file read + hash)
        if let Some((_, stored_mtime)) = existing_hashes.get(&path_str) {
            if *stored_mtime == modified {
                skipped_unchanged += 1;
                // Update progress for skipped files too
                if let Some(ref p) = progress {
                    p.files_scanned.fetch_add(1, Ordering::Relaxed);
                }
                continue;
            }
        }

        // Read file (only when mtime changed)
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Scanner: cannot read {:?}: {}", path, e);
                skipped_errors += 1;
                continue;
            }
        };

        // Hash
        let hash = blake3::hash(content.as_bytes()).to_hex().to_string();

        // Skip unchanged (content hash confirms — handles rare mtime-only changes)
        if let Some((existing_hash, _)) = existing_hashes.get(&path_str) {
            if existing_hash == &hash {
                skipped_unchanged += 1;
                // Update progress for skipped files too
                if let Some(ref p) = progress {
                    p.files_scanned.fetch_add(1, Ordering::Relaxed);
                }
                continue;
            }
        }

        if tx
            .send(ScannedFile {
                path: path_str,
                content: Arc::new(content),
                hash,
                modified,
                language,
            })
            .await
            .is_err()
        {
            // Downstream closed — abort scanning
            tracing::warn!("Scanner: downstream closed, aborting");
            break;
        }

        sent += 1;
        if let Some(ref p) = progress {
            p.files_scanned.fetch_add(1, Ordering::Relaxed);
        }
        if sent.is_multiple_of(100) {
            tracing::info!("Scanner: {}/{} files queued", sent, total);
        }
    }

    tracing::info!(
        "Scanner: {} files sent to pipeline, {} unchanged, {} unsupported, {} too large, {} errors",
        sent,
        skipped_unchanged,
        skipped_unsupported,
        skipped_too_large,
        skipped_errors
    );

    Ok(sent)
}

// ---------------------------------------------------------------------------
// Stage 2: Parser workers — CPU-bound parsing via spawn_blocking
// ---------------------------------------------------------------------------

async fn parser_worker(
    rx: Arc<Mutex<mpsc::Receiver<ScannedFile>>>,
    tx: mpsc::Sender<ParsedDoc>,
    progress: Option<Arc<SyncProgress>>,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    loop {
        if cancel.is_cancelled() {
            tracing::info!("Parser: cancelled by user");
            break;
        }

        // Lock receiver, grab one message, release lock immediately
        let scanned = {
            let mut rx_guard = rx.lock().await;
            rx_guard.recv().await
        };

        let Some(scanned) = scanned else {
            break; // Channel closed, all files consumed
        };

        // Offload CPU-heavy parsing to blocking thread pool
        let parsed = match tokio::task::spawn_blocking(move || -> anyhow::Result<ParsedDoc> {
            let mut parser = CodeParser::new();
            let content_ref = &*scanned.content;

            // Single-pass parse: symbols + chunks + refs + imports from one tree
            let parsed_file = match parser.parse_file(content_ref, &scanned.path, scanned.language.clone())
            {
                Ok(pf) => pf,
                Err(e) => {
                    tracing::warn!("Parser: failed to parse {}: {}", scanned.path, e);
                    Default::default()
                }
            };

            let domain_config = DomainConfig::default_config();
            let (domain, tech_stack) = detect_domain(&scanned.path, content_ref, &domain_config);

            Ok(ParsedDoc {
                path: scanned.path,
                hash: scanned.hash,
                modified: scanned.modified,
                language: scanned.language,
                symbols: parsed_file.symbols,
                chunks: parsed_file.chunks,
                refs: parsed_file.refs,
                imports: parsed_file.imports,
                domain: SmolStr::from(domain.as_str()),
                tech_stack: tech_stack.into_iter().collect(),
                content: scanned.content,
            })
        })
        .await
        {
            Ok(Ok(doc)) => doc,
            Ok(Err(e)) => {
                tracing::warn!("Parser: file parse error, skipping: {}", e);
                continue;
            }
            Err(e) => {
                tracing::warn!("Parser: spawn_blocking panicked, skipping: {}", e);
                continue;
            }
        };

        if tx.send(parsed).await.is_err() {
            break; // Downstream closed
        }
        if let Some(ref p) = progress {
            p.files_parsed.fetch_add(1, Ordering::Relaxed);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Stage 3: Batcher — accumulate chunks with adaptive size/timeout flushing
// ---------------------------------------------------------------------------

/// Compute adaptive batch size based on average chunk content length.
/// Larger chunks → smaller batches to control memory usage.
fn adaptive_batch_size(chunks: &[CodeChunk]) -> usize {
    if chunks.is_empty() {
        return BATCH_CHUNK_SIZE_BASE;
    }

    let total_len: usize = chunks.iter().map(|c| c.content.len()).sum();
    let avg_len = total_len / chunks.len();

    // Heuristic: target ~100KB per batch for embedding
    // avg_len * batch_size ≈ 100KB → batch_size ≈ 100KB / avg_len
    let target_bytes = 100 * 1024;
    let computed = if avg_len > 0 {
        target_bytes / avg_len
    } else {
        BATCH_CHUNK_SIZE_BASE
    };

    computed.clamp(BATCH_CHUNK_SIZE_MIN, BATCH_CHUNK_SIZE_MAX)
}

/// Get total content size in bytes for current chunks.
fn total_content_bytes(chunks: &[CodeChunk]) -> usize {
    chunks.iter().map(|c| c.content.len()).sum()
}

async fn batcher_stage(
    mut rx: mpsc::Receiver<ParsedDoc>,
    tx: mpsc::Sender<ChunkBatch>,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let mut current_chunks: Vec<CodeChunk> = Vec::new();
    let mut current_metadata: Vec<ParsedFileMetadata> = Vec::new();
    let timeout = Duration::from_millis(BATCH_TIMEOUT_MS);
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut deadline_active = false;
    let mut docs_received = 0usize;
    let mut batches_sent = 0usize;

    tracing::info!("Batcher: started");

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("Batcher: cancelled by user");
                break;
            }
            msg = rx.recv() => {
                match msg {
                    Some(doc) => {
                        docs_received += 1;
                        let doc_chunks = doc.chunks.len();

                        // Split ParsedDoc into chunks + metadata
                        let had_chunks = !current_chunks.is_empty();
                        current_chunks.extend(doc.chunks);
                        current_metadata.push(ParsedFileMetadata {
                            path: doc.path.clone(),
                            hash: doc.hash,
                            modified: doc.modified,
                            language: doc.language,
                            symbols: doc.symbols,
                            refs: doc.refs,
                            imports: doc.imports,
                            domain: doc.domain,
                            tech_stack: doc.tech_stack,
                            content: doc.content,
                        });

                        tracing::debug!("Batcher: received doc {} '{}' with {} chunks (total accumulated: {} chunks, {} metadata)",
                            docs_received, doc.path, doc_chunks, current_chunks.len(), current_metadata.len());

                        // Start deadline on first chunk arrival
                        if !had_chunks && !current_chunks.is_empty() && !deadline_active {
                            deadline.as_mut().reset(Instant::now() + timeout);
                            deadline_active = true;
                            tracing::debug!("Batcher: deadline activated");
                        }

                        // Adaptive batch size based on chunk content
                        let batch_limit = adaptive_batch_size(&current_chunks);
                        let content_bytes = total_content_bytes(&current_chunks);

                        tracing::debug!("Batcher: batch_limit={}, content_bytes={}, current_chunks={}",
                            batch_limit, content_bytes, current_chunks.len());

                        // Flush if batch size reached OR content exceeds memory limit
                        if current_chunks.len() >= batch_limit || content_bytes >= BATCH_MAX_CONTENT_BYTES {
                            batches_sent += 1;
                            let chunks_count = current_chunks.len();
                            let metadata_count = current_metadata.len();

                            let batch = ChunkBatch {
                                chunks: std::mem::take(&mut current_chunks),
                                metadata: std::mem::take(&mut current_metadata),
                            };

                            tracing::info!("Batcher: sending batch {} with {} chunks, {} metadata",
                                batches_sent, chunks_count, metadata_count);

                            if tx.send(batch).await.is_err() {
                                tracing::error!("Batcher: downstream closed");
                                return Ok(()); // downstream closed
                            }
                            // Reset deadline for next batch
                            deadline_active = false;
                            tracing::debug!("Batcher: deadline deactivated");
                        }
                    }
                    None => {
                        tracing::info!("Batcher: parser channel closed, received {} docs total", docs_received);
                        // All parsers done — flush remaining
                        if !current_chunks.is_empty() || !current_metadata.is_empty() {
                            batches_sent += 1;
                            let chunks_count = current_chunks.len();
                            let metadata_count = current_metadata.len();

                            let batch = ChunkBatch {
                                chunks: std::mem::take(&mut current_chunks),
                                metadata: std::mem::take(&mut current_metadata),
                            };

                            tracing::info!("Batcher: sending final batch {} with {} chunks, {} metadata",
                                batches_sent, chunks_count, metadata_count);

                            let _ = tx.send(batch).await;
                        } else {
                            tracing::info!("Batcher: no remaining data to flush");
                        }
                        break;
                    }
                }
            }
            _ = &mut deadline, if deadline_active => {
                tracing::debug!("Batcher: deadline fired, chunks={}, metadata={}",
                    current_chunks.len(), current_metadata.len());

                // Timeout — flush partial batch if non-empty
                if !current_chunks.is_empty() || !current_metadata.is_empty() {
                    batches_sent += 1;
                    let chunks_count = current_chunks.len();
                    let metadata_count = current_metadata.len();

                    let batch = ChunkBatch {
                        chunks: std::mem::take(&mut current_chunks),
                        metadata: std::mem::take(&mut current_metadata),
                    };

                    tracing::info!("Batcher: sending timeout batch {} with {} chunks, {} metadata",
                        batches_sent, chunks_count, metadata_count);

                    if tx.send(batch).await.is_err() {
                        tracing::error!("Batcher: downstream closed on timeout");
                        return Ok(());
                    }
                }
                // Deactivate deadline until next batch starts
                deadline_active = false;
                tracing::debug!("Batcher: deadline deactivated after timeout");
            }
        }
    }

    tracing::info!(
        "Batcher: completed — {} docs received, {} batches sent",
        docs_received,
        batches_sent
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Stage 4: Embedder — batch embedding, single owner
// ---------------------------------------------------------------------------

async fn embedder_stage(
    embedder: Arc<EmbedderPool>,
    mut rx: mpsc::Receiver<ChunkBatch>,
    tx: mpsc::Sender<EmbeddedBatch>,
    progress: Option<Arc<SyncProgress>>,
    sqlite: SqliteStorage,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let mut total_embedded = 0usize;
    let mut cache_hits = 0usize;
    let mut batches_received = 0usize;

    tracing::info!("Embedder: started, waiting for batches");

    loop {
        let batch = tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("Embedder: cancelled by user");
                break;
            }
            res = rx.recv() => match res {
                Some(b) => b,
                None => break,
            }
        };

        batches_received += 1;
        tracing::info!(
            "Embedder: received batch {} with {} chunks, {} metadata",
            batches_received,
            batch.chunks.len(),
            batch.metadata.len()
        );

        if batch.chunks.is_empty() {
            // Metadata-only batch (files with no chunks) — pass through
            if !batch.metadata.is_empty() {
                tracing::debug!(
                    "Embedder: metadata-only batch, passing through {} metadata",
                    batch.metadata.len()
                );
                let embedded = EmbeddedBatch {
                    chunks: Vec::new(),
                    embeddings: Vec::new(),
                    metadata: batch.metadata,
                };
                if tx.send(embedded).await.is_err() {
                    tracing::error!("Embedder: downstream closed on metadata-only batch");
                    break;
                }
            }
            continue;
        }

        // Compute content hashes for dedup
        let hashes: Vec<String> = batch
            .chunks
            .iter()
            .map(|c| blake3::hash(c.content.as_bytes()).to_hex().to_string())
            .collect();

        tracing::debug!("Embedder: computed {} hashes", hashes.len());

        // Look up cached embeddings
        let cached = match sqlite.get_cached_embeddings(&hashes).await {
            Ok(c) => {
                tracing::debug!("Embedder: cache lookup found {} cached embeddings", c.len());
                c
            }
            Err(e) => {
                tracing::debug!("Embedding cache lookup failed (will re-embed): {}", e);
                std::collections::HashMap::new()
            }
        };

        // Split into cached vs. needs-embedding
        let mut final_embeddings: Vec<Option<Vec<f32>>> = vec![None; batch.chunks.len()];
        let mut to_embed_indices: Vec<usize> = Vec::new();
        let mut to_embed_texts: Vec<String> = Vec::new();

        for (i, hash) in hashes.iter().enumerate() {
            if let Some(emb) = cached.get(hash) {
                final_embeddings[i] = Some(emb.clone());
                cache_hits += 1;
            } else {
                to_embed_indices.push(i);
                to_embed_texts.push(batch.chunks[i].content.clone());
            }
        }

        tracing::debug!(
            "Embedder: {} chunks from cache, {} need embedding",
            batch.chunks.len() - to_embed_texts.len(),
            to_embed_texts.len()
        );

        // Embed only new chunks
        if !to_embed_texts.is_empty() {
            let count = to_embed_texts.len();
            tracing::info!("Embedder: calling embedder.embed() for {} texts", count);

            match embedder.embed(to_embed_texts).await {
                Ok(new_embeddings) => {
                    tracing::info!("Embedder: successfully embedded {} chunks", count);
                    total_embedded += count;
                    // Fill in results and prepare cache entries
                    let mut cache_entries: Vec<(String, Vec<f32>)> = Vec::with_capacity(count);
                    for (j, idx) in to_embed_indices.iter().enumerate() {
                        final_embeddings[*idx] = Some(new_embeddings[j].clone());
                        cache_entries.push((hashes[*idx].clone(), new_embeddings[j].clone()));
                    }
                    // Store in cache (fire-and-forget — non-critical)
                    if let Err(e) = sqlite.store_cached_embeddings(&cache_entries).await {
                        tracing::warn!("Embedder: cache store failed (non-fatal): {}", e);
                    } else {
                        tracing::debug!("Embedder: cached {} embeddings", cache_entries.len());
                    }
                }
                Err(e) => {
                    tracing::error!("Embedder: failed to embed {} chunks: {}", count, e);
                    // Pass metadata through
                    let embedded = EmbeddedBatch {
                        chunks: Vec::new(),
                        embeddings: Vec::new(),
                        metadata: batch.metadata,
                    };
                    let _ = tx.send(embedded).await;
                    continue;
                }
            }
        } else {
            tracing::debug!(
                "Embedder: all {} chunks from cache, no embedding needed",
                batch.chunks.len()
            );
        }

        if let Some(ref p) = progress {
            p.chunks_embedded
                .store(total_embedded + cache_hits, Ordering::Relaxed);
        }

        // Collect all embeddings (cached + fresh)
        let all_embeddings: Vec<Vec<f32>> = final_embeddings
            .into_iter()
            .map(|e| e.unwrap_or_default())
            .collect();

        tracing::info!(
            "Embedder: sending embedded batch with {} chunks, {} embeddings to writer",
            batch.chunks.len(),
            all_embeddings.len()
        );

        let embedded = EmbeddedBatch {
            chunks: batch.chunks,
            embeddings: all_embeddings,
            metadata: batch.metadata,
        };
        if tx.send(embedded).await.is_err() {
            tracing::error!("Embedder: downstream closed");
            break;
        }

        tracing::debug!("Embedder: batch sent successfully to writer");

        let total = total_embedded + cache_hits;
        if total.is_multiple_of(256) && total > 0 {
            tracing::info!(
                "Embedder: {} chunks processed ({} embedded, {} from cache)",
                total,
                total_embedded,
                cache_hits
            );
        }
    }

    tracing::info!(
        "Embedder: done — {} batches received, {} embedded, {} from cache",
        batches_received,
        total_embedded,
        cache_hits
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Stage 5: Writer — transaction-wrapped SQLite + LanceDB batch writes
// ---------------------------------------------------------------------------

async fn writer_stage(
    sqlite: SqliteStorage,
    lance: Arc<LanceStorage>,
    mut rx: mpsc::Receiver<EmbeddedBatch>,
    collected: Arc<Mutex<Vec<ParsedFileMetadata>>>,
    progress: Option<Arc<SyncProgress>>,
    cancel: tokio_util::sync::CancellationToken,
) -> anyhow::Result<()> {
    let mut pending_metadata: Vec<ParsedFileMetadata> = Vec::new();
    let mut total_files = 0usize;
    let mut total_chunks = 0usize;

    tracing::info!("Writer: started, waiting for embedded batches");

    loop {
        let batch = tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("Writer: cancelled by user");
                break;
            }
            res = rx.recv() => match res {
                Some(b) => b,
                None => break,
            }
        };

        tracing::debug!(
            "Writer: received batch with {} chunks, {} metadata",
            batch.chunks.len(),
            batch.metadata.len()
        );

        let mut lance_success = true;
        // Write chunks+embeddings to LanceDB immediately
        if !batch.chunks.is_empty() {
            tracing::debug!("Writer: writing {} chunks to LanceDB", batch.chunks.len());
            if let Err(e) = lance.upsert_chunks(&batch.chunks, &batch.embeddings).await {
                tracing::error!(
                    "Writer: LanceDB error: {}. Skipping SQLite metadata to avoid split-brain.",
                    e
                );
                lance_success = false;
            } else {
                tracing::debug!(
                    "Writer: successfully wrote {} chunks to LanceDB",
                    batch.chunks.len()
                );
                total_chunks += batch.chunks.len();
            }
        }

        if lance_success {
            // 0. Update progress eagerly!
            let metadata_count = batch.metadata.len();
            if metadata_count > 0 {
                total_files += metadata_count;
                if let Some(ref p) = progress {
                    p.files_written.store(total_files, Ordering::Relaxed);
                }
            }

            // Accumulate metadata for batched SQLite write
            pending_metadata.extend(batch.metadata);
        } else {
            tracing::warn!(
                "Writer: Skipped {} files due to LanceDB error",
                batch.metadata.len()
            );
        }

        // Flush to SQLite when batch size reached
        if pending_metadata.len() >= SQLITE_FLUSH_SIZE {
            let count = pending_metadata.len();
            tracing::info!("Writer: flushing {} metadata entries to SQLite", count);
            flush_sqlite_batch(&sqlite, &mut pending_metadata, &collected).await;
            tracing::info!("Writer: {} files written so far", total_files);
        }
    }

    tracing::info!("Writer: embedder channel closed, flushing remaining data");

    // Flush remaining
    if !pending_metadata.is_empty() {
        let count = pending_metadata.len();
        tracing::info!("Writer: final flush of {} metadata entries", count);
        flush_sqlite_batch(&sqlite, &mut pending_metadata, &collected).await;
    }

    // Resolve cross-file references
    match sqlite.resolve_references().await {
        Ok(resolved) => {
            if resolved > 0 {
                tracing::info!("Writer: resolved {} cross-file references", resolved);
            }
        }
        Err(e) => tracing::error!("Writer: resolve_references failed: {}", e),
    }

    tracing::info!(
        "Writer: {} files, {} chunks written",
        total_files,
        total_chunks
    );
    Ok(())
}

/// Flush a batch of file metadata to SQLite inside a single transaction.
async fn flush_sqlite_batch(
    sqlite: &SqliteStorage,
    pending: &mut Vec<ParsedFileMetadata>,
    collected: &Arc<Mutex<Vec<ParsedFileMetadata>>>,
) {
    let batch: Vec<ParsedFileMetadata> = std::mem::take(pending);

    // Separate metadata collection from SQLite write to avoid holding
    // the collection lock during I/O.
    let mut metadata_for_collection: Vec<(i64, ParsedFileMetadata)> =
        Vec::with_capacity(batch.len());

    // Use a transaction for all writes in this batch
    let pool = sqlite.pool();
    let tx_result = pool.begin().await;
    let mut tx = match tx_result {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Writer: failed to begin transaction: {}", e);
            return;
        }
    };

    for file_meta in batch {
        // 1. Upsert file record
        let file_id_result = sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO files (path, last_modified, content_hash)
            VALUES (?, ?, ?)
            ON CONFLICT(path) DO UPDATE SET
                last_modified = excluded.last_modified,
                content_hash = excluded.content_hash
            RETURNING id
            "#,
        )
        .bind(&file_meta.path)
        .bind(file_meta.modified)
        .bind(&file_meta.hash)
        .fetch_one(&mut *tx)
        .await;

        let file_id = match file_id_result {
            Ok(id) => id,
            Err(e) => {
                tracing::error!("Writer: upsert_file failed for {}: {}", file_meta.path, e);
                continue;
            }
        };

        // 2. Update domain, language, and indexing status
        let tech_json = serde_json::to_string(&file_meta.tech_stack).unwrap_or_default();
        let language_str = match file_meta.language.name() {
            "rust" => "rust",
            "typescript" => "typescript",
            "javascript" => "javascript",
            "python" => "python",
            "go" => "go",
            "vue" => "vue",
         _ => "unknown" };
        let now = chrono::Utc::now().timestamp();

        let _ = sqlx::query(
            "UPDATE files SET domain = ?, tech_stack = ?, language = ?, indexing_status = 'completed', last_indexed_at = ? WHERE id = ?"
        )
            .bind(file_meta.domain.as_str())
            .bind(&tech_json)
            .bind(language_str)
            .bind(now)
            .bind(file_id)
            .execute(&mut *tx)
            .await;

        // 3. Insert symbols (delete + batch insert)
        let _ = sqlx::query("DELETE FROM symbols WHERE file_id = ?")
            .bind(file_id)
            .execute(&mut *tx)
            .await;

        // Batch insert symbols (chunks of 100 to stay within SQLite bind limit)
        for chunk in file_meta.symbols.chunks(100) {
            let mut builder = sqlx::QueryBuilder::new(
                "INSERT INTO symbols (file_id, name, kind, line_start, line_end, signature) ",
            );
            builder.push_values(chunk, |mut b, s| {
                b.push_bind(file_id)
                    .push_bind(&s.name)
                    .push_bind(s.kind)
                    .push_bind(s.line_start)
                    .push_bind(s.line_end)
                    .push_bind(&s.signature);
            });
            let _ = builder.build().execute(&mut *tx).await;
        }

        // 4. Clear + record dependency usages
        let _ = sqlx::query("DELETE FROM dependency_usage WHERE file_id = ?")
            .bind(file_id)
            .execute(&mut *tx)
            .await;

        let ecosystem = match file_meta.language.name() {
            "rust" => "cargo",
            "typescript"
            | "javascript"
            | "vue" => "npm",
            "python" => "pip",
            "go" => "go",
            _ => "unknown"
        };

        for import in &file_meta.imports {
            if !import.is_relative {
                let pkg_name = extract_package_name(&import.path, file_meta.language.clone());
                let items_json = if !import.items.is_empty() {
                    Some(serde_json::to_string(&import.items).unwrap_or_default())
                } else {
                    None
                };
                let usage_type = match file_meta.language.name() {
                    "rust" => "use",
                    _ => "unknown"
                };

                // Find or create dependency
                let dep_id: Option<i64> = sqlx::query_scalar(
                    "SELECT id FROM dependencies WHERE name = ? AND ecosystem = ?",
                )
                .bind(&pkg_name)
                .bind(ecosystem)
                .fetch_optional(&mut *tx)
                .await
                .unwrap_or(None);

                let dep_id = match dep_id {
                    Some(id) => id,
                    None => {
                        let _ = sqlx::query(
                            "INSERT OR IGNORE INTO dependencies (name, version, ecosystem, updated_at) VALUES (?, '?', ?, ?)",
                        )
                        .bind(&pkg_name)
                        .bind(ecosystem)
                        .bind(chrono::Utc::now().timestamp())
                        .execute(&mut *tx)
                        .await;

                        sqlx::query_scalar(
                            "SELECT id FROM dependencies WHERE name = ? AND ecosystem = ?",
                        )
                        .bind(&pkg_name)
                        .bind(ecosystem)
                        .fetch_optional(&mut *tx)
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or(0)
                    }
                };

                if dep_id > 0 {
                    let _ = sqlx::query(
                        r#"
                        INSERT INTO dependency_usage (dependency_id, file_id, line, usage_type, import_path, items)
                        VALUES (?, ?, ?, ?, ?, ?)
                        ON CONFLICT(file_id, line, import_path) DO UPDATE SET
                            dependency_id = excluded.dependency_id,
                            usage_type = excluded.usage_type,
                            items = excluded.items
                        "#,
                    )
                    .bind(dep_id)
                    .bind(file_id)
                    .bind(import.line as i32)
                    .bind(usage_type)
                    .bind(&import.path)
                    .bind(items_json.as_deref())
                    .execute(&mut *tx)
                    .await;
                }
            }
        }

        // 5. Insert references per symbol
        // Get the symbols we just inserted to get their IDs
        let stored_symbols: Vec<Symbol> = sqlx::query_as::<_, Symbol>(
            "SELECT id, file_id, name, kind, line_start, line_end, signature FROM symbols WHERE file_id = ?",
        )
        .bind(file_id)
        .fetch_all(&mut *tx)
        .await
        .unwrap_or_default();

        for symbol in &stored_symbols {
            // Find refs that belong to this symbol's line range
            let symbol_refs: Vec<&SymbolReference> = file_meta
                .refs
                .iter()
                .filter(|r| {
                    r.line >= symbol.line_start
                        && r.line <= symbol.line_end
                        && r.target_name != symbol.name
                })
                .collect();

            if !symbol_refs.is_empty() {
                let _ = sqlx::query("DELETE FROM symbol_references WHERE source_symbol_id = ?")
                    .bind(symbol.id)
                    .execute(&mut *tx)
                    .await;

                // Batch insert references (chunks of 100)
                for chunk in symbol_refs.chunks(100) {
                    let mut builder = sqlx::QueryBuilder::new(
                        "INSERT INTO symbol_references (source_symbol_id, target_name, target_symbol_id, kind, line) ",
                    );
                    builder.push_values(chunk.iter(), |mut b, r| {
                        b.push_bind(symbol.id)
                            .push_bind(&r.target_name)
                            .push_bind(r.target_symbol_id)
                            .push_bind(&r.kind)
                            .push_bind(r.line);
                    });
                    let _ = builder.build().execute(&mut *tx).await;
                }
            }
        }

        // Collect metadata for cross-stack linking (clone the parts we need)
        metadata_for_collection.push((
            file_id,
            ParsedFileMetadata {
                path: file_meta.path,
                hash: file_meta.hash,
                modified: file_meta.modified,
                language: file_meta.language,
                symbols: file_meta.symbols,
                refs: file_meta.refs,
                imports: file_meta.imports,
                domain: file_meta.domain,
                tech_stack: file_meta.tech_stack,
                content: file_meta.content,
            },
        ));
    }

    // Commit transaction
    if let Err(e) = tx.commit().await {
        tracing::error!("Writer: transaction commit failed: {}", e);
        return;
    }

    // Push to shared collection (outside transaction scope)
    let mut coll = collected.lock().await;
    coll.extend(metadata_for_collection.into_iter().map(|(_, m)| m));
}

// ---------------------------------------------------------------------------
// Helper: extract package name (shared with service.rs)
// ---------------------------------------------------------------------------

pub(crate) fn extract_package_name(import_path: &str, language: SupportedLanguage) -> String {
    match language.name() {
        "rust" => import_path
            .split("::")
            .next()
            .unwrap_or(import_path)
            .to_string(),
        "typescript" | "javascript" | "vue" => {
            if import_path.starts_with('@') {
                let parts: Vec<&str> = import_path.split('/').collect();
                if parts.len() >= 2 {
                    format!("{}/{}", parts[0], parts[1])
                } else {
                    import_path.to_string()
                }
            } else {
                import_path
                    .split('/')
                    .next()
                    .unwrap_or(import_path)
                    .to_string()
            }
        }
        "python" => import_path
            .split('.')
            .next()
            .unwrap_or(import_path)
            .to_string(),
        "go" => {
            // Go import paths like "github.com/user/repo/pkg" — use last segment
            import_path
                .rsplit('/')
                .next()
                .unwrap_or(import_path)
                .to_string()
        }
        _ => import_path.to_string(),
    }
}
