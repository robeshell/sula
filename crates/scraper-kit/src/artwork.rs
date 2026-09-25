use std::path::{Path, PathBuf};

use reqwest::Client;

use crate::http::{reqwest_err, send_with_retry};
use crate::types::ArtworkUrls;

pub async fn download_artwork(
    client: &Client,
    folder: &Path,
    movie_stem: Option<&str>,
    urls: &ArtworkUrls,
) -> Result<DownloadedArtwork, String> {
    let name = |role: &str| movie_stem.map(|stem| format!("{stem}-{role}.jpg"))
        .unwrap_or_else(|| format!("{role}.jpg"));
    let mut out = DownloadedArtwork::default();
    for (role, url) in [("poster", &urls.poster_url), ("fanart", &urls.fanart_url), ("banner", &urls.banner_url)] {
        if let Some(url) = url {
            match download_one(client, folder, &name(role), url).await {
                Ok(path) => match role { "poster" => out.poster_path = Some(path), "fanart" => out.fanart_path = Some(path), _ => out.banner_path = Some(path) },
                Err(error) => out.issues.push(format!("{role}: {error}")),
            }
        }
    }
    Ok(out)
}

async fn download_one(
    client: &Client,
    folder: &Path,
    file_name: &str,
    url: &str,
) -> Result<String, String> {
    let response = send_with_retry(client.get(url))
        .await?
        .error_for_status()
        .map_err(reqwest_err)?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let bytes = response.bytes().await.map_err(reqwest_err)?;
    if !content_type.starts_with("image/") && !looks_like_image(&bytes) {
        let shown = if content_type.is_empty() { "unknown" } else { content_type.as_str() };
        return Err(format!("not an image (content-type {shown})"));
    }
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let path = folder.join(file_name);
    media_core::FilesystemService::new().write_file(&bytes, &path, media_core::WriteOptions {
        collision_policy: media_core::CollisionPolicy::Replace,
        ..Default::default()
    }).map_err(|e| e.to_string())?;
    Ok(file_name.to_string())
}

/// JPEG / PNG / GIF / WebP magic bytes.
fn looks_like_image(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"\x89PNG")
        || bytes.starts_with(b"GIF8")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
}

#[derive(Debug, Clone, Default)]
pub struct DownloadedArtwork {
    pub issues: Vec<String>,
    pub poster_path: Option<String>,
    pub fanart_path: Option<String>,
    pub banner_path: Option<String>,
}

pub fn season_poster_name(season: i32) -> String {
    format!("season{season}-poster.jpg")
}

pub async fn download_to_name(
    client: &Client,
    folder: &Path,
    file_name: &str,
    url: &str,
) -> Result<PathBuf, String> {
    download_one(client, folder, file_name, url).await?;
    Ok(folder.join(file_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn partial_download_retains_successes_and_reports_failure() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0; 2048];
                let n = stream.read(&mut buffer).unwrap();
                let request = String::from_utf8_lossy(&buffer[..n]);
                let response = if request.contains("/poster") { "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: 3\r\nConnection: close\r\n\r\nimg" } else { "HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n" };
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let out = download_artwork(&crate::http::build_client(), dir.path(), Some("Movie"), &ArtworkUrls {
            poster_url: Some(format!("http://{addr}/poster")), fanart_url: Some(format!("http://{addr}/fail")), banner_url: None,
        }).await.unwrap();
        server.join().unwrap();
        assert_eq!(out.poster_path.as_deref(), Some("Movie-poster.jpg"));
        assert!(out.fanart_path.is_none()); assert_eq!(out.issues.len(), 1);
        assert!(dir.path().join("Movie-poster.jpg").exists());
    }

    #[test]
    fn sniffs_image_magic() {
        assert!(looks_like_image(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(looks_like_image(b"\x89PNG\r\n\x1a\n"));
        assert!(!looks_like_image(b"<!DOCTYPE html>"));
        assert!(!looks_like_image(b"RIFF\0\0\0\0WAVE"));
    }

    #[tokio::test]
    async fn non_image_body_is_rejected_and_not_written() {
        let (base, server) = crate::http::serve_routes(3, |line| {
            let (ty, body): (&str, &[u8]) = if line.contains("/html") {
                ("text/html", b"<html>error</html>")
            } else if line.contains("/sniff") {
                ("application/octet-stream", b"GIF89a")
            } else {
                ("", b"RIFF\0\0\0\0WEBPVP8 ")
            };
            let ty = if ty.is_empty() { String::new() } else { format!("Content-Type: {ty}\r\n") };
            format!("HTTP/1.1 200 OK\r\n{ty}Content-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), std::str::from_utf8(body).unwrap())
        });
        let dir = tempfile::tempdir().unwrap();
        let client = Client::builder().no_proxy().build().unwrap();
        let out = download_artwork(&client, dir.path(), None, &ArtworkUrls {
            poster_url: Some(format!("{base}/html")), fanart_url: Some(format!("{base}/sniff")), banner_url: Some(format!("{base}/webp")),
        }).await.unwrap();
        server.join().unwrap();
        assert!(out.poster_path.is_none());
        assert!(out.issues[0].contains("not an image"), "{:?}", out.issues);
        assert!(!dir.path().join("poster.jpg").exists());
        assert_eq!(out.fanart_path.as_deref(), Some("fanart.jpg"));
        assert_eq!(out.banner_path.as_deref(), Some("banner.jpg"));
    }
}
