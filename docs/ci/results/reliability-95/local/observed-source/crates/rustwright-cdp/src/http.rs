//! A tiny HTTP/1.1 GET client for the local DevTools HTTP endpoints.
//!
//! Chrome exposes `/json/version`, `/json/list`, `/json/new` and friends on the
//! same port as the WebSocket. These endpoints are only reachable on loopback,
//! so we avoid pulling in a full HTTP stack (and TLS) and speak HTTP/1.1
//! directly over a `TcpStream`.

use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::error::{CdpError, CdpResult};

/// How long to wait for a DevTools HTTP response.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// A parsed DevTools HTTP endpoint, e.g. `http://127.0.0.1:9222`.
#[derive(Debug, Clone)]
pub struct HttpEndpoint {
    /// Hostname or IP address.
    pub host: String,
    /// TCP port.
    pub port: u16,
}

impl HttpEndpoint {
    /// Parse a `http://host:port` (or bare `host:port`) string.
    pub fn parse(input: &str) -> CdpResult<Self> {
        let trimmed = input
            .trim()
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .trim_end_matches('/');
        if trimmed.is_empty() {
            return Err(CdpError::Http(format!("invalid endpoint `{input}`")));
        }
        let (host, port) = match trimmed.rsplit_once(':') {
            Some((host, port)) => {
                let port = port
                    .parse::<u16>()
                    .map_err(|_| CdpError::Http(format!("invalid port in endpoint `{input}`")))?;
                (host.to_string(), port)
            }
            None => (trimmed.to_string(), 9222),
        };
        if host.is_empty() {
            return Err(CdpError::Http(format!(
                "invalid host in endpoint `{input}`"
            )));
        }
        Ok(Self { host, port })
    }

    /// The base URL for this endpoint.
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

/// Perform an HTTP/1.1 `GET` against the DevTools endpoint and return the
/// response body as a string.
pub async fn http_get(endpoint: &HttpEndpoint, path: &str) -> CdpResult<String> {
    let mut stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port)).await?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        host = endpoint.host,
        port = endpoint.port,
    );
    stream.write_all(request.as_bytes()).await?;
    stream.flush().await?;

    let raw = read_response(&mut stream, HTTP_TIMEOUT).await?;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| CdpError::Http("malformed HTTP response from DevTools".to_string()))?;

    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| CdpError::Http("missing HTTP status from DevTools".to_string()))?;

    if !(200..300).contains(&status) {
        return Err(CdpError::Http(format!(
            "DevTools HTTP {status} for {path}: {}",
            body.trim()
        )));
    }

    if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        decode_chunked(body)
    } else {
        Ok(body.to_string())
    }
}

/// Read an HTTP/1.1 response from `stream`, stopping at `Content-Length`,
/// the end of a chunked body, or EOF — whichever comes first.
///
/// Chrome's DevTools HTTP server may keep the connection open, so relying on
/// EOF alone would block indefinitely.
async fn read_response(stream: &mut TcpStream, timeout: Duration) -> CdpResult<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut raw = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut header_end: Option<usize> = None;
    let mut expected_total: Option<usize> = None;
    let mut chunked = false;

    loop {
        if let Some(total) = expected_total {
            if raw.len() >= total {
                break;
            }
        }
        if let Some(end) = header_end {
            if chunked && chunked_body_complete(&raw[end..]) {
                break;
            }
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(CdpError::Http(
                "timed out reading DevTools HTTP response".to_string(),
            ));
        }
        let read = match tokio::time::timeout(remaining, stream.read(&mut buffer)).await {
            Ok(result) => result?,
            Err(_) => {
                return Err(CdpError::Http(
                    "timed out reading DevTools HTTP response".to_string(),
                ))
            }
        };
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&buffer[..read]);

        if header_end.is_none() {
            if let Some(position) = find_subsequence(&raw, b"\r\n\r\n") {
                let end = position + 4;
                header_end = Some(end);
                let head = String::from_utf8_lossy(&raw[..position]);
                if head
                    .to_ascii_lowercase()
                    .contains("transfer-encoding: chunked")
                {
                    chunked = true;
                } else if let Some(length) = content_length(&head) {
                    expected_total = Some(end + length);
                }
            }
        }
    }

    Ok(raw)
}

