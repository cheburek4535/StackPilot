//! URL and port readiness services.
//!
//! Provides robust readiness checking for TCP ports and HTTP/HTTPS URLs
//! with proper URL parsing, cancellation support, timeout handling,
//! and structured failure diagnostics.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use url::Url;

// ---------------------------------------------------------------------------
// URL parsing — using the `url` crate, not manual parsing
// ---------------------------------------------------------------------------

/// Parsed URL information for readiness checks.
#[derive(Debug, Clone)]
pub struct ParsedTarget {
    /// The host to connect to.
    pub host: String,
    /// The port to connect to.
    pub port: u16,
    /// The path to request (for HTTP checks).
    pub path: String,
    /// Whether this is an HTTPS target.
    pub is_https: bool,
    /// The full original URL string.
    pub original: String,
}

impl ParsedTarget {
    /// Parse a URL string into a structured target.
    ///
    /// Supports:
    /// - `http://host:port/path`
    /// - `https://host:port/path`
    /// - `host:port` (bare host:port, defaults to HTTP)
    /// - `host:port/path`
    pub fn parse(input: &str) -> Result<Self, ReadinessError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(ReadinessError::InvalidTarget {
                target: input.to_string(),
                reason: "Empty URL".to_string(),
            });
        }

        // Try parsing as a full URL first — but only for http/https
        // schemes. `Url::parse("localhost:8080")` succeeds with the opaque
        // scheme "localhost", so anything else must fall through to the
        // bare host:port parser below.
        if let Ok(url) = Url::parse(trimmed) {
            if matches!(url.scheme(), "http" | "https") {
                let host = url
                    .host_str()
                    .ok_or_else(|| ReadinessError::InvalidTarget {
                        target: input.to_string(),
                        reason: "No host in URL".to_string(),
                    })?
                    .to_string();

                let port =
                    url.port_or_known_default()
                        .ok_or_else(|| ReadinessError::InvalidTarget {
                            target: input.to_string(),
                            reason: "No port in URL and cannot infer default".to_string(),
                        })?;

                let path = if url.path().is_empty() {
                    "/".to_string()
                } else {
                    url.path().to_string()
                };

                let is_https = url.scheme() == "https";

                return Ok(ParsedTarget {
                    host,
                    port,
                    path,
                    is_https,
                    original: input.to_string(),
                });
            }
        }

        // Try parsing as bare `host:port` or `host:port/path`.
        let (host_port, path) = match trimmed.split_once('/') {
            Some((hp, p)) => (hp, format!("/{}", p)),
            None => (trimmed, "/".to_string()),
        };

        // Handle IPv6 addresses: [::1]:port
        let (host, port) = if host_port.starts_with('[') {
            // IPv6: [host]:port
            let close_bracket =
                host_port
                    .find(']')
                    .ok_or_else(|| ReadinessError::InvalidTarget {
                        target: input.to_string(),
                        reason: "Unclosed bracket in IPv6 address".to_string(),
                    })?;
            let host = &host_port[1..close_bracket];
            let rest = &host_port[close_bracket + 1..];
            let port_str = rest
                .strip_prefix(':')
                .ok_or_else(|| ReadinessError::InvalidTarget {
                    target: input.to_string(),
                    reason: format!("Missing port after IPv6 address: {}", host_port),
                })?;
            let port: u16 = port_str
                .parse()
                .map_err(|_| ReadinessError::InvalidTarget {
                    target: input.to_string(),
                    reason: format!("Invalid port: {}", port_str),
                })?;
            (host.to_string(), port)
        } else {
            // IPv4 or hostname: host:port
            let (host, port_str) =
                host_port
                    .rsplit_once(':')
                    .ok_or_else(|| ReadinessError::InvalidTarget {
                        target: input.to_string(),
                        reason: format!("No port specified: {}", host_port),
                    })?;
            let port: u16 = port_str
                .parse()
                .map_err(|_| ReadinessError::InvalidTarget {
                    target: input.to_string(),
                    reason: format!("Invalid port: {}", port_str),
                })?;
            (host.to_string(), port)
        };

        Ok(ParsedTarget {
            host,
            port,
            path,
            is_https: false,
            original: input.to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Readiness errors
// ---------------------------------------------------------------------------

/// Errors from readiness checks.
#[derive(Debug, Clone)]
pub enum ReadinessError {
    /// The target URL/host:port could not be parsed.
    InvalidTarget { target: String, reason: String },
    /// DNS resolution failed.
    DnsResolutionFailed { target: String, reason: String },
    /// The readiness check timed out.
    Timeout {
        target: String,
        elapsed: Duration,
        last_error: String,
        dependent_process: Option<String>,
        suggested_action: Option<String>,
    },
    /// The operation was cancelled.
    Cancelled { target: String },
}

impl std::fmt::Display for ReadinessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadinessError::InvalidTarget { target, reason } => {
                write!(f, "Invalid target '{}': {}", target, reason)
            }
            ReadinessError::DnsResolutionFailed { target, reason } => {
                write!(f, "DNS resolution failed for '{}': {}", target, reason)
            }
            ReadinessError::Timeout {
                target,
                elapsed,
                last_error,
                ..
            } => {
                write!(
                    f,
                    "Timeout waiting for '{}': elapsed={}s, last error: {}",
                    target,
                    elapsed.as_secs(),
                    last_error
                )
            }
            ReadinessError::Cancelled { target } => {
                write!(f, "Readiness check cancelled for '{}'", target)
            }
        }
    }
}

