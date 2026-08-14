use crate::error::{self, ApiError};
use crate::http::{serve, Request, Response};
use crate::state::AppState;
use log::*;
use std::sync::Arc;

/// Real origins this service accepts cross-origin requests from — same list
/// as the production CORS config of the FastAPI service this replaces
/// (`ALLOWED_ORIGINS` in `main.py`), plus the two local dev ports the
/// frontend already uses when talking to `xindeler-auth`.
fn cors_origin(request: &Request) -> Option<&'static str> {
    match request.header("Origin")? {
        "https://xindeler.com" => Some("https://xindeler.com"),
        "https://www.xindeler.com" => Some("https://www.xindeler.com"),
        "http://localhost:5173" => Some("http://localhost:5173"),
        "http://127.0.0.1:5173" => Some("http://127.0.0.1:5173"),
        _ => None,
    }
}

pub fn start() {
    let config = crate::config::get();
    let state = Arc::new(AppState::from_config(config));
    let addr = config.bind_addr;
    let worker_count = config.http_workers;
    info!("Starting webserver on {addr}");

    serve(
        addr,
        worker_count,
        move |request| dispatch(request, &state),
        panic_response,
    )
}

fn ping(_request: &Request) -> Response {
    Response::text("pong")
}

fn dispatch(request: &Request, _state: &AppState) -> Response {
    let path = request_path(request.raw_url());
    debug!("[{}] -> {}", request.remote_addr(), path);

    let response = match (request.method(), path) {
        // Preflight CORS — browsers send OPTIONS before any cross-origin POST with JSON.
        ("OPTIONS", _) => Response::empty_204(),
        ("GET", "/ping") => ping(request),
        // Fase 1 adds: GET /api/status, GET /api/waitlist/count,
        // POST /api/waitlist, POST /api/contribute.
        // Fase 2 adds: POST /api/session/*, GET /api/session/me,
        // POST /api/account/*.
        _ => Response::empty_404(),
    };

    finalize(response, cors_origin(request))
}

fn request_path(raw_url: &str) -> &str {
    raw_url.split('?').next().unwrap_or("/")
}

fn privacy_headers(response: Response) -> Response {
    response
        .with_unique_header("Cache-Control", "no-store")
        .with_unique_header("Pragma", "no-cache")
        .with_unique_header("Referrer-Policy", "no-referrer")
}

/// Applies the headers that every response carries, whatever its status.
fn finalize(response: Response, allowed_origin: Option<&'static str>) -> Response {
    let response = privacy_headers(response)
        .with_unique_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        .with_unique_header(
            "Access-Control-Allow-Headers",
            "Content-Type, Authorization",
        )
        .with_unique_header("Vary", "Origin");

    match allowed_origin {
        Some(origin) => response.with_unique_header("Access-Control-Allow-Origin", origin),
        None => response,
    }
}

/// Response used when a handler panics — routed through the same envelope
/// and headers as every other response, instead of a bare framework 500.
fn panic_response() -> Response {
    finalize(error::response(ApiError::InternalServerError), None)
}

#[cfg(test)]
mod tests {
    use super::{cors_origin, dispatch};
    use crate::http::Request;
    use crate::state::AppState;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn state() -> AppState {
        AppState::from_config(
            &crate::config::AppConfig::from_iter(Vec::<(&str, &str)>::new()).unwrap(),
        )
    }

    fn loopback_peer() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 12345)
    }

    #[test]
    fn ping_responds_pong() {
        let request = Request::fake("GET", "/ping").with_peer(loopback_peer());
        let response = dispatch(&request, &state());
        assert_eq!(response.status_code, 200);
    }

    #[test]
    fn unknown_route_is_a_bare_404() {
        let request = Request::fake("GET", "/nope").with_peer(loopback_peer());
        let response = dispatch(&request, &state());
        assert_eq!(response.status_code, 404);
    }

    #[test]
    fn cors_origin_only_allows_known_origins() {
        let allowed = Request::fake("GET", "/ping").with_header("Origin", "https://xindeler.com");
        assert_eq!(cors_origin(&allowed), Some("https://xindeler.com"));

        let unknown =
            Request::fake("GET", "/ping").with_header("Origin", "https://evil.example.com");
        assert_eq!(cors_origin(&unknown), None);
    }

    #[test]
    fn every_response_carries_privacy_and_cors_headers() {
        let request = Request::fake("GET", "/ping").with_peer(loopback_peer());
        let response = dispatch(&request, &state());
        assert_eq!(response.header("Cache-Control"), Some("no-store"));
        assert_eq!(response.header("Referrer-Policy"), Some("no-referrer"));
        assert_eq!(response.header("Vary"), Some("Origin"));
    }
}
