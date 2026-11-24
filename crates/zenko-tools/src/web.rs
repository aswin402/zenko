//! SSRF-safe, streaming-bounded HTTP fetcher with deterministic URL parsing and
//! RFC-compliant IP range filtering — zero regex, zero unbounded RAM reads.

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    DEFAULT_WEB_FETCH_MAX_BYTES, DEFAULT_WEB_FETCH_MAX_REDIRECTS, DEFAULT_WEB_FETCH_TIMEOUT_SECS,
    DEFAULT_WEB_FETCH_USER_AGENT, Result, RiskLevel, TOOL_WEB_FETCH, Tool, ToolContext,
    ToolExposure, ToolOutput, ToolSpec, ZenkoError, sanitize_untrusted_markup,
};

struct SsrfSafeResolver;

impl reqwest::dns::Resolve for SsrfSafeResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            if is_blocked_hostname(&host) {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("blocked local/internal host `{host}` (SSRF protection)"),
                )) as Box<dyn std::error::Error + Send + Sync>);
            }
            let resolved: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
                .collect();
            for sock in &resolved {
                if is_blocked_ip(sock.ip()) {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!(
                            "host `{host}` resolved to private/loopback IP `{}` (SSRF protection)",
                            sock.ip()
                        ),
                    )) as Box<dyn std::error::Error + Send + Sync>);
                }
            }
            if resolved.is_empty() {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("DNS returned no addresses for `{host}`"),
                )) as Box<dyn std::error::Error + Send + Sync>);
            }
            let addrs: reqwest::dns::Addrs = Box::new(resolved.into_iter());
            Ok(addrs)
        })
    }
}

pub(crate) fn build_ssrf_safe_http_client() -> Result<Client> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(std::sync::Arc::new(SsrfSafeResolver))
        .pool_max_idle_per_host(4)
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| ZenkoError::Tool(format!("failed to build SSRF-safe HTTP client: {e}")))
}

pub struct WebFetchTool {
    client: Option<Client>,
}

impl WebFetchTool {
    pub fn new(_fallback_client: Client) -> Self {
        Self {
            client: build_ssrf_safe_http_client().ok(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedHttpUrl {
    pub(crate) scheme: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) origin: String,
    pub(crate) base_dir: String,
}

pub(crate) fn parse_http_url(raw: &str) -> Result<ParsedHttpUrl> {
    let trimmed = raw.trim();
    let (scheme, default_port, rest) = match trimmed.split_once("://") {
        Some((s, r)) if s.eq_ignore_ascii_case("https") => ("https", 443u16, r),
        Some((s, r)) if s.eq_ignore_ascii_case("http") => ("http", 80u16, r),
        _ => return Err(ZenkoError::PolicyDenied("only http:// and https:// URLs are supported".into())),
    };

    let authority_end = rest
        .find(['/', '?', '#'])
        .unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let path_and_query = &rest[authority_end..];
    let path_only = path_and_query
        .split(['?', '#'])
        .next()
        .unwrap_or("/");
    let base_dir = match path_only.rfind('/') {
        Some(idx) => path_only[..=idx].to_string(),
        None => "/".to_string(),
    };
    let host_port = authority
        .rsplit_once('@')
        .map(|(_, hp)| hp)
        .unwrap_or(authority);

    let (host, port) = match host_port.starts_with('[') {
        true => {
            let close = host_port
                .find(']')
                .ok_or_else(|| ZenkoError::Tool("malformed IPv6 host in URL".into()))?;
            let h = &host_port[1..close];
            let suffix = &host_port[close + 1..];
            let p = match suffix {
                "" => default_port,
                s => match s.strip_prefix(':') {
                    Some(port_str) => port_str
                        .parse::<u16>()
                        .map_err(|_| ZenkoError::Tool("invalid port in URL".into()))?,
                    None => return Err(ZenkoError::Tool("malformed trailing characters after IPv6 host".into())),
                },
            };
            (h.to_string(), p)
        }
        false => match host_port.rsplit_once(':') {
            Some((h, port_str)) => {
                let p = port_str
                    .parse::<u16>()
                    .map_err(|_| ZenkoError::Tool("invalid port in URL".into()))?;
                (h.to_string(), p)
            }
            None => (host_port.to_string(), default_port),
        },
    };

    match host.is_empty() {
        true => Err(ZenkoError::Tool("missing hostname in URL".into())),
        false => Ok(ParsedHttpUrl {
            scheme: scheme.to_string(),
            host,
            port,
            origin: format!("{scheme}://{host_port}"),
            base_dir,
        }),
    }
}

/// Bitwise/structural RFC IP classifier blocking loopback, RFC1918 private,
/// link-local / cloud metadata (`169.254.0.0/16`), CGNAT (`100.64.0.0/10`),
/// multicast, unspecified, ULA (`fc00::/7`), and IPv4-mapped IPv6 bypasses.
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_ipv4(v4),
        IpAddr::V6(v6) => is_blocked_ipv6(v6),
    }
}

fn is_blocked_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        || (a == 100 && (b & 0b1100_0000) == 64) // 100.64.0.0/10 CGNAT
        || (a == 198 && (b & 0b1111_1110) == 18) // 198.18.0.0/15 benchmark
        || a >= 240 // 240.0.0.0/4 reserved
}

