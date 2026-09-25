//! Shared HTTP client for scrapers — honors env + macOS system proxy (Clash etc.).

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use reqwest::{Client, Proxy, RequestBuilder, Response};

/// Error marker for HTTP 429 that survived all retries; humanized to `err.rateLimit`.
pub const RATE_LIMITED: &str = "rateLimited";
const MAX_RETRIES: u32 = 3;
const MAX_RETRY_AFTER_SECS: u64 = 10;

/// Build an HTTP client that follows `HTTP(S)_PROXY` and, on macOS, system proxy.
pub fn build_client() -> Client {
    let mut builder = Client::builder()
        .user_agent("sula/0.1.0")
        .timeout(Duration::from_secs(30));

    if let Some(proxy_url) = detect_proxy_url() {
        match Proxy::all(&proxy_url) {
            Ok(proxy) => {
                builder = builder.proxy(proxy);
                tracing::info!("scraper http client using configured proxy");
            }
            Err(_) => {
                tracing::warn!("invalid proxy url, continuing without explicit proxy");
            }
        }
    }

    builder.build().expect("http client")
}

fn detect_proxy_url() -> Option<String> {
    for key in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
    {
        if let Ok(val) = std::env::var(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    if let Some(url) = macos_system_proxy() {
        return Some(url);
    }
    // Clash fake-ip (198.18.0.0/16) with system proxy off: probe local ports.
    if dns_looks_like_clash_fake_ip("api.themoviedb.org") {
        if let Some(url) = probe_local_proxy() {
            return Some(url);
        }
    }
    None
}

fn dns_looks_like_clash_fake_ip(host: &str) -> bool {
    let Ok(mut addrs) = (host, 443u16).to_socket_addrs() else {
        return false;
    };
    addrs.any(|addr| match addr.ip() {
        std::net::IpAddr::V4(v4) => v4.octets()[0] == 198 && v4.octets()[1] == 18,
        _ => false,
    })
}

fn probe_local_proxy() -> Option<String> {
    // Common Clash / Surge / V2RayN local HTTP ports.
    const PORTS: &[u16] = &[7897, 7890, 7891, 33331, 10809, 1087, 6152, 8888, 10808];
    for port in PORTS {
        let addr = format!("127.0.0.1:{port}");
        if TcpStream::connect_timeout(
            &addr.parse().ok()?,
            Duration::from_millis(120),
        )
        .is_ok()
        {
            return Some(format!("http://127.0.0.1:{port}"));
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn macos_system_proxy() -> Option<String> {
    let output = std::process::Command::new("scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_scutil_proxy(&text)
}

#[cfg(not(target_os = "macos"))]
fn macos_system_proxy() -> Option<String> {
    None
}

fn parse_scutil_proxy(text: &str) -> Option<String> {
    let https_on = scutil_flag(text, "HTTPSEnable");
    let http_on = scutil_flag(text, "HTTPEnable");
    let socks_on = scutil_flag(text, "SOCKSEnable");

    if https_on {
        let host = scutil_str(text, "HTTPSProxy")?;
        let port = scutil_str(text, "HTTPSPort")?;
        return Some(format!("http://{host}:{port}"));
    }
    if http_on {
        let host = scutil_str(text, "HTTPProxy")?;
        let port = scutil_str(text, "HTTPPort")?;
        return Some(format!("http://{host}:{port}"));
    }
    if socks_on {
        let host = scutil_str(text, "SOCKSProxy")?;
        let port = scutil_str(text, "SOCKSPort")?;
        return Some(format!("socks5://{host}:{port}"));
    }
    None
}

fn scutil_flag(text: &str, key: &str) -> bool {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(key) {
            let rest = rest.trim().trim_start_matches(':').trim();
            return rest == "1" || rest.eq_ignore_ascii_case("true");
        }
    }
    false
}

fn scutil_str<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(key) {
            let rest = rest.trim().trim_start_matches(':').trim();
            if !rest.is_empty() {
                return Some(rest);
            }
        }
    }
    None
}

/// Stringify a reqwest error without its URL (OMDb / TMDB v3 carry the API key in the query).
pub fn reqwest_err(err: reqwest::Error) -> String {
    err.without_url().to_string()
}

/// Send a request, retrying HTTP 429/503 with `Retry-After` or exponential backoff.
/// A 429 that persists after retries becomes [`RATE_LIMITED`]; other statuses are
/// returned as-is for the caller to handle.
pub async fn send_with_retry(request: RequestBuilder) -> Result<Response, String> {
    let mut attempt = 0;
    loop {
        let Some(this) = request.try_clone() else {
            // Streaming bodies cannot be replayed: send once.
            return finish(request.send().await.map_err(reqwest_err)?);
        };
        let response = this.send().await.map_err(reqwest_err)?;
        let status = response.status().as_u16();
        if (status != 429 && status != 503) || attempt >= MAX_RETRIES {
            return finish(response);
        }
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok());
        let delay = retry_delay(attempt, retry_after);
        tracing::debug!(status, attempt, ?delay, "scraper http retry");
        tokio::time::sleep(delay).await;
        attempt += 1;
    }
}

fn finish(response: Response) -> Result<Response, String> {
    if response.status().as_u16() == 429 {
        return Err(RATE_LIMITED.into());
    }
    Ok(response)
}

/// Delay before retry `attempt` (0-based): `Retry-After` seconds (capped), else 1s/2s/4s.
fn retry_delay(attempt: u32, retry_after: Option<&str>) -> Duration {
    if let Some(secs) = retry_after.and_then(|v| v.trim().parse::<u64>().ok()) {
        return Duration::from_secs(secs.min(MAX_RETRY_AFTER_SECS));
    }
    Duration::from_secs(1u64 << attempt.min(4))
}

/// Mask `apikey=` / `api_key=` / `key=` / `token=` query values in free-form error text.
pub fn redact_secrets(text: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r#"(?i)\b((?:api[_-]?)?key|(?:access[_-]?)?token)=[^&\s#"')]+"#)
            .expect("redact regex")
    });
    re.replace_all(text, "$1=***").into_owned()
}

