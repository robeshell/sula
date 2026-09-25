//! Port of Swift `ThumbnailCache` disk + memory tiers.
//! CACHE-01 disk JPEG; CACHE-02 in-process path LRU; CACHE-03 warm/lazy elsewhere.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use image::ImageReader;
use md5::{Digest, Md5};
use thiserror::Error;

/// Shared poster thumb size for list grid + detail panel (cache key must match).
pub const POSTER_THUMB_WIDTH: u32 = 140;
pub const POSTER_THUMB_HEIGHT: u32 = 210;
/// Season strip thumbs — same 2:3 poster ratio as cover.
pub const SEASON_THUMB_WIDTH: u32 = 72;
pub const SEASON_THUMB_HEIGHT: u32 = 108;
/// Episode still thumbs — landscape 16:9.
pub const EPISODE_STILL_WIDTH: u32 = 160;
pub const EPISODE_STILL_HEIGHT: u32 = 90;

/// Skip decode when source JPEG is already small enough for UI use.
const SMALL_JPEG_COPY_MAX_BYTES: u64 = 96 * 1024;
const MEMORY_CAPACITY: usize = 256;

#[derive(Debug, Error)]
pub enum ThumbnailError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("image error: {0}")]
    Image(String),
    #[error("source missing: {0}")]
    Missing(PathBuf),
}

pub struct ThumbnailCache {
    dir: PathBuf,
    memory: Mutex<MemoryLru>,
}

struct MemoryLru {
    map: HashMap<String, PathBuf>,
    order: VecDeque<String>,
    capacity: usize,
}

impl MemoryLru {
    fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    fn get(&mut self, key: &str) -> Option<PathBuf> {
        let path = self.map.get(key)?.clone();
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
            self.order.push_back(key.to_string());
        }
        Some(path)
    }

    fn put(&mut self, key: String, path: PathBuf) {
        if self.map.contains_key(&key) {
            if let Some(pos) = self.order.iter().position(|k| k == &key) {
                self.order.remove(pos);
            }
        } else if self.map.len() >= self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
        self.order.push_back(key.clone());
        self.map.insert(key, path);
    }

    fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }
}

