//! Shared HTTP client for scrapers — honors env + macOS system proxy (Clash etc.).

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use reqwest::{Client, Proxy};

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

/// Map raw reqwest/TMDB errors to stable i18n keys (`err.*`).
pub fn humanize_error(err: &str) -> String {
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
}
