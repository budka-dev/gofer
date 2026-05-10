use git2::{BlameOptions, Repository};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlameInfo {
    /// First line in the file covered by this hunk (1-based).
    pub line: u32,
    /// Number of consecutive lines this hunk spans. Callers asking about a
    /// range need this to know how far each commit's authorship extends.
    #[serde(default = "default_lines_in_hunk")]
    pub lines_in_hunk: u32,
    pub commit_id: String,
    pub author: String,
    pub message: String,
    pub timestamp: i64,
}

fn default_lines_in_hunk() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    pub id: String,
    pub author: String,
    pub email: String,
    pub message: String,
    pub timestamp: i64,
}

/// LRU-style cache entry for git results.
struct CacheEntry<T: Clone> {
    value: T,
    created: std::time::Instant,
}

const CACHE_TTL_SECS: u64 = 60;
const CACHE_MAX_ENTRIES: usize = 128;

/// Git repository wrapper with LRU cache for blame/history results.
pub struct GitRepo {
    repo: Repository,
    blame_cache: std::sync::Mutex<HashMap<String, CacheEntry<Vec<BlameInfo>>>>,
    history_cache: std::sync::Mutex<HashMap<String, CacheEntry<Vec<CommitInfo>>>>,
}

impl GitRepo {
    /// Open a git repository at the given path
    pub fn open(path: &Path) -> Option<Self> {
        Repository::discover(path).ok().map(|repo| Self {
            repo,
            blame_cache: std::sync::Mutex::new(HashMap::new()),
            history_cache: std::sync::Mutex::new(HashMap::new()),
        })
    }

    /// Get blame info for a specific line range in a file (cached).
    pub fn blame_lines(&self, file_path: &Path, start_line: u32, end_line: u32) -> Vec<BlameInfo> {
        let key = format!("{}:{}:{}", file_path.display(), start_line, end_line);

        // Check cache
        if let Ok(cache) = self.blame_cache.lock() {
            if let Some(entry) = cache.get(&key) {
                if entry.created.elapsed().as_secs() < CACHE_TTL_SECS {
                    return entry.value.clone();
                }
            }
        }

        let results = self.blame_lines_uncached(file_path, start_line, end_line);

        // Store in cache
        if let Ok(mut cache) = self.blame_cache.lock() {
            // Evict oldest entries if cache is full
            if cache.len() >= CACHE_MAX_ENTRIES {
                let oldest = cache
                    .iter()
                    .min_by_key(|(_, v)| v.created)
                    .map(|(k, _)| k.clone());
                if let Some(k) = oldest {
                    cache.remove(&k);
                }
            }
            cache.insert(
                key,
                CacheEntry {
                    value: results.clone(),
                    created: std::time::Instant::now(),
                },
            );
        }

        results
    }

    fn blame_lines_uncached(
        &self,
        file_path: &Path,
        start_line: u32,
        end_line: u32,
    ) -> Vec<BlameInfo> {
        let mut results = Vec::new();

        // Make path relative to repo root
        let workdir = match self.repo.workdir() {
            Some(wd) => wd,
            None => return results,
        };

        let relative_path = match file_path.strip_prefix(workdir) {
            Ok(p) => p,
            Err(_) => file_path,
        };

        let mut opts = BlameOptions::new();
        opts.min_line(start_line as usize);
        opts.max_line(end_line as usize);

        let blame = match self.repo.blame_file(relative_path, Some(&mut opts)) {
            Ok(b) => b,
            Err(_) => return results,
        };

        for hunk in blame.iter() {
            let commit_id = hunk.final_commit_id().to_string();
            let sig = hunk.final_signature();

            let author = sig.name().unwrap_or("Unknown").to_string();
            let timestamp = sig.when().seconds();

            // Get commit message
            let message = self
                .repo
                .find_commit(hunk.final_commit_id())
                .ok()
                .and_then(|c| {
                    c.message()
                        .map(|m| m.lines().next().unwrap_or("").to_string())
                })
                .unwrap_or_default();

            let line = hunk.final_start_line() as u32;
            let lines_in_hunk = hunk.lines_in_hunk() as u32;

            results.push(BlameInfo {
                line,
                lines_in_hunk: lines_in_hunk.max(1),
                commit_id,
                author,
                message,
                timestamp,
            });
        }

        results
    }