impl ThumbnailCache {
    pub fn open_default() -> Result<Self, ThumbnailError> {
        let caches = dirs::cache_dir().ok_or_else(|| {
            ThumbnailError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no cache directory",
            ))
        })?;
        Self::open(caches.join("sula").join("thumbnails"))
    }

    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, ThumbnailError> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            memory: Mutex::new(MemoryLru::new(MEMORY_CAPACITY)),
        })
    }

    pub fn cache_dir(&self) -> &Path {
        &self.dir
    }

    /// Pure string join — mirrors Swift `(folderPath as NSString).appendingPathComponent(posterPath)`.
    /// Absolute `poster_path` is kept as-is (already resolved on disk).
    pub fn join_poster_path(folder_path: &str, poster_path: &str) -> PathBuf {
        let poster = Path::new(poster_path);
        if poster.is_absolute() {
            poster.to_path_buf()
        } else {
            Path::new(folder_path).join(poster)
        }
    }

    /// Resolve a readable poster file: preferred path, then common artwork names.
    pub fn resolve_poster_source(folder_path: &str, poster_path: &str) -> Option<PathBuf> {
        Self::resolve_poster_source_with_fallbacks(folder_path, poster_path, true)
    }

    /// When `allow_fallbacks` is false, only the exact joined path is accepted
    /// (used for season posters so missing season art does not become show poster.jpg).
    pub fn resolve_poster_source_with_fallbacks(
        folder_path: &str,
        poster_path: &str,
        allow_fallbacks: bool,
    ) -> Option<PathBuf> {
        let primary = Self::join_poster_path(folder_path, poster_path);
        if primary.is_file() {
            return Some(primary);
        }
        if !allow_fallbacks {
            return None;
        }
        const FALLBACKS: &[&str] = &[
            "poster.jpg",
            "poster.png",
            "poster.webp",
            "cover.jpg",
            "folder.jpg",
        ];
        let folder = Path::new(folder_path);
        for name in FALLBACKS {
            let candidate = folder.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }

    /// Cache key without mtime: `path_WxH` (Swift `ThumbnailCache.key`).
    pub fn key(path: &str, width: u32, height: u32) -> String {
        format!("{path}_{width}x{height}")
    }

    /// Ensure a disk JPEG exists for `source` at target size; return cache file path.
    /// Disk key includes mtime like Swift (`key_mtime`) for poster replacement invalidation.
    pub fn ensure(
        &self,
        source: &Path,
        width: u32,
        height: u32,
    ) -> Result<PathBuf, ThumbnailError> {
        if !source.is_file() {
            return Err(ThumbnailError::Missing(source.to_path_buf()));
        }
        let path_str = source.to_string_lossy();
        let base_key = Self::key(&path_str, width, height);
        let mtime = mtime_secs(source);
        let disk_key = format!("{base_key}_{mtime}");

        if let Ok(mut mem) = self.memory.lock() {
            if let Some(cached) = mem.get(&disk_key) {
                if cached.is_file() {
                    return Ok(cached);
                }
            }
        }

        let cache_path = self.file_url_for(&disk_key);
        if cache_path.is_file() {
            self.remember(&disk_key, cache_path.clone());
            return Ok(cache_path);
        }

        let tmp = cache_path.with_extension("jpg.tmp");
        let max_dim = width.max(height).saturating_mul(2).max(1);

        // Fast path: already-small JPEG — one NAS read via copy, no CPU decode.
        if looks_like_jpeg(source) {
            let len = fs::metadata(source)?.len();
            if len > 0 && len <= SMALL_JPEG_COPY_MAX_BYTES {
                fs::copy(source, &tmp)?;
                fs::rename(&tmp, &cache_path)?;
                self.remember(&disk_key, cache_path.clone());
                return Ok(cache_path);
            }
            // Already at/under display size: copy instead of re-encode.
            if let Ok((w, h)) = jpeg_dimensions(source) {
                if w.max(h) <= max_dim {
                    fs::copy(source, &tmp)?;
                    fs::rename(&tmp, &cache_path)?;
                    self.remember(&disk_key, cache_path.clone());
                    return Ok(cache_path);
                }
            }
        }

        let jpeg = downsample_to_jpeg(source, max_dim)?;
        fs::write(&tmp, &jpeg)?;
        fs::rename(&tmp, &cache_path)?;
        self.remember(&disk_key, cache_path.clone());
        Ok(cache_path)
    }

    /// Warm standard poster thumb into disk cache (list + detail share this size).
    pub fn ensure_poster_thumb(&self, source: &Path) -> Result<PathBuf, ThumbnailError> {
        self.ensure(source, POSTER_THUMB_WIDTH, POSTER_THUMB_HEIGHT)
    }

    /// Clear memory + disk thumbnail cache (settings action; SET-08 adjacent).
    pub fn clear_all(&self) -> Result<usize, ThumbnailError> {
        if let Ok(mut mem) = self.memory.lock() {
            mem.clear();
        }
        let mut removed = 0usize;
        if self.dir.is_dir() {
            for entry in fs::read_dir(&self.dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() {
                    fs::remove_file(&path)?;
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    fn remember(&self, key: &str, path: PathBuf) {
        if let Ok(mut mem) = self.memory.lock() {
            mem.put(key.to_string(), path);
        }
    }

    fn file_url_for(&self, key: &str) -> PathBuf {
        let mut hasher = Md5::new();
        hasher.update(key.as_bytes());
        let hex = format!("{:x}", hasher.finalize());
        self.dir.join(format!("{hex}.jpg"))
    }
}

fn mtime_secs(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn looks_like_jpeg(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let e = e.to_ascii_lowercase();
            e == "jpg" || e == "jpeg"
        })
        .unwrap_or(false)
}

fn jpeg_dimensions(path: &Path) -> Result<(u32, u32), ThumbnailError> {
    ImageReader::open(path)
        .map_err(|e| ThumbnailError::Image(e.to_string()))?
        .with_guessed_format()
        .map_err(|e| ThumbnailError::Image(e.to_string()))?
        .into_dimensions()
        .map_err(|e| ThumbnailError::Image(e.to_string()))
}

fn downsample_to_jpeg(path: &Path, max_dim: u32) -> Result<Vec<u8>, ThumbnailError> {
    let reader = ImageReader::open(path)
        .map_err(|e| ThumbnailError::Image(e.to_string()))?
        .with_guessed_format()
        .map_err(|e| ThumbnailError::Image(e.to_string()))?;
    let img = reader
        .decode()
        .map_err(|e| ThumbnailError::Image(e.to_string()))?;
    // `thumbnail` is faster than high-quality `resize` for preview-sized outputs.
    let resized = img.thumbnail(max_dim, max_dim);
    let mut out = Cursor::new(Vec::new());
    resized
        .to_rgb8()
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .map_err(|e| ThumbnailError::Image(e.to_string()))?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};
    use tempfile::tempdir;

    #[test]
    fn ensure_writes_and_reuses_disk_cache() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("poster.png");
        let img = RgbImage::from_pixel(200, 300, Rgb([10, 20, 30]));
        img.save(&src).unwrap();

        let cache = ThumbnailCache::open(dir.path().join("thumbs")).unwrap();
        let first = cache.ensure(&src, 32, 46).unwrap();
        assert!(first.is_file());
        let second = cache.ensure(&src, 32, 46).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn clear_all_removes_disk_files() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("poster.png");
        let img = RgbImage::from_pixel(80, 120, Rgb([1, 2, 3]));
        img.save(&src).unwrap();
        let cache = ThumbnailCache::open(dir.path().join("thumbs")).unwrap();
        let path = cache.ensure(&src, 32, 46).unwrap();
        assert!(path.is_file());
        let n = cache.clear_all().unwrap();
        assert!(n >= 1);
        assert!(!path.is_file());
    }

    #[test]
    fn small_jpeg_copied_without_resize() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("small.jpg");
        let img = RgbImage::from_pixel(40, 60, Rgb([9, 8, 7]));
        img.save(&src).unwrap();
        let cache = ThumbnailCache::open(dir.path().join("thumbs")).unwrap();
        let out = cache.ensure(&src, 140, 210).unwrap();
        assert!(out.is_file());
    }
}
