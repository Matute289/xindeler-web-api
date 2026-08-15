//! Thin, hand-rolled HTTP client for the two `xindeler-auth` endpoints this
//! service's session login needs.
//!
//! Deliberately **not** `xindeler-authc`: its `sign_in()`/`register()`
//! helpers call `net_prehash()` on whatever they're given, and this service
//! always receives an *already* prehashed password from the frontend —
//! using those helpers would hash it a second time and break every login.
//! `xindeler-auth-common` supplies the wire types (zero risk of hand-typing
//! the JSON shape wrong); the HTTP calls themselves are ours.

use std::time::Duration;
use xindeler_auth_common::{
    AuthToken, SignInPayload, SignInResponse, ValidityCheckPayload, ValidityCheckResponse,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum AuthClientError {
    /// xindeler-auth answered with a status outside 200-299. Carries only
    /// the status code — callers map it to their own public error code
    /// without echoing xindeler-auth's response body to the client.
    Rejected(u16),
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
        if !response.status().is_success() {
            return Err(AuthClientError::Rejected(response.status().as_u16()));
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
}
