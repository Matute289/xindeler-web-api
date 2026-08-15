use crate::authclient::{is_totp_specific, AuthClientError, SignInOutcome};
use crate::db::db;
use crate::error::{self, ApiError};
use crate::http::{Request, Response};
use crate::state::AppState;
use crate::totp_status;
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use xindeler_auth_common::AuthToken;
use xindeler_web_api_common::{
    LoginPayload, LoginTotpChallengeResponse, LoginTotpRequest, MeResponse, OkResponse,
};

const SESSION_COOKIE: &str = "session";

/// 7 days, absolute — no sliding renewal. A player who wants to stay logged
/// in longer just logs in again; this bounds how long a stolen cookie stays
/// useful without needing a revocation list beyond `sessions.revoked_at`.
/// Decisión #3 of backlog 007, proposed and not objected to.
const SESSION_TTL_SECS: i64 = 7 * 24 * 3600;

/// The identity behind a live session cookie — `uuid` is the stable key
/// (used to revoke every session for an account), `username` is a cached
/// display value that goes stale the instant `change_username` succeeds,
/// which is exactly why every mutating account endpoint revokes and forces
/// a relogin instead of patching it in place.
pub struct SessionIdentity {
    pub uuid: String,
    pub username: String,
}

fn hash_token(raw: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(raw.as_bytes());
    hex::encode(digest.finalize())
}

fn generate_session_token() -> String {
    hex::encode(rand::random::<[u8; 32]>())
}

/// `max_age_secs` of `0` clears the cookie (logout, or any mutating account
/// action that revokes the session).
fn set_cookie(response: Response, raw_token: &str, max_age_secs: i64) -> Response {
    response.with_unique_header(
        "Set-Cookie",
        format!(
            "{SESSION_COOKIE}={raw_token}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={max_age_secs}"
        ),
    )
}

pub fn clear_cookie(response: Response) -> Response {
    set_cookie(response, "", 0)
}

/// Resolves the session cookie into the account it belongs to. Every
/// endpoint that requires "logged in" starts here.
pub fn resolve_session(request: &Request) -> Result<SessionIdentity, ApiError> {
    let raw = request
        .cookie(SESSION_COOKIE)
        .ok_or(ApiError::Unauthorized)?;
    let session_id = hash_token(raw);
    let now = Utc::now().timestamp();

    db()?
        .query_row(
            "SELECT uuid, username FROM sessions \
             WHERE session_id = ?1 AND revoked_at IS NULL AND expires_at > ?2",
            params![session_id, now],
            |row| {
                Ok(SessionIdentity {
                    uuid: row.get(0)?,
                    username: row.get(1)?,
                })
            },
        )
        .optional()?
        .ok_or(ApiError::Unauthorized)
}

/// Revokes every live session for an account — used by `change_username`,
/// `change_password` and `delete_account` in the same request that confirms
/// the change (hallazgo 8 of backlog 007), never a background job.
pub fn revoke_all_sessions(uuid: &str) -> Result<(), ApiError> {
    let now = Utc::now().timestamp();
    db()?.execute(
        "UPDATE sessions SET revoked_at = ?1 WHERE uuid = ?2 AND revoked_at IS NULL",
        params![now, uuid],
    )?;
    Ok(())
}

fn map_sign_in_error(err: AuthClientError) -> ApiError {
    match err {
        // xindeler-auth answers InvalidLogin (bad username/password) and
        // InvalidToken (bad/expired/already-consumed AuthToken, the error
        // verify() would see) with 400, not 401 — confirmed against the
        // real, deployed service during the B-05 smoke test, not assumed
        // from the wire-type names. 403 stays here too, as the fallback for
        // an EMAIL_VERIFICATION_REQUIRED-shaped 403 whose body didn't parse
        // (login() intercepts the well-formed case before this function
        // ever runs — see AuthClientError::EmailVerificationRequired).
        AuthClientError::RejectedWithBody { status, .. }
            if status == 400 || status == 401 || status == 403 =>
        {
            ApiError::InvalidCredentials
        }
        AuthClientError::RejectedWithBody { status: 429, .. } => ApiError::RateLimit,
        AuthClientError::RejectedWithBody { status, code, .. } => {
            log::warn!("xindeler-auth answered with unexpected status {status} (code={code})");
            ApiError::UpstreamAuthError
        }
        AuthClientError::Request(err) => {
            log::warn!("request to xindeler-auth failed: {err}");
            ApiError::UpstreamAuthError
        }
        AuthClientError::MissingServiceToken => {
            log::error!("AUTH_SERVICE_TOKEN is not configured — session login cannot work");
            ApiError::InternalServerError
        }
        // login() unwraps this variant before ever calling map_sign_in_error
        // (see above); verify() never produces it (different endpoint). Kept
        // here only so the match stays exhaustive if a future caller forgets
        // to special-case it — falls back to the same generic 401 as any
        // other rejected credential.
        AuthClientError::EmailVerificationRequired(_) => ApiError::InvalidCredentials,
    }
}