/// Map raw reqwest/TMDB errors to stable i18n keys (`err.*`).
pub fn humanize_error(err: &str) -> String {
    let redacted = redact_secrets(err);
    let err = redacted.as_str();
    let lower = err.to_ascii_lowercase();

    // Network / proxy first. Tunnel `403 Forbidden` must not become apiKey/forbidden.
    if lower.contains("tunnel")
        || lower.contains("proxy")
        || lower.contains("ssl")
        || lower.contains("certificate")
        || lower.contains("connection")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("dns")
        || lower.contains("error trying to connect")
        || lower.contains("error sending request")
        || lower.contains("unexpected eof")
        || lower.contains("failed to connect")
        || lower.contains("network unreachable")
    {
        return "err.connect".into();
    }

    if lower.contains("api key missing")
        || lower.contains("invalid api key")
        || http_status_mentions(&lower, 401)
    {
        return "err.apiKey".into();
    }
    if http_status_mentions(&lower, 403) || lower.contains("forbidden") {
        return "err.forbidden".into();
    }
    if lower.contains("429") || lower.contains("ratelimited") || http_status_mentions(&lower, 429)
    {
        return "err.rateLimit".into();
    }
    if err.chars().count() > 160 {
        format!("{}…", err.chars().take(160).collect::<String>())
    } else {
        err.to_string()
    }
}

