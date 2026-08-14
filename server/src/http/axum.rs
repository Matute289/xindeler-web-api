//! axum adapter for the framework-agnostic HTTP types.
//!
//! This is the only module in the crate that knows which HTTP framework is
//! serving requests.
//!
//! Handlers stay synchronous. The async surface is confined to this file:
//! each request is parsed here, handed to `spawn_blocking`, and the
//! synchronous handler runs there. That keeps blocking work (SQLite,
//! outbound HTTP to `xindeler-auth`) off the async reactor without needing
//! `async fn` handlers anywhere else in the crate.

use super::{BodyError, Request, Response, MAX_REQUEST_SIZE};
use ::axum::extract::{ConnectInfo, State};
use ::axum::Router;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

/// How long in-flight requests get to finish after a shutdown signal.
///
/// Kept well under systemd's default TimeoutStopSec so a wedged handler
/// cannot stall the unit and get SIGKILLed at an arbitrary point instead.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

struct AppHandler<F> {
    handle: F,
    on_panic: fn() -> Response,
}

pub fn serve<F>(addr: SocketAddr, workers: usize, handler: F, on_panic: fn() -> Response) -> !
where
    F: Fn(&Request) -> Response + Send + Sync + 'static,
{
    // Two reactor threads are ample: they only parse requests and shuttle
    // them to the blocking pool, which is where all the real work happens.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(workers)
        .enable_io()
        .enable_time()
        .build()
        .expect("failed to build the async runtime");

    let state = Arc::new(AppHandler {
        handle: handler,
        on_panic,
    });

    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .unwrap_or_else(|err| panic!("failed to bind {addr}: {err}"));

        // A fallback-only router, deliberately: the hand-written match in
        // web.rs stays the router, so path/method dispatch is one thing to
        // reason about, not two.
        let app = Router::new()
            .fallback(dispatch::<F>)
            .with_state(state)
            .into_make_service_with_connect_info::<SocketAddr>();

        // The grace period starts when the signal arrives, not when the
        // server starts, so the timeout arm cannot fire during normal
        // operation.
        let (signalled, on_signal) = tokio::sync::oneshot::channel();
        let server = ::axum::serve(listener, app).with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = signalled.send(());
        });

        tokio::select! {
            result = server => {
                if let Err(err) = result {
                    log::error!("http server failed: {err}");
                }
            }
            _ = async move {
                let _ = on_signal.await;
                tokio::time::sleep(SHUTDOWN_GRACE).await;
            } => {
                log::warn!(
                    "in-flight requests did not finish within {SHUTDOWN_GRACE:?}; exiting anyway"
                );
            }
        }
    });

    // Blocking handlers may still be running; give them the same budget and
    // then abandon them rather than hanging on shutdown.
    runtime.shutdown_timeout(SHUTDOWN_GRACE);
    std::process::exit(0)
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut terminate =
            signal(SignalKind::terminate()).expect("failed to install the SIGTERM handler");
        let mut interrupt =
            signal(SignalKind::interrupt()).expect("failed to install the SIGINT handler");

        tokio::select! {
            _ = terminate.recv() => log::info!("received SIGTERM, shutting down"),
            _ = interrupt.recv() => log::info!("received SIGINT, shutting down"),
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        log::info!("received Ctrl-C, shutting down");
    }
}

async fn dispatch<F>(
    State(state): State<Arc<AppHandler<F>>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: ::axum::extract::Request,
) -> ::axum::response::Response
where
    F: Fn(&Request) -> Response + Send + Sync + 'static,
{
    let (parts, body) = request.into_parts();

    // path_and_query, not the whole URI: an absolute-form target keeps only
    // the origin-form part, which is what the router matches on.
    let url = parts
        .uri
        .path_and_query()
        .map(|target| target.as_str().to_owned())
        .unwrap_or_else(|| "/".to_owned());

    // Header values that are not valid UTF-8 are dropped, so they read as
    // absent. Unreachable behind nginx, and failing closed is the safe side.
    let headers = parts
        .headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_owned(), value.to_owned()))
        })
        .collect();

    let body = read_body(body).await;

    let request = Request {
        method: parts.method.as_str().to_owned(),
        url,
        headers,
        body,
        remote_addr: peer,
    };

    let on_panic = state.on_panic;
    match tokio::task::spawn_blocking(move || (state.handle)(&request)).await {
        Ok(response) => into_axum(response),
        Err(err) => {
            log::error!("request handler panicked: {err}");
            into_axum(on_panic())
        }
    }
}

async fn read_body(body: ::axum::body::Body) -> Result<Vec<u8>, BodyError> {
    // Reading one byte past the cap and comparing means a body of exactly
    // MAX_REQUEST_SIZE still passes and MAX_REQUEST_SIZE + 1 does not.
    let limit = (MAX_REQUEST_SIZE + 1) as usize;
    match ::axum::body::to_bytes(body, limit).await {
        Ok(bytes) if bytes.len() as u64 > MAX_REQUEST_SIZE => Err(BodyError::TooLarge),
        Ok(bytes) => Ok(bytes.to_vec()),
        Err(err) => {
            // to_bytes fails either because the cap was exceeded or because
            // the connection broke mid-body. Keep them apart so the first
            // stays a 413 and the second stays a 500.
            let exceeded_cap = std::error::Error::source(&err)
                .is_some_and(|source| source.is::<http_body_util::LengthLimitError>());
            if exceeded_cap {
                Err(BodyError::TooLarge)
            } else {
                Err(BodyError::Io(std::io::Error::other(err.to_string())))
            }
        }
    }
}

fn into_axum(response: Response) -> ::axum::response::Response {
    let mut builder = ::axum::http::Response::builder().status(response.status_code);
    for (name, value) in response.headers {
        builder = builder.header(name.as_ref(), value.as_ref());
    }

    builder
        .body(::axum::body::Body::from(response.data))
        .unwrap_or_else(|err| {
            log::error!("failed to build response: {err}");
            ::axum::http::Response::builder()
                .status(500)
                .body(::axum::body::Body::empty())
                .expect("a bare 500 is always constructible")
        })
}