impl std::error::Error for ReadinessError {}

// ---------------------------------------------------------------------------
// Readiness check parameters
// ---------------------------------------------------------------------------

/// Parameters for a port readiness check.
#[derive(Debug, Clone)]
pub struct PortReadinessCheck {
    pub host: String,
    pub port: u16,
    pub timeout: Duration,
    pub poll_interval: Duration,
}

impl PortReadinessCheck {
    pub fn new(host: impl Into<String>, port: u16, timeout: Duration) -> Self {
        Self {
            host: host.into(),
            port,
            timeout,
            poll_interval: Duration::from_secs(1),
        }
    }
}

/// Parameters for a URL readiness check.
#[derive(Debug, Clone)]
pub struct UrlReadinessCheck {
    pub url: String,
    pub timeout: Duration,
    pub poll_interval: Duration,
    pub expected_status: Option<u16>,
}

impl UrlReadinessCheck {
    pub fn new(url: impl Into<String>, timeout: Duration) -> Self {
        Self {
            url: url.into(),
            timeout,
            poll_interval: Duration::from_secs(1),
            expected_status: None,
        }
    }
}

/// Result of a readiness check.
#[derive(Debug, Clone)]
pub struct ReadinessResult {
    pub success: bool,
    pub target: String,
    pub elapsed: Duration,
    pub message: String,
    pub suggested_action: Option<String>,
}

// ---------------------------------------------------------------------------
// Readiness service
// ---------------------------------------------------------------------------

/// Loopback aliases for a host name: `localhost` ↔ `127.0.0.1` (plus `::1`),
/// so a port/URL check works regardless of which loopback interface the
/// server bound to.
///
/// Why: Node 17+ resolves `localhost` in OS order (verbatim), and on Windows
/// that is `::1` first — Vite (and other dev servers) then bind ONLY to the
/// IPv6 loopback and print `Local: http://localhost:5174/`, while a check
/// configured against `127.0.0.1` sees nothing. Other hosts pass through
/// unchanged.
pub fn loopback_host_aliases(host: &str) -> Vec<String> {
    let lower = host.trim().to_ascii_lowercase();
    match lower.as_str() {
        "localhost" | "127.0.0.1" | "::1" | "0.0.0.0" => vec![
            "127.0.0.1".to_string(),
            "localhost".to_string(),
            "::1".to_string(),
        ],
        _ => vec![host.to_string()],
    }
}

/// Resolve every `(host, port)` pair — including loopback aliases — into a
/// deduplicated list of socket addresses. Hosts that fail to resolve are
/// skipped (one unresolvable alias must not kill the whole wait: e.g. a
/// system without IPv6 loopback cannot resolve `::1`).
pub fn resolve_loopback_addrs(host: &str, port: u16) -> Vec<std::net::SocketAddr> {
    let mut addrs: Vec<std::net::SocketAddr> = Vec::new();
    for h in loopback_host_aliases(host) {
        let addr_str = format!("{}:{}", h, port);
        if let Ok(a) = addr_str.to_socket_addrs() {
            addrs.extend(a);
        }
    }
    addrs.sort_unstable();
    addrs.dedup();
    addrs
}