fn is_blocked_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(mapped_v4) = ip.to_ipv4_mapped() {
        return is_blocked_ipv4(mapped_v4);
    }
    let segs = ip.segments();
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (segs[0] & 0xfe00) == 0xfc00 // fc00::/7 Unique Local
        || (segs[0] & 0xffc0) == 0xfe80 // fe80::/10 Link-Local
}

fn is_blocked_hostname(host: &str) -> bool {
    let lower = host.trim().trim_end_matches('.').to_ascii_lowercase();
    lower == "localhost"
        || lower.ends_with(".localhost")
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
        || lower == "metadata.google.internal"
}

pub(crate) async fn verify_url_safe(raw: &str) -> Result<ParsedHttpUrl> {
    let parsed = parse_http_url(raw)?;
    if is_blocked_hostname(&parsed.host) {
        return Err(ZenkoError::PolicyDenied(format!(
            "blocked local/internal host `{}` (SSRF protection)",
            parsed.host
        )));
    }

    if let Ok(ip) = parsed.host.parse::<IpAddr>() {
        return match is_blocked_ip(ip) {
            true => Err(ZenkoError::PolicyDenied(format!(
                "blocked private/loopback IP `{ip}` (SSRF protection)"
            ))),
            false => Ok(parsed),
        };
    }

    let addrs = tokio::net::lookup_host((parsed.host.as_str(), parsed.port))
        .await
        .map_err(|e| ZenkoError::Tool(format!("DNS resolution failed for `{}`: {e}", parsed.host)))?;

    let mut resolved_any = false;
    for sock in addrs {
        resolved_any = true;
        if is_blocked_ip(sock.ip()) {
            return Err(ZenkoError::PolicyDenied(format!(
                "host `{}` resolved to private/loopback IP `{}` (SSRF protection)",
                parsed.host,
                sock.ip()
            )));
        }
    }

    match resolved_any {
        true => Ok(parsed),
        false => Err(ZenkoError::Tool(format!(
            "DNS returned no addresses for `{}`",
            parsed.host
        ))),
    }
}

fn has_explicit_scheme(loc: &str) -> bool {
    let Some(colon) = loc.find(':') else {
        return false;
    };
    if colon == 0 {
        return false;
    }
    let bytes = loc.as_bytes();
    bytes[0].is_ascii_alphabetic()
        && bytes[1..colon]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
}

