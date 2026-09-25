//! Remote actor avatar disk cache (CACHE-04). Aligned to Swift `AvatarCache`.

use std::fs;
use std::path::{Path, PathBuf};

use md5::{Digest, Md5};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AvatarError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid remote url")]
    InvalidUrl,
}

pub struct AvatarCache {
    dir: PathBuf,
}

impl AvatarCache {
    pub fn open_default() -> Result<Self, AvatarError> {
        let caches = dirs::cache_dir().ok_or_else(|| {
            AvatarError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no cache directory",
            ))
        })?;
        Self::open(caches.join("sula").join("avatars"))
    }

    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, AvatarError> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    pub fn cache_dir(&self) -> &Path {
        &self.dir
    }

    pub fn cached_path(&self, remote_url: &str) -> Option<PathBuf> {
        let path = self.file_path(remote_url).ok()?;
        path.is_file().then_some(path)
    }

    pub fn store(&self, remote_url: &str, data: &[u8]) -> Result<PathBuf, AvatarError> {
        if data.is_empty() {
            return Err(AvatarError::InvalidUrl);
        }
        fs::create_dir_all(&self.dir)?;
        let path = self.file_path(remote_url)?;
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, data)?;
        fs::rename(&tmp, &path)?;
        Ok(path)
    }

    pub fn clear(&self) -> Result<usize, AvatarError> {
        let mut n = 0usize;
        if !self.dir.is_dir() {
            fs::create_dir_all(&self.dir)?;
            return Ok(0);
        }
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                fs::remove_file(&path)?;
                n += 1;
            }
        }
        Ok(n)
    }

    fn file_path(&self, remote_url: &str) -> Result<PathBuf, AvatarError> {
        let name = cache_file_name(remote_url)?;
        Ok(self.dir.join(name))
    }
}

fn cache_file_name(remote_url: &str) -> Result<String, AvatarError> {
    let normalized = normalize_remote_url(remote_url)?;
    let mut hasher = Md5::new();
    hasher.update(normalized.as_bytes());
    let hex = format!("{:x}", hasher.finalize());
    let path_part = remote_url
        .split('?')
        .next()
        .unwrap_or(remote_url)
        .rsplit('/')
        .next()
        .unwrap_or("");
    let ext = path_part
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .filter(|e| matches!(e.as_str(), "jpg" | "jpeg" | "png" | "webp" | "gif"))
        .unwrap_or_else(|| "jpg".into());
    Ok(format!("{hex}.{ext}"))
}

fn normalize_remote_url(remote_url: &str) -> Result<String, AvatarError> {
    let trimmed = remote_url.trim();
    if trimmed.is_empty() {
        return Err(AvatarError::InvalidUrl);
    }
    // Light normalize: lowercase scheme/host-ish prefix without full URL parser.
    let lower = trimmed.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(AvatarError::InvalidUrl);
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn store_and_reuse() {
        let dir = tempdir().unwrap();
        let cache = AvatarCache::open(dir.path()).unwrap();
        let url = "https://image.tmdb.org/t/p/w185/abc.jpg";
        assert!(cache.cached_path(url).is_none());
        let path = cache.store(url, b"fake-jpeg").unwrap();
        assert!(path.is_file());
        assert_eq!(cache.cached_path(url).unwrap(), path);
        let n = cache.clear().unwrap();
        assert!(n >= 1);
        assert!(cache.cached_path(url).is_none());
    }

    #[test]
    fn rejects_empty_url() {
        assert!(normalize_remote_url("").is_err());
        assert!(normalize_remote_url("not-a-url").is_err());
    }
}