/// Check if a TCP port is open on the given host.
pub fn check_port(host: &str, port: u16) -> Result<(), String> {
    let addrs = resolve_loopback_addrs(host, port);
    if addrs.is_empty() {
        return Err(format!("DNS resolve failed for {}:{}", host, port));
    }

    for addr in &addrs {
        if TcpStream::connect_timeout(addr, Duration::from_secs(2)).is_ok() {
            return Ok(());
        }
    }

    Err(format!("Port {}:{} is not open", host, port))
}

/// Check if a URL responds with a 2xx/3xx status code.
pub fn check_url(url: &str) -> Result<u16, String> {
    let parsed = ParsedTarget::parse(url).map_err(|e| e.to_string())?;

    let addrs = resolve_loopback_addrs(&parsed.host, parsed.port);
    if addrs.is_empty() {
        return Err(format!("DNS resolve failed: {}", parsed.host));
    }

    for addr in addrs {
        if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
            let request = format!(
                "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                parsed.path, parsed.host
            );
            if stream.write_all(request.as_bytes()).is_err() {
                continue;
            }

            let mut reader = BufReader::new(&stream);
            let mut first_line = String::new();
            if reader.read_line(&mut first_line).is_err() {
                continue;
            }

            let parts: Vec<&str> = first_line.split_whitespace().collect();
            if let Some(code_str) = parts.get(1) {
                if let Ok(code) = code_str.parse::<u16>() {
                    if (200..400).contains(&code) {
                        return Ok(code);
                    }
                }
            }
        }
    }

    Err(format!("URL '{}' did not respond successfully", url))
}

/// Probe a URL with a single HTTP request, returning the response status
/// code. `Ok(code)` for ANY HTTP response (2xx, 3xx, 4xx, 5xx) — unlike
/// [`check_url`] which only accepts 2xx/3xx — and `Err` only when the
/// server cannot be reached at all (DNS/connection failure).
pub fn probe_url_status(url: &str) -> Result<u16, String> {
    let parsed = ParsedTarget::parse(url).map_err(|e| e.to_string())?;
    let addrs = resolve_loopback_addrs(&parsed.host, parsed.port);
    if addrs.is_empty() {
        return Err(format!("DNS resolve failed: {}", parsed.host));
    }

    for addr in addrs {
        if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
            let request = format!(
                "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                parsed.path, parsed.host
            );
            if stream.write_all(request.as_bytes()).is_err() {
                continue;
            }
            let mut reader = BufReader::new(&stream);
            let mut first_line = String::new();
            if reader.read_line(&mut first_line).is_err() {
                continue;
            }
            let parts: Vec<&str> = first_line.split_whitespace().collect();
            if let Some(code_str) = parts.get(1) {
                if let Ok(code) = code_str.parse::<u16>() {
                    return Ok(code);
                }
            }
        }
    }

    Err(format!("URL '{}' is not reachable", url))
}