fn resolve_redirect_url(base: &ParsedHttpUrl, location: &str) -> String {
    let loc = location.trim();
    match (
        has_explicit_scheme(loc),
        loc.starts_with("//"),
        loc.starts_with('/'),
    ) {
        (true, _, _) => loc.to_string(),
        (_, true, _) => format!("{}:{loc}", base.scheme),
        (_, _, true) => format!("{}{loc}", base.origin),
        _ => format!("{}{}{loc}", base.origin, base.base_dir),
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Web
    }

    fn concurrency(&self) -> zenko_core::ToolConcurrency {
        zenko_core::ToolConcurrency::Shared
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_WEB_FETCH.into(),
            description: "Fetch a public HTTP/HTTPS URL safely (SSRF-protected, streaming byte-bounded) and return clean readable text.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Public HTTP or HTTPS URL to fetch" },
                    "max_chars": { "type": "integer", "description": "Maximum characters to return (default 10000)" }
                },
                "required": ["url"]
            }),
            risk: RiskLevel::Moderate,
        }
    }

    async fn execute(
        &self,
        _ctx: &ToolContext,
        args: Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let initial_url = args
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `url`".into()))?;
        let max_chars = args
            .get("max_chars")
            .and_then(Value::as_u64)
            .unwrap_or(10_000)
            .clamp(500, 32_000) as usize;

        let fetch_fut = self.fetch_with_verified_redirects(initial_url, cancel.clone());
        let (final_url, status, raw_bytes, byte_capped) = tokio::select! {
            _ = cancel.cancelled() => return Err(ZenkoError::Cancelled),
            res = tokio::time::timeout(Duration::from_secs(DEFAULT_WEB_FETCH_TIMEOUT_SECS), fetch_fut) => {
                match res {
                    Ok(inner) => inner?,
                    Err(_) => return Ok(ToolOutput::err(format!("Fetch timed out after {DEFAULT_WEB_FETCH_TIMEOUT_SECS}s"))),
                }
            }
        };

        let body = String::from_utf8_lossy(&raw_bytes);
        let cleaned = sanitize_untrusted_markup(&strip_html_to_text(&body));
        let final_text = match cleaned.len() > max_chars {
            true => {
                let cut = cleaned.floor_char_boundary(max_chars);
                format!("{}\n\n[truncated at {max_chars} chars]", &cleaned[..cut])
            }
            false => cleaned,
        };
        let cap_note = match byte_capped {
            true => format!(" [body capped at {} KB]", DEFAULT_WEB_FETCH_MAX_BYTES / 1024),
            false => String::new(),
        };

        Ok(ToolOutput::ok(format!(
            "HTTP {status} — {final_url}{cap_note}\n\n{final_text}"
        )))
    }
}

impl WebFetchTool {
    async fn fetch_with_verified_redirects(
        &self,
        start_url: &str,
        cancel: CancellationToken,
    ) -> Result<(String, reqwest::StatusCode, Vec<u8>, bool)> {
        let client = self.client.as_ref().ok_or_else(|| {
            ZenkoError::Tool("SSRF-safe HTTP client failed to initialize".into())
        })?;
        let mut current_url = start_url.to_string();

        for _hop in 0..=DEFAULT_WEB_FETCH_MAX_REDIRECTS {
            let parsed = verify_url_safe(&current_url).await?;
            let resp = tokio::select! {
                _ = cancel.cancelled() => return Err(ZenkoError::Cancelled),
                res = client.get(&current_url).header("User-Agent", DEFAULT_WEB_FETCH_USER_AGENT).send() => {
                    res.map_err(|e| ZenkoError::Tool(format!("fetch failed: {e}")))?
                }
            };

            let status = resp.status();
            if status.is_redirection() {
                let Some(loc) = resp.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()) else {
                    return Err(ZenkoError::Tool("redirect response missing Location header".into()));
                };
                current_url = resolve_redirect_url(&parsed, loc);
                continue;
            }

            let mut resp = resp;
            let mut buf = Vec::with_capacity(DEFAULT_WEB_FETCH_MAX_BYTES.min(32_768));
            let mut capped = false;
            while let Some(chunk) = resp
                .chunk()
                .await
                .map_err(|e| ZenkoError::Tool(format!("read body failed: {e}")))?
            {
                let remaining = DEFAULT_WEB_FETCH_MAX_BYTES.saturating_sub(buf.len());
                match chunk.len() <= remaining {
                    true => buf.extend_from_slice(&chunk),
                    false => {
                        buf.extend_from_slice(&chunk[..remaining]);
                        capped = true;
                        break;
                    }
                }
            }
            return Ok((current_url, status, buf, capped));
        }

        Err(ZenkoError::Tool(format!(
            "exceeded maximum redirects ({DEFAULT_WEB_FETCH_MAX_REDIRECTS})"
        )))
    }
}

#[derive(Clone, Copy)]
enum HtmlState {
    Text,
    Tag,
    SkipScript,
    SkipStyle,
}

