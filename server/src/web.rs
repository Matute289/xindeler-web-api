use crate::config::NetworkConfig;
use crate::error::{self, ApiError};
use crate::http::{serve, BodyError, Request, Response};
use crate::state::AppState;
use crate::waitlist;
use log::*;
use std::net::IpAddr;
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
        move |request| dispatch(request, &state, &config.network),
        panic_response,
    )
}

fn ping(_request: &Request) -> Response {
    Response::text("pong")
}

/// The real client IP behind nginx. Our nginx sets `X-Forwarded-For` to
/// `$remote_addr` (overwriting any client-supplied value), so trusting it
/// from a peer that's actually nginx (loopback by default) is safe — see
/// `NetworkConfig` in `config.rs`.
fn remote(request: &Request, network: &NetworkConfig) -> IpAddr {
    let peer = request.remote_addr().ip();
    if !network.trusts(peer) {
        return peer;
    }
    request
        .header("X-Forwarded-For")
        .and_then(|value| value.split(',').next())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(peer)
}

/// The body is already read by the HTTP adapter; this only surfaces a
/// failure at the point a handler asks for it.
fn take_request_data(request: &Request) -> Result<&[u8], ApiError> {
    request.body().map_err(|err| match err {
        BodyError::TooLarge => ApiError::RequestTooLarge,
        BodyError::Io(err) => {
            log::error!("Failed to read request: {err}");
            ApiError::Io(std::io::Error::new(err.kind(), err.to_string()))
        }
    })
}

fn dispatch(request: &Request, state: &AppState, network: &NetworkConfig) -> Response {
    let path = request_path(request.raw_url());
    let remote_ip = remote(request, network);
    debug!("[{remote_ip}] -> {path}");

    let response = match (request.method(), path) {
        // Preflight CORS — browsers send OPTIONS before any cross-origin POST with JSON.
        ("OPTIONS", _) => Response::empty_204(),
        ("GET", "/ping") => ping(request),
        ("GET", "/api/status") => waitlist::server_status(state).unwrap_or_else(error::response),
        ("GET", "/api/waitlist/count") => {
            waitlist::waitlist_count(state).unwrap_or_else(error::response)
        }
        ("POST", "/api/waitlist") => take_request_data(request)
            .and_then(|body| waitlist::join_waitlist(body, remote_ip, state))
            .unwrap_or_else(error::response),
        ("POST", "/api/contribute") => take_request_data(request)
            .and_then(|body| waitlist::join_contribute(body, remote_ip, state))
            .unwrap_or_else(error::response),
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
    use super::{cors_origin, dispatch, remote};
    use crate::config::AppConfig;
    use crate::http::Request;
    use crate::state::AppState;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn config() -> AppConfig {
        AppConfig::from_iter(Vec::<(&str, &str)>::new()).unwrap()
    }

    fn state(config: &AppConfig) -> AppState {
        AppState::from_config(config)
    }

    fn loopback_peer() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 12345)
    }

    #[test]
    fn ping_responds_pong() {
        let config = config();
        let request = Request::fake("GET", "/ping").with_peer(loopback_peer());
        let response = dispatch(&request, &state(&config), &config.network);
        assert_eq!(response.status_code, 200);
    }

    #[test]
    fn unknown_route_is_a_bare_404() {
        let config = config();
        let request = Request::fake("GET", "/nope").with_peer(loopback_peer());
        let response = dispatch(&request, &state(&config), &config.network);
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
        let config = config();
        let request = Request::fake("GET", "/ping").with_peer(loopback_peer());
        let response = dispatch(&request, &state(&config), &config.network);
        assert_eq!(response.header("Cache-Control"), Some("no-store"));
        assert_eq!(response.header("Referrer-Policy"), Some("no-referrer"));
        assert_eq!(response.header("Vary"), Some("Origin"));
    }

    #[test]
    fn trusted_proxy_forwarded_for_is_used() {
        let config = config();
        let request = Request::fake("GET", "/ping")
            .with_peer(loopback_peer())
            .with_header("X-Forwarded-For", "203.0.113.9, 10.0.0.1");
        assert_eq!(
            remote(&request, &config.network),
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9))
        );
    }

    #[test]
    fn untrusted_peer_cannot_spoof_forwarded_for() {
        let config = config();
        let untrusted_peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 50)), 12345);
        let request = Request::fake("GET", "/ping")
            .with_peer(untrusted_peer)
            .with_header("X-Forwarded-For", "1.2.3.4");
        assert_eq!(
            remote(&request, &config.network),
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 50))
        );
    }
}
