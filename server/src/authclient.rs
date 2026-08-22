//! Thin, hand-rolled HTTP client for `xindeler-auth`.
//!
//! Deliberately **not** `xindeler-authc`: its `sign_in()`/`register()`
//! helpers call `net_prehash()` on whatever they're given, and this service
//! always receives an *already* prehashed password from the frontend —
//! using those helpers would hash it a second time and break every login.
//! `xindeler-auth-common` supplies the wire types (zero risk of hand-typing
//! the JSON shape wrong); the HTTP calls themselves are ours.

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Deserialize;
use std::time::Duration;
use uuid::Uuid;
use xindeler_auth_common::{
    AuthToken, ChallengeId, ChangePasswordPayload, ChangeUsernamePayload,
    CharacterAccessTokenResponse, DeleteAccountPayload, EmailVerificationRequiredResponse,
    ForgotPasswordPayload, IssueCharacterAccessTokenPayload, ResetPasswordPayload, SignInPayload,
    SignInResponse, TotpBackupCodesResponse, TotpChallengeResponse, TotpCodePayload,
    TotpEnrollPayload, TotpEnrollResponse, TotpLoginPayload, UsernameAvailabilityResponse,
    ValidityCheckPayload, ValidityCheckResponse,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum AuthClientError {
    /// `sign_in()` specifically: xindeler-auth rejected the login with a
    /// `403 EMAIL_VERIFICATION_REQUIRED` — a legacy pre-2FA account that
    /// still needs to confirm an email before it can log in. Unlike other
    /// rejections, this one *is* forwarded to the client verbatim (session
    /// login is the only caller that unwraps this variant instead of
    /// collapsing it via `map_sign_in_error`), because the frontend's legacy
    /// recovery modal needs `completion_token`/`deadline` to keep working.
    EmailVerificationRequired(EmailVerificationRequiredResponse),
    /// `sign_in()` specifically: xindeler-auth rejected the login with a
    /// `423 ACCOUNT_LOGIN_LOCKED` (G-08 — three wrong passwords in a row).
    /// Forwarded verbatim like `EmailVerificationRequired` above, for the
    /// same reason: the frontend needs the exact `locked_until` timestamp to
    /// show a real countdown, not just a generic message.
    AccountLoginLocked {
        message: String,
        locked_until: i64,
    },
    /// xindeler-auth answered with a status outside 200-299. Carries its own
    /// `code`/`message` (parsed from the response body when it's shaped
    /// like `{code, message, request_id}` — every rejection from that
    /// service is) so a caller can forward TOTP-specific codes
    /// (`TOTP_INVALID_CODE`, `ACCOUNT_2FA_LOCKED`, ...) verbatim instead of
    /// collapsing everything to one generic error, the same reasoning
    /// `EmailVerificationRequired` already established. `code`/`message`
    /// are empty strings if the body didn't parse — callers that don't care
    /// about the specific code just match on `status`.
    RejectedWithBody {
        status: u16,
        code: String,
        message: String,
    },
    Request(reqwest::Error),
    /// `verify()` was called but this service has no `AUTH_SERVICE_TOKEN`
    /// configured — a deployment problem, not a client input problem.
    MissingServiceToken,
    /// `issue_character_access_token()` was called but this service has no
    /// `WEB_API_SERVICE_TOKEN` configured — the *new*, distinct credential
    /// (Fase F), never `service_token`/`MissingServiceToken` above.
    MissingCharacterServiceToken,
}

/// What a 2FA-aware login resolves to: either xindeler-auth had no TOTP to
/// check and handed back a usable token directly, or the account has a
/// confirmed TOTP enrollment and login must pause for `/login/2fa` before a
/// session gets created — hallazgo 2 of backlog 007, now that Fase L exists.
pub enum SignInOutcome {
    Token(AuthToken),
    TotpChallenge {
        challenge_id: ChallengeId,
        expires_in: u64,
    },
}

/// xindeler-auth's own error envelope, `{code, message, request_id}`
/// (`server/src/error.rs::PublicErrorBody` there) — redeclared locally
/// because that type isn't part of `xindeler-auth-common` (it's private to
/// that repo's server crate). `request_id` is dropped: xindeler-web-api
/// stamps its own on every response it sends, so xindeler-auth's would be
/// noise here, not something a caller could correlate against anything.
#[derive(Deserialize)]
struct RemoteErrorBody {
    code: String,
    message: String,
}

/// xindeler-auth's `AccountLoginLockedResponse` (G-08), redeclared locally —
/// same reasoning as `RemoteErrorBody` above: that type is `Serialize`-only
/// in `xindeler-auth-common` (it's built server-side there, never consumed),
/// so this service needs its own `Deserialize` shape to read `locked_until`
/// back off the wire.
#[derive(Deserialize)]
struct AccountLoginLockedBody {
    code: String,
    message: String,
    locked_until: i64,
}

/// Codes callers should forward verbatim (via `error::forwarded_response`)
/// instead of collapsing through their usual generic status-based error
/// mapping — the frontend needs to tell these apart from a plain
/// wrong-password rejection to show the right copy (a code entry field, a
/// "locked, contact support" message, a cooldown notice, etc.), same
/// reasoning already applied to `EMAIL_VERIFICATION_REQUIRED`.
///
/// Two families share this list on purpose: the TOTP ones (any of the
/// `/2fa/*`/`/login/2fa` calls, plus the step-up on `change_username`/
/// `delete_account`) and `change_username`'s own non-step-up rejections —
/// `change_username` can fail with `400` for four *semantically different*
/// reasons (wrong password, name taken, name reserved, changed too
/// recently — confirmed reading `auth::change_username` in xindeler-auth,
/// not assumed), and only the first of those is genuinely
/// `INVALID_CREDENTIALS`.
///
/// `ACCOUNT_NOT_ACTIVE` (G-08's permanent-lockout tier, or any other
/// moderation ban) joins this list too: `sign_in()`'s `403` handling below
/// tags it with this code specifically so it doesn't collapse into the same
/// generic invalid-login copy as a plain wrong password.
///
/// `INVALID_EMAIL`, `INVALID_TOKEN` and `ACCOUNT_EXPIRED` join this list for
/// the register/recovery proxies (`register`, `verify_email`,
/// `set_account_email`, `resend_verification`): `AuthModal`'s
/// `legacyErrorMessage()` already branches on all three today against
/// xindeler-auth's direct responses, and needs the same codes once those
/// calls go through this proxy instead of collapsing to a generic message.
pub fn should_forward_verbatim(code: &str) -> bool {
    matches!(
        code,
        "TOTP_INVALID_CODE"
            | "TOTP_NOT_ENROLLED"
            | "TOTP_ALREADY_ENROLLED"
            | "TOTP_ALREADY_CONFIRMED"
            | "TOTP_CHALLENGE_INVALID"
            | "ACCOUNT_2FA_LOCKED"
            | "USERNAME_UNAVAILABLE"
            | "USERNAME_RESERVED"
            | "USERNAME_CHANGE_COOLDOWN"
            | "ACCOUNT_NOT_ACTIVE"
            | "INVALID_EMAIL"
            | "INVALID_TOKEN"
            | "ACCOUNT_EXPIRED"
    )
}

impl From<reqwest::Error> for AuthClientError {
    fn from(err: reqwest::Error) -> Self {
        AuthClientError::Request(err)
    }
}

/// Parses a rejected response's body as xindeler-auth's error envelope. A
/// body that doesn't match (a legacy plain-text response, an unrelated
/// proxy error) falls back to empty `code`/`message` rather than failing —
/// `status` alone is still enough for a caller that only checks status.
fn rejection_with_body(status: u16, response: reqwest::blocking::Response) -> AuthClientError {
    match response.json::<RemoteErrorBody>() {
        Ok(body) => AuthClientError::RejectedWithBody {
            status,
            code: body.code,
            message: body.message,
        },
        Err(_) => AuthClientError::RejectedWithBody {
            status,
            code: String::new(),
            message: String::new(),
        },
    }
}

pub struct AuthClient {
    client: reqwest::blocking::Client,
    base_url: String,
    service_token: Option<String>,
    character_service_token: Option<String>,
}

impl AuthClient {
    pub fn new(
        base_url: &str,
        service_token: Option<&str>,
        character_service_token: Option<&str>,
    ) -> Self {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client config is always valid");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
            service_token: service_token.map(str::to_owned),
            character_service_token: character_service_token.map(str::to_owned),
        }
    }

    /// `password_prehash` must already be the client-side prehash — never a
    /// raw password (see module docs).
    pub fn sign_in(
        &self,
        username: &str,
        password_prehash: &str,
    ) -> Result<SignInOutcome, AuthClientError> {
        let payload = SignInPayload {
            username: username.to_owned(),
            password: password_prehash.to_owned(),
        };
        let response = self
            .client
            .post(format!("{}/generate_token", self.base_url))
            .json(&payload)
            .send()?;
        let status = response.status();
        if status.as_u16() == 202 {
            let challenge: TotpChallengeResponse = response.json()?;
            return Ok(SignInOutcome::TotpChallenge {
                challenge_id: challenge.challenge_id,
                expires_in: challenge.expires_in,
            });
        }
        if !status.is_success() {
            // 423 is G-08's temporary lockout (three wrong passwords in a
            // row) — the only status that carries `locked_until`, so it gets
            // its own typed parse same as the 403 case below.
            if status.as_u16() == 423 {
                if let Ok(body) = response.json::<AccountLoginLockedBody>() {
                    if body.code == "ACCOUNT_LOGIN_LOCKED" {
                        return Err(AuthClientError::AccountLoginLocked {
                            message: body.message,
                            locked_until: body.locked_until,
                        });
                    }
                }
                return Err(AuthClientError::RejectedWithBody {
                    status: 423,
                    code: String::new(),
                    message: String::new(),
                });
            }
            // 403 is worth inspecting before falling back to the generic
            // path: xindeler-auth uses it both for EMAIL_VERIFICATION_REQUIRED
            // (needs its own typed shape, deadline/completion_token) and for
            // ACCOUNT_NOT_ACTIVE (G-08's permanent-lockout tier, or any other
            // moderation ban — just needs its code forwarded so the frontend
            // doesn't show the same copy as a wrong password). Read the body
            // once as bytes since it can only be consumed a single time, then
            // try each shape against that buffer. Any other shape (or a body
            // that fails to parse at all) falls through to the generic case.
            if status.as_u16() == 403 {
                let bytes = response.bytes().unwrap_or_default();
                if let Ok(body) =
                    serde_json::from_slice::<EmailVerificationRequiredResponse>(&bytes)
                {
                    if body.code == "EMAIL_VERIFICATION_REQUIRED" {
                        return Err(AuthClientError::EmailVerificationRequired(body));
                    }
                }
                if let Ok(body) = serde_json::from_slice::<RemoteErrorBody>(&bytes) {
                    if body.code == "ACCOUNT_NOT_ACTIVE" {
                        return Err(AuthClientError::RejectedWithBody {
                            status: 403,
                            code: body.code,
                            message: body.message,
                        });
                    }
                }
                return Err(AuthClientError::RejectedWithBody {
                    status: 403,
                    code: String::new(),
                    message: String::new(),
                });
            }
            return Err(rejection_with_body(status.as_u16(), response));
        }
        Ok(SignInOutcome::Token(
            response.json::<SignInResponse>()?.token,
        ))
    }

    /// Redeems a `/generate_token` challenge (`SignInOutcome::TotpChallenge`)
    /// for the real `AuthToken` — the second half of a 2FA login. Same
    /// caller-side flow as `sign_in` from here: `verify()` it to learn the
    /// `uuid`/`username`, then create the session.
    pub fn totp_login(
        &self,
        challenge_id: ChallengeId,
        code: &str,
    ) -> Result<AuthToken, AuthClientError> {
        let payload = TotpLoginPayload {
            challenge_id,
            code: code.to_owned(),
        };
        let response = self
            .client
            .post(format!("{}/login/2fa", self.base_url))
            .json(&payload)
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejection_with_body(status.as_u16(), response));
        }
        Ok(response.json::<SignInResponse>()?.token)
    }

    /// Resolves a fresh `AuthToken` (15s TTL, single-use — consumed by this
    /// call) into the uuid/username that own it. Server-to-server only:
    /// requires the shared service token, same credential the game server
    /// already uses against this same endpoint.
    pub fn verify(&self, token: AuthToken) -> Result<ValidityCheckResponse, AuthClientError> {
        let service_token = self
            .service_token
            .as_deref()
            .ok_or(AuthClientError::MissingServiceToken)?;
        let payload = ValidityCheckPayload { token };
        let response = self
            .client
            .post(format!("{}/verify", self.base_url))
            .bearer_auth(service_token)
            .json(&payload)
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejection_with_body(status.as_u16(), response));
        }
        Ok(response.json()?)
    }

    /// Fase F: mints a fresh `CharacterAccessToken` for `uuid`, to hand off
    /// to `game_server_client::GameServerClient` immediately afterward.
    /// Deliberately never cached across calls — 60s TTL, one-shot, minted
    /// fresh per character-list-read or per-rename action, same as
    /// xindeler-auth's own design intends (N-01 spec's "Redemption"
    /// section). Uses `character_service_token`
    /// (`WEB_API_SERVICE_TOKEN`) — never `service_token`, which
    /// xindeler-auth explicitly refuses for this endpoint.
    pub fn issue_character_access_token(
        &self,
        uuid: Uuid,
    ) -> Result<xindeler_auth_common::CharacterAccessToken, AuthClientError> {
        let character_service_token = self
            .character_service_token
            .as_deref()
            .ok_or(AuthClientError::MissingCharacterServiceToken)?;
        let payload = IssueCharacterAccessTokenPayload { uuid };
        let response = self
            .client
            .post(format!("{}/issue-character-access-token", self.base_url))
            .bearer_auth(character_service_token)
            .json(&payload)
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejection_with_body(status.as_u16(), response));
        }
        Ok(response.json::<CharacterAccessTokenResponse>()?.token)
    }

    // The methods below are public, unauthenticated endpoints on
    // xindeler-auth's side (no service token — same rate-limited-by-IP tier
    // as `/generate_token`/`/register`). `password_prehash` fields must
    // already be prehashed, same as `sign_in`.

    pub fn check_username(&self, username: &str) -> Result<bool, AuthClientError> {
        let encoded = utf8_percent_encode(username, NON_ALPHANUMERIC);
        let response = self
            .client
            .get(format!(
                "{}/check-username?username={encoded}",
                self.base_url
            ))
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejection_with_body(status.as_u16(), response));
        }
        Ok(response.json::<UsernameAvailabilityResponse>()?.available)
    }

    /// `code` is the TOTP code, required only when the account has a
    /// confirmed enrollment — xindeler-auth's own
    /// `require_step_up_if_confirmed` treats a missing code as a no-op when
    /// there's nothing to step up against, so it's safe to always pass
    /// through whatever the caller gave (including `None`).
    pub fn change_username(
        &self,
        old_username: &str,
        password_prehash: &str,
        new_username: &str,
        code: Option<&str>,
    ) -> Result<(), AuthClientError> {
        let payload = ChangeUsernamePayload {
            old_username: old_username.to_owned(),
            password: password_prehash.to_owned(),
            new_username: new_username.to_owned(),
            code: code.map(str::to_owned),
        };
        let response = self
            .client
            .post(format!("{}/change_username", self.base_url))
            .json(&payload)
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejection_with_body(status.as_u16(), response));
        }
        Ok(())
    }

    /// xindeler-auth's `ChangePasswordPayload` has no `code` field —
    /// changing a password isn't currently step-up gated the way
    /// `change_username`/`delete_account` are (confirmed reading
    /// `xindeler-auth`'s source, not assumed).
    pub fn change_password(
        &self,
        username: &str,
        current_password_prehash: &str,
        new_password_prehash: &str,
    ) -> Result<(), AuthClientError> {
        let payload = ChangePasswordPayload {
            username: username.to_owned(),
            current_password: current_password_prehash.to_owned(),
            new_password: new_password_prehash.to_owned(),
        };
        let response = self
            .client
            .post(format!("{}/change_password", self.base_url))
            .json(&payload)
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejection_with_body(status.as_u16(), response));
        }
        Ok(())
    }

    pub fn delete_account(
        &self,
        username: &str,
        password_prehash: &str,
        code: Option<&str>,
    ) -> Result<(), AuthClientError> {
        let payload = DeleteAccountPayload {
            username: username.to_owned(),
            password: password_prehash.to_owned(),
            code: code.map(str::to_owned),
        };
        let response = self
            .client
            .post(format!("{}/delete_account", self.base_url))
            .json(&payload)
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejection_with_body(status.as_u16(), response));
        }
        Ok(())
    }

    /// Always answers 200 on xindeler-auth's side, whether or not the email
    /// exists — anti-enumeration, same as `/api/waitlist`'s dedup response.
    /// A non-2xx here means the request itself was malformed, never "no
    /// such account".
    pub fn forgot_password(&self, email: &str) -> Result<(), AuthClientError> {
        let payload = ForgotPasswordPayload {
            email: email.to_owned(),
        };
        let response = self
            .client
            .post(format!("{}/forgot-password", self.base_url))
            .json(&payload)
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejection_with_body(status.as_u16(), response));
        }
        Ok(())
    }

    pub fn reset_password(
        &self,
        token: &str,
        new_password_prehash: &str,
    ) -> Result<(), AuthClientError> {
        let payload = ResetPasswordPayload {
            token: token.to_owned(),
            new_password: new_password_prehash.to_owned(),
        };
        let response = self
            .client
            .post(format!("{}/reset-password", self.base_url))
            .json(&payload)
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejection_with_body(status.as_u16(), response));
        }
        Ok(())
    }

    // The five methods below are Fase L (2FA/TOTP) of xindeler-auth — see
    // `totp_login` above for the sixth (login-side) one. All require
    // `username`+`password` explicitly in the body, same as every other
    // mutable xindeler-auth endpoint: there's no bearer/session token to
    // reuse across these calls.

    pub fn totp_enroll(
        &self,
        username: &str,
        password_prehash: &str,
    ) -> Result<TotpEnrollResponse, AuthClientError> {
        let payload = TotpEnrollPayload {
            username: username.to_owned(),
            password: password_prehash.to_owned(),
        };
        let response = self
            .client
            .post(format!("{}/2fa/enroll", self.base_url))
            .json(&payload)
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejection_with_body(status.as_u16(), response));
        }
        Ok(response.json()?)
    }

    pub fn totp_confirm(
        &self,
        username: &str,
        password_prehash: &str,
        code: &str,
    ) -> Result<Vec<String>, AuthClientError> {
        let payload = TotpCodePayload {
            username: username.to_owned(),
            password: password_prehash.to_owned(),
            code: code.to_owned(),
        };
        let response = self
            .client
            .post(format!("{}/2fa/confirm", self.base_url))
            .json(&payload)
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejection_with_body(status.as_u16(), response));
        }
        Ok(response.json::<TotpBackupCodesResponse>()?.backup_codes)
    }

    pub fn totp_disable(
        &self,
        username: &str,
        password_prehash: &str,
        code: &str,
    ) -> Result<(), AuthClientError> {
        let payload = TotpCodePayload {
            username: username.to_owned(),
            password: password_prehash.to_owned(),
            code: code.to_owned(),
        };
        let response = self
            .client
            .post(format!("{}/2fa/disable", self.base_url))
            .json(&payload)
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejection_with_body(status.as_u16(), response));
        }
        Ok(())
    }

    pub fn totp_regenerate_backup_codes(
        &self,
        username: &str,
        password_prehash: &str,
        code: &str,
    ) -> Result<Vec<String>, AuthClientError> {
        let payload = TotpCodePayload {
            username: username.to_owned(),
            password: password_prehash.to_owned(),
            code: code.to_owned(),
        };
        let response = self
            .client
            .post(format!("{}/2fa/backup-codes/regenerate", self.base_url))
            .json(&payload)
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejection_with_body(status.as_u16(), response));
        }
        Ok(response.json::<TotpBackupCodesResponse>()?.backup_codes)
    }
}
