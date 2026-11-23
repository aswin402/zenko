use crate::WebState;
use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;
use std::time::Instant;
use zenko_core::{
    LOG_SSE_SKIP_PATH, LogChannel, LogDirection, LogStatusTone, emit_hono_line, format_latency,
    print_hono_line,
};

/// Hono-style HTTP request middleware reusing Zenko's unified `zenko_core::logger`.
/// Terminal: `<-- HTTP    GET /path` and `--> HTTP    GET /path 200 1.4ms`
/// WebUI log ring: pushes the completed structured `--> HTTP` entry.
pub async fn hono_logger_middleware(
    State(state): State<WebState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let subject = format!("{method} {path}");
    let should_log = path != LOG_SSE_SKIP_PATH;

    if should_log {
        print_hono_line(LogDirection::In, LogChannel::Http, &subject, None, None);
    }

    let started = Instant::now();
    let response = next.run(req).await;

    if should_log {
        let elapsed = format_latency(started.elapsed());
        let code = response.status().as_u16();
        let code_str = code.to_string();
        let tone = LogStatusTone::from_http_status(code);
        let entry = emit_hono_line(
            LogDirection::Out,
            LogChannel::Http,
            &subject,
            Some((&code_str, tone)),
            Some(&elapsed),
        );
        state.runtime.logs.push(entry);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn reuses_core_hono_latency_formatter() {
        assert_eq!(format_latency(Duration::from_micros(420)), "420µs");
        assert_eq!(format_latency(Duration::from_micros(1_450)), "1.4ms");
        assert_eq!(format_latency(Duration::from_millis(1_250)), "1.25s");
    }
}