    /// Get recent commits that modified a file (cached).
    pub fn file_history(&self, file_path: &Path, limit: usize) -> Vec<CommitInfo> {
        let key = format!("{}:{}", file_path.display(), limit);

        // Check cache
        if let Ok(cache) = self.history_cache.lock() {
            if let Some(entry) = cache.get(&key) {
                if entry.created.elapsed().as_secs() < CACHE_TTL_SECS {
                    return entry.value.clone();
                }
            }
        }

        let results = self.file_history_uncached(file_path, limit);

        // Store in cache
        if let Ok(mut cache) = self.history_cache.lock() {
            if cache.len() >= CACHE_MAX_ENTRIES {
                let oldest = cache
                    .iter()
                    .min_by_key(|(_, v)| v.created)
                    .map(|(k, _)| k.clone());
                if let Some(k) = oldest {
                    cache.remove(&k);
                }
            }
            cache.insert(
                key,
                CacheEntry {
                    value: results.clone(),
                    created: std::time::Instant::now(),
                },
            );
        }

        results
    }

    fn file_history_uncached(&self, file_path: &Path, limit: usize) -> Vec<CommitInfo> {
        let mut results = Vec::new();

        let workdir = match self.repo.workdir() {
            Some(wd) => wd,
            None => return results,
        };

        let relative_path = match file_path.strip_prefix(workdir) {
            Ok(p) => p,
            Err(_) => file_path,
        };

        let mut revwalk = match self.repo.revwalk() {
            Ok(rw) => rw,
            Err(_) => return results,
        };

        if revwalk.push_head().is_err() {
            return results;
        }

        let path_str = relative_path.to_string_lossy();

        for oid in revwalk.flatten().take(limit * 10) {
            let commit = match self.repo.find_commit(oid) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Check if this commit modified the file
            if let Some(parent) = commit.parents().next() {
                let tree = match commit.tree() {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                let parent_tree = match parent.tree() {
                    Ok(t) => t,
                    Err(_) => continue,
                };

                let diff = match self
                    .repo
                    .diff_tree_to_tree(Some(&parent_tree), Some(&tree), None)
                {
                    Ok(d) => d,
                    Err(_) => continue,
                };

                let mut modified = false;
                diff.foreach(
                    &mut |delta, _| {
                        if let Some(p) = delta.new_file().path() {
                            if p.to_string_lossy() == path_str {
                                modified = true;
                            }
                        }
                        true
                    },
                    None,
                    None,
                    None,
                )
                .ok();

                if !modified {
                    continue;
                }
            } else {
                // Initial commit (no parent) — проверяем, есть ли файл в дереве
                let tree = match commit.tree() {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if tree.get_path(relative_path).is_err() {
                    continue;
                }
            }

            let sig = commit.author();
            results.push(CommitInfo {
                id: oid.to_string(),
                author: sig.name().unwrap_or("Unknown").to_string(),
                email: sig.email().unwrap_or("").to_string(),
                message: commit
                    .message()
                    .map(|m| m.lines().next().unwrap_or("").to_string())
                    .unwrap_or_default(),
                timestamp: sig.when().seconds(),
            });

            if results.len() >= limit {
                break;
            }
        }

        results
    }

    /// Get the commit that last modified a specific line
    #[allow(dead_code)]
    pub fn line_history(&self, file_path: &Path, line: u32) -> Option<BlameInfo> {
        self.blame_lines(file_path, line, line).into_iter().next()
    }

    /// Get diff output as patch text
    pub fn git_diff(&self, file_path: Option<&Path>, staged: bool) -> Option<String> {
        let mut opts = git2::DiffOptions::new();
        if let Some(path) = file_path {
            let workdir = self.repo.workdir()?;
            let rel = path.strip_prefix(workdir).unwrap_or(path);
            opts.pathspec(rel);
        }

        let diff = if staged {
            let head_tree = self.repo.head().ok()?.peel_to_tree().ok()?;
            self.repo
                .diff_tree_to_index(Some(&head_tree), None, Some(&mut opts))
                .ok()?
        } else {
            self.repo
                .diff_index_to_workdir(None, Some(&mut opts))
                .ok()?
        };

        let mut text = String::new();
        diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
            let origin = line.origin();
            let content = String::from_utf8_lossy(line.content());
            match origin {
                '+' | '-' | ' ' => {
                    text.push(origin);
                    text.push_str(&content);
                }
                _ => text.push_str(&content),
            }
            true
        })
        .ok()?;

        Some(text)
    }
}
