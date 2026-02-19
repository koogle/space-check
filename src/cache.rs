use crate::scanner::{CruftEntry, LargeFileEntry, TopFolderEntry};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CacheKey {
    pub scan_root: PathBuf,
    pub threshold_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedScanResult {
    pub key: CacheKey,
    pub created_at: SystemTime,
    pub top_folders: Vec<TopFolderEntry>,
    pub cruft_items: Vec<CruftEntry>,
    pub large_file_items: Vec<LargeFileEntry>,
}

#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub cache_dir: PathBuf,
    pub ttl: Duration,
}

impl CacheKey {
    fn to_filename(&self) -> String {
        let mut hasher = DefaultHasher::new();
        self.scan_root.hash(&mut hasher);
        self.threshold_bytes.hash(&mut hasher);
        format!("scan_{:016x}.json", hasher.finish())
    }
}

impl CacheConfig {
    pub fn default_config() -> Option<Self> {
        let cache_dir = dirs::cache_dir()?.join("space-check");
        Some(CacheConfig {
            cache_dir,
            ttl: Duration::from_secs(300),
        })
    }
}

/// Try to load a valid (non-expired) cache entry for the given key.
pub fn load(config: &CacheConfig, key: &CacheKey) -> Option<CachedScanResult> {
    let path = config.cache_dir.join(key.to_filename());
    let data = std::fs::read_to_string(&path).ok()?;
    let result: CachedScanResult = serde_json::from_str(&data).ok()?;

    // TTL check
    let elapsed = result.created_at.elapsed().ok()?;
    if elapsed > config.ttl {
        let _ = std::fs::remove_file(&path);
        return None;
    }

    // Verify key matches (defense against hash collisions)
    if result.key != *key {
        return None;
    }

    Some(result)
}

/// Save scan results to the cache.
pub fn save(config: &CacheConfig, result: &CachedScanResult) {
    let _ = std::fs::create_dir_all(&config.cache_dir);
    let path = config.cache_dir.join(result.key.to_filename());
    if let Ok(json) = serde_json::to_string(result) {
        let _ = std::fs::write(&path, json);
    }
}

/// Invalidate cache entries whose scan root is an ancestor of any deleted path.
pub fn invalidate_containing(config: &CacheConfig, deleted_paths: &[PathBuf]) {
    let entries = match std::fs::read_dir(&config.cache_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let result: CachedScanResult = match serde_json::from_str(&data) {
            Ok(r) => r,
            Err(_) => {
                let _ = std::fs::remove_file(&path);
                continue;
            }
        };

        let should_invalidate = deleted_paths
            .iter()
            .any(|deleted| deleted.starts_with(&result.key.scan_root));

        if should_invalidate {
            let _ = std::fs::remove_file(&path);
        }
    }
}