/// Shared tail end of both login paths (direct token, or after redeeming a
/// 2FA challenge): resolve the token's identity, create the session row,
/// record the now-known TOTP status, set the cookie. `totp_confirmed` is
/// `false` for a direct login (xindeler-auth would have issued a challenge
/// instead if the account had TOTP confirmed) and `true` after `/login/2fa`
/// succeeds — either way it's a fact this call already knows, not a guess.
fn create_session(
    auth_token: AuthToken,
    fallback_username: String,
    totp_confirmed: bool,
    state: &AppState,
) -> Result<Response, ApiError> {
    let verified = state
        .auth_client
        .verify(auth_token)
        .map_err(map_sign_in_error)?;
    let username = verified.username.unwrap_or(fallback_username);
    let uuid = verified.uuid.to_string();

    let raw_session = generate_session_token();
    let session_id = hash_token(&raw_session);
    let now = Utc::now().timestamp();

    db()?.execute(
        "INSERT INTO sessions (session_id, uuid, username, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![session_id, uuid, username, now, now + SESSION_TTL_SECS],
    )?;

    if totp_confirmed {
        totp_status::mark_confirmed(&uuid)?;
    } else {
        totp_status::mark_disabled(&uuid)?;
    }

    let response = Response::json(&MeResponse {
        username,
        totp_enabled: totp_confirmed,
    });
    Ok(set_cookie(response, &raw_session, SESSION_TTL_SECS))
}

pub fn login(body: &[u8], remote_ip: IpAddr, state: &AppState) -> Result<Response, ApiError> {
    let payload: LoginPayload = serde_json::from_slice(body)?;
    if payload.username.trim().is_empty() || payload.password_prehash.trim().is_empty() {
        return Err(ApiError::InvalidRequest(
            "username and password_prehash are required".into(),
        ));
    }

    if !state.login_requests.check(remote_ip) {
        return Err(ApiError::RateLimit);
    }

    let outcome = match state
        .auth_client
        .sign_in(&payload.username, &payload.password_prehash)
    {
        Ok(outcome) => outcome,
        // Forwarded verbatim, not through the generic error envelope — the
        // frontend's legacy account-recovery modal needs the exact
        // completion_token/deadline shape xindeler-auth already returns.
        Err(AuthClientError::EmailVerificationRequired(body)) => {
            return Ok(Response::json(&body).with_status_code(403));
        }
        Err(err) => return Err(map_sign_in_error(err)),
    };

    match outcome {
        SignInOutcome::Token(token) => create_session(token, payload.username, false, state),
        // The account has a confirmed TOTP enrollment — hallazgo 2 of
        // backlog 007: no session gets created until the second factor is
        // confirmed via login_2fa() below, or a present-but-2FA-off cookie
        // would itself leak that the account has no 2FA.
        SignInOutcome::TotpChallenge {
            challenge_id,
            expires_in,
        } => Ok(Response::json(&LoginTotpChallengeResponse {
            challenge_id: challenge_id.serialize(),
            expires_in,
        })
        .with_status_code(202)),
    }
}

pub fn login_2fa(body: &[u8], state: &AppState) -> Result<Response, ApiError> {
    let payload: LoginTotpRequest = serde_json::from_slice(body)?;
    if payload.challenge_id.trim().is_empty() || payload.code.trim().is_empty() {
        return Err(ApiError::InvalidRequest(
            "challenge_id and code are required".into(),
        ));
    }
    let challenge_id = payload
        .challenge_id
        .parse()
        .map_err(|_| ApiError::InvalidRequest("challenge_id is malformed".into()))?;

    let auth_token = match state.auth_client.totp_login(challenge_id, &payload.code) {
        Ok(token) => token,
        Err(AuthClientError::RejectedWithBody {
            status,
            code,
            message,
        }) if is_totp_specific(&code) => {
            return Ok(error::forwarded_response(status, code, message));
        }
        Err(err) => return Err(map_sign_in_error(err)),
    };

    // Unlike login(), this request carries no client-supplied username to
    // fall back on if verify() omits it (see `ValidityCheckResponse`'s own
    // doc comment on why that field is optional) — in the extremely
    // unlikely case it's missing, create_session() falls back to an empty
    // string rather than failing the whole login.
    create_session(auth_token, String::new(), true, state)
}

pub fn me(request: &Request) -> Result<Response, ApiError> {
    let identity = resolve_session(request)?;
    let totp_enabled = totp_status::is_enabled(&identity.uuid)?;
    Ok(Response::json(&MeResponse {
        username: identity.username,
        totp_enabled,
    }))
}

pub fn logout(request: &Request) -> Result<Response, ApiError> {
    if let Some(raw) = request.cookie(SESSION_COOKIE) {
        let session_id = hash_token(raw);
        db()?.execute(
            "DELETE FROM sessions WHERE session_id = ?1",
            params![session_id],
        )?;
    }
    // 200 regardless of whether a session existed — logging out twice, or
    // logging out with no cookie at all, is not an error from the caller's
    // point of view.
    Ok(clear_cookie(Response::json(&OkResponse { ok: true })))
}

#[cfg(test)]
mod tests {
    use super::{generate_session_token, hash_token};

    #[test]
    fn session_tokens_are_unique_and_hash_deterministically() {
        let a = generate_session_token();
        let b = generate_session_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 64); // 32 bytes hex-encoded
        assert_eq!(hash_token(&a), hash_token(&a));
        assert_ne!(hash_token(&a), hash_token(&b));
    }
}