fn content_length(head: &str) -> Option<usize> {
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            value.trim().parse::<usize>().ok()
        } else {
            None
        }
    })
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Whether `body` contains a complete chunked payload (terminated by a
/// zero-length chunk followed by the trailing CRLF).
fn chunked_body_complete(body: &[u8]) -> bool {
    let mut rest = body;
    loop {
        let Some(position) = find_subsequence(rest, b"\r\n") else {
            return false;
        };
        let size_line = String::from_utf8_lossy(&rest[..position]);
        let size_hex = size_line.split(';').next().unwrap_or(&size_line).trim();
        let Ok(size) = usize::from_str_radix(size_hex, 16) else {
            return false;
        };
        if size == 0 {
            return true;
        }
        let chunk_start = position + 2;
        if rest.len() < chunk_start + size + 2 {
            return false;
        }
        rest = &rest[chunk_start + size + 2..];
    }
}

/// Perform a `GET` and deserialize the JSON body.
pub async fn http_get_json<T: DeserializeOwned>(
    endpoint: &HttpEndpoint,
    path: &str,
) -> CdpResult<T> {
    let body = http_get(endpoint, path).await?;
    let value = serde_json::from_str(&body)?;
    Ok(value)
}

/// Resolve a WebSocket debugger URL from an HTTP endpoint or accept a raw
/// `ws://` URL as-is.
pub async fn discover_ws_url(endpoint: &str) -> CdpResult<String> {
    if endpoint.starts_with("ws://") || endpoint.starts_with("wss://") {
        return Ok(endpoint.to_string());
    }
    let http = HttpEndpoint::parse(endpoint)?;
    let version: crate::protocol::version::BrowserVersion =
        http_get_json(&http, "/json/version").await?;
    if version.web_socket_debugger_url.is_empty() {
        return Err(CdpError::Http(
            "DevTools /json/version did not include webSocketDebuggerUrl".to_string(),
        ));
    }
    Ok(version.web_socket_debugger_url)
}

fn decode_chunked(body: &str) -> CdpResult<String> {
    let mut out = String::new();
    let mut rest = body;
    loop {
        let (size_line, after) = rest.split_once("\r\n").ok_or_else(|| {
            CdpError::Http("malformed chunked response (missing size line)".to_string())
        })?;
        let size_hex = size_line.split(';').next().unwrap_or(size_line).trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| CdpError::Http(format!("invalid chunk size `{size_hex}`")))?;
        if size == 0 {
            break;
        }
        if after.len() < size {
            return Err(CdpError::Http(
                "malformed chunked response (short chunk)".to_string(),
            ));
        }
        out.push_str(&after[..size]);
        rest = after[size..].strip_prefix("\r\n").unwrap_or(&after[size..]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_length_case_insensitively() {
        let head = "HTTP/1.1 200 OK\r\nContent-Length: 42\r\nX-Other: 1";
        assert_eq!(content_length(head), Some(42));
    }

    #[test]
    fn missing_content_length_is_none() {
        let head = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked";
        assert_eq!(content_length(head), None);
    }

    #[test]
    fn detects_complete_chunked_bodies() {
        assert!(chunked_body_complete(b"5\r\nhello\r\n0\r\n\r\n"));
        assert!(!chunked_body_complete(b"5\r\nhello"));
    }

    #[test]
    fn decodes_chunked_bodies() {
        assert_eq!(
            decode_chunked("5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n").unwrap(),
            "hello world"
        );
    }

    #[test]
    fn endpoint_parses_default_port() {
        let endpoint = HttpEndpoint::parse("http://127.0.0.1").unwrap();
        assert_eq!(endpoint.port, 9222);
        assert_eq!(endpoint.host, "127.0.0.1");
    }
}