/// Decide which URL a browser tab should actually open.
///
/// Non-root doc URLs (`/docs`, `/swagger-ui/index.html`, ...) frequently
/// 404: the framework's docs package may not be installed (e.g. NestJS
/// without `@nestjs/swagger`) or the route differs from the default. A
/// browser tab on a dead page is worse than no tab at all, so the requested
/// path is probed first and the ORIGIN ROOT is opened instead when the
/// server answers with an error status. The original URL is kept as the
/// best-effort fallback when the server is still unreachable (it may simply
/// be warming up).
///
/// - 2xx/3xx on the requested path → the path itself.
/// - definitive 4xx/5xx on the path → the origin root (when it responds).
/// - unreachable server → the original URL (best effort).
pub fn resolve_browser_url(url: &str) -> Result<String, String> {
    let parsed = ParsedTarget::parse(url).map_err(|e| e.to_string())?;
    // Root URLs have nothing to fall back to — open them directly.
    if parsed.path == "/" {
        return Ok(url.to_string());
    }

    let origin = format!(
        "{}://{}:{}/",
        if parsed.is_https { "https" } else { "http" },
        parsed.host,
        parsed.port
    );

    // Probe the requested path: up to ~8s, 500ms apart. An HTTP error
    // status is decisive and stops the loop immediately.
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        match probe_url_status(url) {
            Ok(code) if (200..400).contains(&code) => return Ok(url.to_string()),
            Ok(_) => break,
            Err(_) => {
                if Instant::now() >= deadline {
                    return Ok(url.to_string());
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }

    // The path is broken (404/500...): fall back to the origin root.
    match probe_url_status(&origin) {
        Ok(code) if (200..400).contains(&code) => Ok(origin),
        _ => Ok(url.to_string()),
    }
}

/// Wait for a TCP port to become open, with cancellation support.
pub async fn wait_for_port(
    check: &PortReadinessCheck,
    cancelled: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> ReadinessResult {
    let start = Instant::now();
    let deadline = start + check.timeout;
    let target = format!("{}:{}", check.host, check.port);

    // Loopback aliases included: a server bound to `::1` (Vite on Windows)
    // must still satisfy a wait configured against `127.0.0.1`.
    let addrs = resolve_loopback_addrs(&check.host, check.port);
    if addrs.is_empty() {
        return ReadinessResult {
            success: false,
            target: target.clone(),
            elapsed: start.elapsed(),
            message: format!("DNS resolution failed: {}", check.host),
            suggested_action: Some(format!(
                "Ensure '{}' resolves to a valid address.",
                check.host
            )),
        };
    }

    loop {
        if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
            return ReadinessResult {
                success: false,
                target: target.clone(),
                elapsed: start.elapsed(),
                message: "Readiness check cancelled".to_string(),
                suggested_action: None,
            };
        }

        if Instant::now() >= deadline {
            return ReadinessResult {
                success: false,
                target: target.clone(),
                elapsed: start.elapsed(),
                message: format!(
                    "Port {}:{} not open after {}s",
                    check.host,
                    check.port,
                    check.timeout.as_secs()
                ),
                suggested_action: Some(format!(
                    "Ensure a service is listening on {}:{}.\n\
                     Check if the service is running and has started successfully.",
                    check.host, check.port
                )),
            };
        }

        for addr in &addrs {
            if TcpStream::connect_timeout(addr, Duration::from_secs(2)).is_ok() {
                return ReadinessResult {
                    success: true,
                    target: target.clone(),
                    elapsed: start.elapsed(),
                    message: format!("Port {}:{} is open", check.host, check.port),
                    suggested_action: None,
                };
            }
        }

        let _ = tokio::time::timeout(
            check.poll_interval,
            tokio::time::sleep(Duration::from_millis(100)),
        )
        .await;
    }
}

/// Wait for a URL to respond successfully, with cancellation support.
pub async fn wait_for_url(
    check: &UrlReadinessCheck,
    cancelled: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> ReadinessResult {
    let start = Instant::now();
    let deadline = start + check.timeout;

    let parsed = match ParsedTarget::parse(&check.url) {
        Ok(p) => p,
        Err(e) => {
            return ReadinessResult {
                success: false,
                target: check.url.clone(),
                elapsed: start.elapsed(),
                message: e.to_string(),
                suggested_action: None,
            };
        }
    };

    // Loopback aliases included: a server bound to `::1` (Vite on Windows)
    // must still satisfy a URL wait configured against `127.0.0.1`.
    let addrs = resolve_loopback_addrs(&parsed.host, parsed.port);
    if addrs.is_empty() {
        return ReadinessResult {
            success: false,
            target: check.url.clone(),
            elapsed: start.elapsed(),
            message: format!("DNS resolution failed: {}", parsed.host),
            suggested_action: Some(format!(
                "Ensure '{}' resolves to a valid address.",
                parsed.host
            )),
        };
    }

    let mut last_error = String::new();

    loop {
        if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
            return ReadinessResult {
                success: false,
                target: check.url.clone(),
                elapsed: start.elapsed(),
                message: "Readiness check cancelled".to_string(),
                suggested_action: None,
            };
        }

        if Instant::now() >= deadline {
            return ReadinessResult {
                success: false,
                target: check.url.clone(),
                elapsed: start.elapsed(),
                message: format!(
                    "URL '{}' not available after {}s. Last error: {}",
                    check.url,
                    check.timeout.as_secs(),
                    last_error
                ),
                suggested_action: Some(format!(
                    "Ensure the server is running and responding at '{}'.\n\
                     Check if the service has started correctly and the port is open.",
                    check.url
                )),
            };
        }

        for addr in &addrs {
            if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
                let request = format!(
                    "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                    parsed.path, parsed.host
                );
                if stream.write_all(request.as_bytes()).is_ok() {
                    let mut reader = BufReader::new(&stream);
                    let mut first_line = String::new();
                    if reader.read_line(&mut first_line).is_ok() {
                        let parts: Vec<&str> = first_line.split_whitespace().collect();
                        if let Some(code_str) = parts.get(1) {
                            if let Ok(code) = code_str.parse::<u16>() {
                                let status_ok = if let Some(expected) = check.expected_status {
                                    code == expected
                                } else {
                                    (200..400).contains(&code)
                                };
                                if status_ok {
                                    return ReadinessResult {
                                        success: true,
                                        target: check.url.clone(),
                                        elapsed: start.elapsed(),
                                        message: format!(
                                            "URL '{}' responded with {}",
                                            check.url, code
                                        ),
                                        suggested_action: None,
                                    };
                                }
                                last_error = format!("HTTP {}", code);
                            }
                        }
                    }
                }
            }
        }

        let _ = tokio::time::timeout(
            check.poll_interval,
            tokio::time::sleep(Duration::from_millis(100)),
        )
        .await;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_url_full() {
        let parsed = ParsedTarget::parse("http://localhost:3000/api").unwrap();
        assert_eq!(parsed.host, "localhost");
        assert_eq!(parsed.port, 3000);
        assert_eq!(parsed.path, "/api");
        assert!(!parsed.is_https);
    }

    #[test]
    fn parse_url_https() {
        let parsed = ParsedTarget::parse("https://example.com:443/").unwrap();
        assert_eq!(parsed.host, "example.com");
        assert_eq!(parsed.port, 443);
        assert!(parsed.is_https);
    }

    #[test]
    fn parse_url_bare_host_port() {
        let parsed = ParsedTarget::parse("localhost:8080").unwrap();
        assert_eq!(parsed.host, "localhost");
        assert_eq!(parsed.port, 8080);
        assert_eq!(parsed.path, "/");
    }

    #[test]
    fn parse_url_bare_host_port_path() {
        let parsed = ParsedTarget::parse("localhost:8080/health").unwrap();
        assert_eq!(parsed.host, "localhost");
        assert_eq!(parsed.port, 8080);
        assert_eq!(parsed.path, "/health");
    }

    #[test]
    fn parse_url_ipv6() {
        let parsed = ParsedTarget::parse("[::1]:3000/path").unwrap();
        assert_eq!(parsed.host, "::1");
        assert_eq!(parsed.port, 3000);
        assert_eq!(parsed.path, "/path");
    }

    #[test]
    fn parse_url_empty_error() {
        assert!(ParsedTarget::parse("").is_err());
    }

    #[test]
    fn parse_url_no_port_error() {
        assert!(ParsedTarget::parse("localhost").is_err());
    }

    #[test]
    fn parse_url_invalid_port_error() {
        assert!(ParsedTarget::parse("localhost:notaport").is_err());
    }

    #[test]
    fn readiness_error_display() {
        let err = ReadinessError::Timeout {
            target: "localhost:3000".to_string(),
            elapsed: Duration::from_secs(30),
            last_error: "connection refused".to_string(),
            dependent_process: None,
            suggested_action: None,
        };
        let display = format!("{}", err);
        assert!(display.contains("Timeout"));
        assert!(display.contains("localhost:3000"));
    }

    #[test]
    fn readiness_result_fields() {
        let result = ReadinessResult {
            success: true,
            target: "localhost:3000".to_string(),
            elapsed: Duration::from_secs(1),
            message: "Port open".to_string(),
            suggested_action: None,
        };
        assert!(result.success);
        assert_eq!(result.target, "localhost:3000");
    }

    #[test]
    fn check_port_does_not_panic() {
        // This will fail (no server running) but should not panic.
        let _ = check_port("localhost", 1);
    }

    /// A server bound to the IPv6 loopback ONLY (Vite on Windows: Node 17+
    /// resolves `localhost` to ::1 first and binds just that interface)
    /// must still be detected by a check configured against `127.0.0.1`.
    #[test]
    fn check_port_finds_ipv6_only_listener_via_loopback_aliases() {
        fn bind_v6() -> Option<std::net::TcpListener> {
            std::net::TcpListener::bind("[::1]:0").ok()
        }
        // A proper request/response loop: read the request first, then
        // answer — matching a real HTTP server and avoiding the accept
        // race of a write-only responder.
        fn serve(listener: std::net::TcpListener, respond: bool) -> std::thread::JoinHandle<()> {
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    if respond {
                        let mut reader = BufReader::new(&mut stream);
                        let mut request = String::new();
                        let _ = reader.read_line(&mut request);
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
                    }
                }
            })
        }

        // Each check gets its own listener: the port check connects and
        // immediately disconnects, which would race a shared listener.
        let port_listener = match bind_v6() {
            Some(l) => l,
            None => return, // host without an IPv6 loopback — nothing to test
        };
        let port = port_listener.local_addr().unwrap().port();
        let _thread = serve(port_listener, false);

        assert!(
            check_port("127.0.0.1", port).is_ok(),
            "127.0.0.1 check must find a ::1-only listener"
        );

        let url_listener = match bind_v6() {
            Some(l) => l,
            None => return,
        };
        let url_port = url_listener.local_addr().unwrap().port();
        let _thread = serve(url_listener, true);

        let url = format!("http://127.0.0.1:{}/", url_port);
        // Readiness checks are retried in production; the test polls the
        // same way so a cold accept thread cannot make it flaky.
        let mut last_err = String::new();
        let mut ok = false;
        for _ in 0..5 {
            match check_url(&url) {
                Ok(_) => {
                    ok = true;
                    break;
                }
                Err(e) => {
                    last_err = e;
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
        assert!(ok, "URL check against 127.0.0.1 must reach a ::1-only listener: {last_err}");
    }

    #[test]
    fn check_url_does_not_panic() {
        let _ = check_url("http://localhost:1/health");
    }

    #[test]
    fn probe_url_status_returns_error_codes() {
        // A tiny HTTP server: /docs → 404, / → 200.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut request = String::new();
                let _ = reader.read_line(&mut request);
                let (code, body) = if request.contains("/docs") {
                    ("404 Not Found", "not found")
                } else {
                    ("200 OK", "hello")
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    code,
                    body.len(),
                    body
                );
            }
        });

        assert_eq!(
            probe_url_status(&format!("http://127.0.0.1:{}/", port)).unwrap(),
            200
        );
        assert_eq!(
            probe_url_status(&format!("http://127.0.0.1:{}/docs", port)).unwrap(),
            404
        );
        // Unreachable server → Err, not a status code.
        assert!(probe_url_status("http://127.0.0.1:1/x").is_err());
    }

    #[test]
    fn resolve_browser_url_falls_back_to_root_on_404() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut request = String::new();
                let _ = reader.read_line(&mut request);
                let (code, body) = if request.contains("/docs") {
                    ("404 Not Found", "not found")
                } else {
                    ("200 OK", "hello")
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    code,
                    body.len(),
                    body
                );
            }
        });

        // A 404 on the doc path → the origin root is opened instead.
        let resolved = resolve_browser_url(&format!("http://127.0.0.1:{}/docs", port)).unwrap();
        assert_eq!(resolved, format!("http://127.0.0.1:{}/", port));
        // A 200 on the doc path → the path itself.
        let resolved = resolve_browser_url(&format!("http://127.0.0.1:{}/", port)).unwrap();
        assert_eq!(resolved, format!("http://127.0.0.1:{}/", port));
        // Root URLs are returned unchanged.
        let root = format!("http://127.0.0.1:{}/", port);
        assert_eq!(resolve_browser_url(&root).unwrap(), root);
    }

    #[test]
    fn port_readiness_check_defaults() {
        let check = PortReadinessCheck::new("localhost", 3000, Duration::from_secs(10));
        assert_eq!(check.host, "localhost");
        assert_eq!(check.port, 3000);
        assert_eq!(check.timeout, Duration::from_secs(10));
    }

    #[test]
    fn url_readiness_check_defaults() {
        let check = UrlReadinessCheck::new("http://localhost:3000", Duration::from_secs(10));
        assert_eq!(check.url, "http://localhost:3000");
        assert_eq!(check.timeout, Duration::from_secs(10));
        assert!(check.expected_status.is_none());
    }

    #[test]
    fn loopback_aliases_cover_localhost_variants() {
        let aliases = loopback_host_aliases("localhost");
        assert!(aliases.contains(&"127.0.0.1".to_string()));
        assert!(aliases.contains(&"::1".to_string()));
        let aliases = loopback_host_aliases("127.0.0.1");
        assert!(aliases.contains(&"localhost".to_string()));
        // Foreign hosts pass through untouched.
        assert_eq!(loopback_host_aliases("192.168.1.10"), vec!["192.168.1.10"]);
        assert_eq!(
            loopback_host_aliases("api.example.com"),
            vec!["api.example.com"]
        );
        // Resolver skips unresolvable aliases instead of failing wholesale.
        assert!(!resolve_loopback_addrs("127.0.0.1", 5174).is_empty());
    }
}