fn strip_html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len().min(32_768));
    let mut state = HtmlState::Text;
    let lower = html.to_ascii_lowercase();
    let lower_bytes = lower.as_bytes();
    let mut i = 0;

    while i < html.len() {
        let rest_lower = &lower_bytes[i..];
        match state {
            HtmlState::SkipScript => match rest_lower.starts_with(b"</script>") {
                true => {
                    state = HtmlState::Text;
                    i += 9;
                }
                false => i += 1,
            },
            HtmlState::SkipStyle => match rest_lower.starts_with(b"</style>") {
                true => {
                    state = HtmlState::Text;
                    i += 8;
                }
                false => i += 1,
            },
            HtmlState::Text | HtmlState::Tag => {
                if rest_lower.starts_with(b"<script") {
                    state = HtmlState::SkipScript;
                    i += 7;
                    continue;
                }
                if rest_lower.starts_with(b"<style") {
                    state = HtmlState::SkipStyle;
                    i += 6;
                    continue;
                }
                let ch = html[i..].chars().next().unwrap_or(' ');
                i += ch.len_utf8();
                match (state, ch) {
                    (_, '<') => state = HtmlState::Tag,
                    (HtmlState::Tag, '>') => {
                        state = HtmlState::Text;
                        if !out.ends_with('\n') && !out.ends_with(' ') {
                            out.push(' ');
                        }
                    }
                    (HtmlState::Text, c) => out.push(c),
                    (HtmlState::Tag, _) => {}
                    _ => {}
                }
            }
        }
    }

    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssrf_classifier_blocks_local_private_metadata_and_mapped_ips() {
        for blocked in [
            "127.0.0.1",
            "10.0.0.5",
            "172.16.1.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fc00::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
        ] {
            let ip: IpAddr = blocked.parse().expect("valid ip");
            assert!(is_blocked_ip(ip), "expected {blocked} to be blocked");
        }

        for public in ["1.1.1.1", "8.8.8.8", "93.184.216.34", "2606:4700:4700::1111"] {
            let ip: IpAddr = public.parse().expect("valid ip");
            assert!(!is_blocked_ip(ip), "expected {public} to be allowed");
        }

        assert!(is_blocked_hostname("localhost"));
        assert!(is_blocked_hostname("metadata.google.internal"));
        assert!(!is_blocked_hostname("docs.rs"));
    }

    #[test]
    fn html_state_machine_strips_scripts_styles_and_tags() {
        let html = "<html><style>body{color:red}</style><script>alert(1)</script><h1>Zenko</h1><p>Fast Rust</p></html>";
        assert_eq!(strip_html_to_text(html), "Zenko Fast Rust");
    }

    #[test]
    fn parse_http_url_rejects_malformed_ipv6_trailing_garbage() {
        let p1 = parse_http_url("https://[::1]:8080/a").expect("valid ipv6 with port");
        assert_eq!(p1.host, "::1");
        assert_eq!(p1.port, 8080);

        let p2 = parse_http_url("https://[2606:4700::1111]/ok").expect("valid ipv6");
        assert_eq!(p2.host, "2606:4700::1111");

        assert!(parse_http_url("https://[::1]evil.com/a").is_err());
        assert!(parse_http_url("https://[::1]:notaport/a").is_err());
    }

    #[test]
    fn resolve_redirect_url_preserves_directory_and_passes_bad_schemes_to_validator() {
        let base = parse_http_url("https://docs.rs/zenko/latest/index.html").expect("base");
        assert_eq!(
            resolve_redirect_url(&base, "guide.html"),
            "https://docs.rs/zenko/latest/guide.html"
        );
        assert_eq!(resolve_redirect_url(&base, "/root"), "https://docs.rs/root");
        assert_eq!(
            resolve_redirect_url(&base, "HTTPS://example.com/b"),
            "HTTPS://example.com/b"
        );
        let file_redir = resolve_redirect_url(&base, "file:///etc/passwd");
        assert!(parse_http_url(&file_redir).is_err());
        let js_redir = resolve_redirect_url(&base, "javascript:alert(1)");
        assert!(parse_http_url(&js_redir).is_err());
    }
}
