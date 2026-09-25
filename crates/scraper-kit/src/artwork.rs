use reqwest::Client;

use crate::http::{reqwest_err, send_with_retry};

/// Download one image and check that it really is one. Where (and whether) the
/// bytes are written is the caller's decision.
pub async fn fetch_image(client: &Client, url: &str) -> Result<Vec<u8>, String> {
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
    Ok(bytes.to_vec())
}

/// JPEG / PNG / GIF / WebP magic bytes.
fn looks_like_image(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"\x89PNG")
        || bytes.starts_with(b"GIF8")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
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
        let client = crate::http::build_client();
        let poster = fetch_image(&client, &format!("http://{addr}/poster")).await;
        let fanart = fetch_image(&client, &format!("http://{addr}/fail")).await;
        server.join().unwrap();
        assert_eq!(poster.unwrap(), b"img");
        assert!(fanart.is_err());
    }

    #[test]
    fn sniffs_image_magic() {
        assert!(looks_like_image(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(looks_like_image(b"\x89PNG\r\n\x1a\n"));
        assert!(!looks_like_image(b"<!DOCTYPE html>"));
        assert!(!looks_like_image(b"RIFF\0\0\0\0WAVE"));
    }

    #[tokio::test]
    async fn non_image_body_is_rejected() {
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
        let client = Client::builder().no_proxy().build().unwrap();
        let html = fetch_image(&client, &format!("{base}/html")).await;
        let sniff = fetch_image(&client, &format!("{base}/sniff")).await;
        let webp = fetch_image(&client, &format!("{base}/webp")).await;
        server.join().unwrap();
        assert!(html.unwrap_err().contains("not an image"));
        assert_eq!(sniff.unwrap(), b"GIF89a");
        assert!(webp.unwrap().starts_with(b"RIFF"));
    }
}
