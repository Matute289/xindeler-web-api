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
    /// Username/password_prehash rejected by xindeler-auth, or an
    /// AuthToken it issued failed /verify. Same public code either way —
    /// never distinguish "wrong password" from "verify failed" in the
    /// response, that distinction is an internal implementation detail.
    InvalidCredentials,
    /// No session cookie, or it doesn't match a live row in `sessions`.
    Unauthorized,
    /// xindeler-auth didn't respond, or responded with something this
    /// service doesn't know how to interpret (not a client input problem).
    UpstreamAuthError,
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
            ApiError::InvalidCredentials | ApiError::Unauthorized => 401,
            ApiError::UpstreamAuthError => 502,
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
/// same split `xindeler-auth` uses in its `error.rs`. `code`/`message` are
/// `String`, not `&'static str`: `forwarded_response()` below needs to fill
/// them with codes read from xindeler-auth's own response body (TOTP
/// errors), not just this service's fixed catalog from `public_fields()`.
#[derive(Debug, Serialize)]
pub struct PublicErrorBody {
    pub code: String,
    pub message: String,
    pub request_id: String,
}

pub fn public_fields(error: &ApiError) -> (&'static str, &'static str) {
    match error {
        ApiError::InvalidRequest(_) => ("INVALID_REQUEST", "The request is invalid."),
        ApiError::RateLimit => ("RATE_LIMITED", "Too many requests. Try again later."),
        ApiError::RequestTooLarge => ("REQUEST_TOO_LARGE", "The request body is too large."),
        ApiError::InvalidCredentials => (
            "INVALID_CREDENTIALS",
            "The username or password is incorrect.",
        ),
        ApiError::Unauthorized => ("UNAUTHORIZED", "Not logged in."),
        ApiError::UpstreamAuthError => ("UPSTREAM_ERROR", "Authentication service unavailable."),
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
        code: code.to_owned(),
        message: message.to_owned(),
        request_id,
    })
    .with_status_code(error.status_code())
}

/// Forwards a `code`/`message` xindeler-auth itself produced (a TOTP-specific
/// rejection like `TOTP_INVALID_CODE`, or `EMAIL_VERIFICATION_REQUIRED`)
/// instead of collapsing it through the fixed catalog in `public_fields()`.
/// Used when the specific code matters to the frontend and this service has
/// nothing more specific to say than xindeler-auth already did.
pub fn forwarded_response(status: u16, code: String, message: String) -> Response {
    let request_id = hex::encode(rand::random::<[u8; 8]>());
    if status >= 500 {
        log::error!("request_id={request_id} request failed: forwarded {code} ({status})");
    } else {
        log::info!("request_id={request_id} request rejected code={code} (forwarded)");
    }
    Response::json(&PublicErrorBody {
        code,
        message,
        request_id,
    })
    .with_status_code(status)
}