fn http_status_mentions(lower: &str, code: u16) -> bool {
    let code = code.to_string();
    lower.contains(&format!("http {code}"))
        || lower.contains(&format!("status: {code}"))
        || lower.contains(&format!("status {code}"))
        || lower.contains(&format!("{code} unauthorized"))
        || lower.contains(&format!("{code} forbidden"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scutil_https_proxy() {
        let sample = r#"
<dictionary> {
  HTTPEnable : 0
  HTTPSEnable : 1
  HTTPSProxy : 127.0.0.1
  HTTPSPort : 7890
}
"#;
        assert_eq!(
            parse_scutil_proxy(sample).as_deref(),
            Some("http://127.0.0.1:7890")
        );
    }

    #[test]
    fn humanizes_ssl() {
        let msg = humanize_error("error trying to connect: unexpected EOF during handshake");
        assert_eq!(msg, "err.connect");
    }

    #[test]
    fn humanizes_proxy_tunnel_403_as_connect() {
        let tunnel = humanize_error("Tunnel connection failed: 403 Forbidden");
        assert_eq!(tunnel, "err.connect");
    }

    #[test]
    fn humanizes_tmdb_http_401_as_api_key() {
        let msg =
            humanize_error("TMDB HTTP 401 Unauthorized: {\"status_message\":\"Invalid API key\"}");
        assert_eq!(msg, "err.apiKey");
    }

    #[test]
    fn does_not_treat_year_401_as_api_key() {
        let msg = humanize_error("no match for title (401 Thieves)");
        assert_ne!(msg, "err.apiKey");
    }

    #[test]
    fn humanize_redacts_query_keys() {
        let msg = humanize_error(
            "HTTP status server error (500 Internal Server Error) for url (https://www.omdbapi.com/?apikey=SECRET&t=x)",
        );
        assert!(!msg.contains("SECRET"), "{msg}");
        let msg = humanize_error("decode failed for https://api.themoviedb.org/3/tv/1?api_key=SECRET");
        assert!(!msg.contains("SECRET"), "{msg}");
        let long = format!("{}?apikey=SECRET&t=x {}", "x".repeat(120), "y".repeat(120));
        let msg = humanize_error(&long);
        assert!(!msg.contains("SECRET"), "{msg}");
        assert_eq!(
            redact_secrets("?apikey=SECRET&t=x KEY=abc token=t0k"),
            "?apikey=***&t=x KEY=*** token=***"
        );
    }

    #[test]
    fn rate_limited_marker_humanizes() {
        assert_eq!(humanize_error(RATE_LIMITED), "err.rateLimit");
    }

    #[test]
    fn retry_delay_honors_retry_after_then_backoff() {
        assert_eq!(retry_delay(0, Some("2")), Duration::from_secs(2));
        assert_eq!(retry_delay(0, Some("600")), Duration::from_secs(10));
        assert_eq!(retry_delay(0, None), Duration::from_secs(1));
        assert_eq!(retry_delay(1, None), Duration::from_secs(2));
        assert_eq!(retry_delay(2, None), Duration::from_secs(4));
        assert_eq!(
            retry_delay(1, Some("Wed, 21 Oct 2015 07:28:00 GMT")),
            Duration::from_secs(2)
        );
    }

    /// Serve `responses` in order on a local socket; the thread yields the request count.
    fn serve(responses: Vec<&'static str>) -> (String, std::thread::JoinHandle<usize>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            for response in &responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0; 2048];
                let _ = stream.read(&mut buffer).unwrap();
                stream.write_all(response.as_bytes()).unwrap();
            }
            responses.len()
        });
        (format!("http://{addr}/"), handle)
    }

    const TOO_MANY: &str = "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    const OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";

    #[tokio::test]
    async fn retries_429_then_succeeds() {
        let (url, server) = serve(vec![TOO_MANY, TOO_MANY, OK]);
        let client = Client::builder().no_proxy().build().unwrap();
        let response = send_with_retry(client.get(&url)).await.unwrap();
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(server.join().unwrap(), 3);
    }

    #[tokio::test]
    async fn reqwest_err_drops_url_with_key() {
        let (url, server) = serve(vec![
            "HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ]);
        let client = Client::builder().no_proxy().build().unwrap();
        let err = send_with_retry(client.get(format!("{url}?apikey=SECRET&t=x")))
            .await
            .unwrap()
            .error_for_status()
            .map_err(reqwest_err)
            .unwrap_err();
        server.join().unwrap();
        assert!(err.contains("500"), "{err}");
        assert!(!err.contains("SECRET"), "{err}");
    }

    #[tokio::test]
    async fn persistent_429_becomes_rate_limited() {
        let (url, server) = serve(vec![TOO_MANY; MAX_RETRIES as usize + 1]);
        let client = Client::builder().no_proxy().build().unwrap();
        let err = send_with_retry(client.get(&url)).await.unwrap_err();
        assert_eq!(err, RATE_LIMITED);
        assert_eq!(server.join().unwrap(), MAX_RETRIES as usize + 1);
    }
}
