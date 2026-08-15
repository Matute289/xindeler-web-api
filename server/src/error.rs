use crate::http::Response;
use serde::Serialize;

/// Internal error vocabulary.
///
/// The inner values (`Db`, `Migration`, `InvalidRequest`, `Io`) are read only
/// through the `{error:?}` Debug format in `error::response()`'s log line —
/// dead-code analysis doesn't count that as a "real" read, hence the allow.
#[derive(Debug)]
#[allow(dead_code)]
pub enum ApiError {
    InternalServerError,
    Db(rusqlite::Error),
    Migration(refinery::Error),
    /// A request field failed validation. Maps to 422, matching the Pydantic
    /// behavior of the Python service this replaces — the frontend already
    /// expects 422 for bad input, not 400.
    InvalidRequest(String),
    RateLimit,
    RequestTooLarge,
    Io(std::io::Error),
}

impl ApiError {
    pub fn status_code(&self) -> u16 {
        match self {
            ApiError::InternalServerError
            | ApiError::Db(_)
            | ApiError::Migration(_)
            | ApiError::Io(_) => 500,
            ApiError::InvalidRequest(_) => 422,
            ApiError::RateLimit => 429,
            ApiError::RequestTooLarge => 413,
        }
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(error: rusqlite::Error) -> Self {
        ApiError::Db(error)
    }
}

impl From<refinery::Error> for ApiError {
    fn from(error: refinery::Error) -> Self {
        ApiError::Migration(error)
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        ApiError::InvalidRequest(error.to_string())
    }
}

/// What a client is allowed to see. Kept separate from `ApiError` so an
/// internal detail (a DB error, a panic) never leaks into a response body —
/// same split `xindeler-auth` uses in its `error.rs`.
#[derive(Debug, Serialize)]
pub struct PublicErrorBody {
    pub code: &'static str,
    pub message: &'static str,
    pub request_id: String,
}

pub fn public_fields(error: &ApiError) -> (&'static str, &'static str) {
    match error {
        ApiError::InvalidRequest(_) => ("INVALID_REQUEST", "The request is invalid."),
        ApiError::RateLimit => ("RATE_LIMITED", "Too many requests. Try again later."),
        ApiError::RequestTooLarge => ("REQUEST_TOO_LARGE", "The request body is too large."),
        ApiError::InternalServerError
        | ApiError::Db(_)
        | ApiError::Migration(_)
        | ApiError::Io(_) => ("INTERNAL_ERROR", "Internal server error."),
    }
}

pub fn response(error: ApiError) -> Response {
    let request_id = hex::encode(rand::random::<[u8; 8]>());
    let (code, message) = public_fields(&error);
    if error.status_code() >= 500 {
        log::error!("request_id={request_id} request failed: {error:?}");
    } else {
        log::info!("request_id={request_id} request rejected code={code}");
    }
    // Cache-Control/Pragma/Referrer-Policy are applied uniformly to every
    // response (including this one) by web::finalize().
    Response::json(&PublicErrorBody {
        code,
        message,
        request_id,
    })
    .with_status_code(error.status_code())
}
