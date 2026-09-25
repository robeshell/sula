//! Poster thumbnails and actor avatars, served from local caches.

use std::path::PathBuf;
use std::sync::Arc;

use crate::app::{blocking, err_string};
use crate::state::AppState;

const MAX_AVATAR_BYTES: usize = 10 * 1024 * 1024;

impl AppState {
    /// A cached thumbnail for a poster inside a library, generating it if needed.
    /// Returns `None` when there is no usable image or it lies outside every library.
    pub async fn poster_thumbnail(
        &self,
        folder_path: String,
        poster_path: String,
        width: Option<u32>,
        height: Option<u32>,
        allow_fallbacks: Option<bool>,
    ) -> Result<Option<PathBuf>, String> {
        let width = width.unwrap_or(media_core::POSTER_THUMB_WIDTH);
        let height = height.unwrap_or(media_core::POSTER_THUMB_HEIGHT);
        let allow_fallbacks = allow_fallbacks.unwrap_or(true);
        let (db, thumbs) = (Arc::clone(&self.db), Arc::clone(&self.thumbs));
        blocking(move || {
            let Some(source) = media_core::ThumbnailCache::resolve_poster_source_with_fallbacks(
                &folder_path,
                &poster_path,
                allow_fallbacks,
            ) else {
                return Ok(None);
            };
            // Thumbnails land in a UI-readable cache: only images inside a library.
            let canonical = media_core::scanner::canonicalize_lossy(std::path::Path::new(&source));
            let inside_library = db.list_libraries().map_err(err_string)?.iter().any(|library| {
                let root = media_core::scanner::canonicalize_lossy(std::path::Path::new(&library.root_path));
                media_core::db::path_rooted_under(&canonical, &root)
            });
            if !inside_library {
                return Ok(None);
            }
            match thumbs.ensure(&source, width, height) {
                Ok(path) => Ok(Some(path)),
                Err(media_core::ThumbnailError::Missing(_)) => Ok(None),
                Err(err) => Err(err.to_string()),
            }
        })
        .await
    }

    /// A cached copy of an actor photo, downloading it on first use.
    pub async fn actor_avatar(&self, url: String) -> Result<Option<PathBuf>, String> {
        let url = url.trim().to_string();
        if url.is_empty() {
            return Ok(None);
        }
        if let Some(cached) = self.avatars.cached_path(&url) {
            return Ok(Some(cached));
        }
        // The URL comes from scraped metadata: only fetch public http(s) hosts, also
        // after redirects, and cap the body so a hostile source can't fill the disk.
        let parsed = reqwest::Url::parse(&url).map_err(err_string)?;
        if !is_public_http_url(&parsed) {
            return Ok(None);
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 || !is_public_http_url(attempt.url()) {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(err_string)?;
        let mut response = client.get(parsed).send().await.map_err(|e| e.without_url().to_string())?;
        if !response.status().is_success() {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url().to_string())? {
            if bytes.len() + chunk.len() > MAX_AVATAR_BYTES {
                return Ok(None);
            }
            bytes.extend_from_slice(&chunk);
        }
        let avatars = Arc::clone(&self.avatars);
        blocking(move || avatars.store(&url, &bytes).map_err(err_string)).await.map(Some)
    }

    /// Empties the poster thumbnail and avatar caches; returns the files removed.
    pub async fn clear_image_caches(&self) -> Result<usize, String> {
        let (thumbs, avatars) = (Arc::clone(&self.thumbs), Arc::clone(&self.avatars));
        blocking(move || {
            let thumbs = thumbs.clear_all().map_err(err_string)?;
            let avatars = avatars.clear().map_err(err_string)?;
            Ok(thumbs + avatars)
        })
        .await
    }
}

fn is_public_http_url(url: &reqwest::Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else { return false };
    let host = host.trim_start_matches('[').trim_end_matches(']').trim_end_matches('.').to_ascii_lowercase();
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            !(ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast())
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            let unique_local = (ip.segments()[0] & 0xfe00) == 0xfc00;
            let link_local = (ip.segments()[0] & 0xffc0) == 0xfe80;
            !(ip.is_loopback() || ip.is_unspecified() || unique_local || link_local)
                && ip.to_ipv4_mapped().is_none_or(|v4| !(v4.is_loopback() || v4.is_private()))
        }
        Err(_) => host != "localhost" && !host.ends_with(".localhost") && !host.ends_with(".local"),
    }
}

#[cfg(test)]
mod tests {
    use super::is_public_http_url;

    #[test]
    fn avatar_urls_must_be_public_http() {
        let ok = |raw: &str| is_public_http_url(&reqwest::Url::parse(raw).unwrap());
        assert!(ok("https://image.tmdb.org/t/p/w185/a.jpg"));
        assert!(ok("http://lain.bgm.tv/pic/crt/l/a.jpg"));
        for bad in ["file:///etc/passwd", "http://localhost:8080/x", "http://127.0.0.1/x", "http://10.0.0.5/x",
            "http://192.168.1.2/x", "http://169.254.169.254/latest", "http://[::1]/x", "http://[fd00::1]/x",
            "http://nas.local/x", "http://[::ffff:127.0.0.1]/x"] {
            assert!(!ok(bad), "{bad}");
        }
    }
}
