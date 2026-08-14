use crate::http::Response;
use serde::Serialize;

/// Internal error vocabulary. Fase 1/2 extend this (validation, rate limit,
/// upstream-auth failures) — kept deliberately small in Fase 0, where the
/// only handler is `/ping`.
#[derive(Debug)]
pub enum ApiError {
    InternalServerError,
}

impl ApiError {
    pub fn status_code(&self) -> u16 {
        match self {
            ApiError::InternalServerError => 500,
        }
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
        ApiError::InternalServerError => ("INTERNAL_ERROR", "Internal server error."),
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
