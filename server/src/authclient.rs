//! Thin, hand-rolled HTTP client for the two `xindeler-auth` endpoints this
//! service's session login needs.
//!
//! Deliberately **not** `xindeler-authc`: its `sign_in()`/`register()`
//! helpers call `net_prehash()` on whatever they're given, and this service
//! always receives an *already* prehashed password from the frontend —
//! using those helpers would hash it a second time and break every login.
//! `xindeler-auth-common` supplies the wire types (zero risk of hand-typing
//! the JSON shape wrong); the HTTP calls themselves are ours.

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use std::time::Duration;
use xindeler_auth_common::{
    AuthToken, ChangePasswordPayload, ChangeUsernamePayload, DeleteAccountPayload,
    EmailVerificationRequiredResponse, ForgotPasswordPayload, ResetPasswordPayload, SignInPayload,
    SignInResponse, UsernameAvailabilityResponse, ValidityCheckPayload, ValidityCheckResponse,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum AuthClientError {
    /// xindeler-auth answered with a status outside 200-299. Carries only
    /// the status code — callers map it to their own public error code
    /// without echoing xindeler-auth's response body to the client.
    Rejected(u16),
    /// `sign_in()` specifically: xindeler-auth rejected the login with a
    /// `403 EMAIL_VERIFICATION_REQUIRED` — a legacy pre-2FA account that
    /// still needs to confirm an email before it can log in. Unlike other
    /// rejections, this one *is* forwarded to the client verbatim (session
    /// login is the only caller that unwraps this variant instead of
    /// collapsing it via `map_sign_in_error`), because the frontend's legacy
    /// recovery modal needs `completion_token`/`deadline` to keep working.
    EmailVerificationRequired(EmailVerificationRequiredResponse),
    Request(reqwest::Error),
    /// `verify()` was called but this service has no `AUTH_SERVICE_TOKEN`
    /// configured — a deployment problem, not a client input problem.
    MissingServiceToken,
}

impl From<reqwest::Error> for AuthClientError {
    fn from(err: reqwest::Error) -> Self {
        AuthClientError::Request(err)
    }
}

pub struct AuthClient {
    client: reqwest::blocking::Client,
    base_url: String,
    service_token: Option<String>,
}

impl AuthClient {
    pub fn new(base_url: &str, service_token: Option<&str>) -> Self {
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
        }
    }

    /// `password_prehash` must already be the client-side prehash — never a
    /// raw password (see module docs).
    pub fn sign_in(
        &self,
        username: &str,
        password_prehash: &str,
    ) -> Result<AuthToken, AuthClientError> {
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
        if !status.is_success() {
            // 403 is the one rejection worth inspecting: it's the only
            // status xindeler-auth uses for EMAIL_VERIFICATION_REQUIRED, and
            // any other shape on a 403 (or a body that fails to parse) just
            // falls through to the generic Rejected(403) every other
            // rejection gets.
            if status.as_u16() == 403 {
                if let Ok(body) = response.json::<EmailVerificationRequiredResponse>() {
                    if body.code == "EMAIL_VERIFICATION_REQUIRED" {
                        return Err(AuthClientError::EmailVerificationRequired(body));
                    }
                }
            }
            return Err(AuthClientError::Rejected(status.as_u16()));
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
        if !response.status().is_success() {
            return Err(AuthClientError::Rejected(response.status().as_u16()));
        }
        Ok(response.json()?)
    }

    // All four methods below are public, unauthenticated endpoints on
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
        if !response.status().is_success() {
            return Err(AuthClientError::Rejected(response.status().as_u16()));
        }
        Ok(response.json::<UsernameAvailabilityResponse>()?.available)
    }

    pub fn change_username(
        &self,
        old_username: &str,
        password_prehash: &str,
        new_username: &str,
    ) -> Result<(), AuthClientError> {
        let payload = ChangeUsernamePayload {
            old_username: old_username.to_owned(),
            password: password_prehash.to_owned(),
            new_username: new_username.to_owned(),
        };
        let response = self
            .client
            .post(format!("{}/change_username", self.base_url))
            .json(&payload)
            .send()?;
        if !response.status().is_success() {
            return Err(AuthClientError::Rejected(response.status().as_u16()));
        }
        Ok(())
    }

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
        if !response.status().is_success() {
            return Err(AuthClientError::Rejected(response.status().as_u16()));
        }
        Ok(())
    }

    pub fn delete_account(
        &self,
        username: &str,
        password_prehash: &str,
    ) -> Result<(), AuthClientError> {
        let payload = DeleteAccountPayload {
            username: username.to_owned(),
            password: password_prehash.to_owned(),
        };
        let response = self
            .client
            .post(format!("{}/delete_account", self.base_url))
            .json(&payload)
            .send()?;
        if !response.status().is_success() {
            return Err(AuthClientError::Rejected(response.status().as_u16()));
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
        if !response.status().is_success() {
            return Err(AuthClientError::Rejected(response.status().as_u16()));
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
        if !response.status().is_success() {
            return Err(AuthClientError::Rejected(response.status().as_u16()));
        }
        Ok(())
    }
}
