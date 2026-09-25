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
    let bytes = send_with_retry(client.get(url))
        .await?
        .error_for_status()
        .map_err(reqwest_err)?
        .bytes()
        .await
        .map_err(reqwest_err)?;
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let path = folder.join(file_name);
    media_core::FilesystemService::new().write_file(&bytes, &path, media_core::WriteOptions {
        collision_policy: media_core::CollisionPolicy::Replace,
        ..Default::default()
    }).map_err(|e| e.to_string())?;
    Ok(file_name.to_string())
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
                let response = if request.contains("/poster") { "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nimg" } else { "HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n" };
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
}
