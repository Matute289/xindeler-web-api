//! Black-box integration tests — spawns the real binary on a free port with
//! a temp SQLite DB, same pattern as `xindeler-auth`'s
//! `server/tests/http_security.rs`. These exist because the unit tests in
//! `web.rs`/`waitlist.rs` call functions directly; only a real running
//! binary proves the router/CORS/headers/rate-limiter are actually wired
//! together correctly end to end.

use reqwest::blocking::{Client, Response};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

struct TestServer {
    child: Child,
    base_url: String,
    database: PathBuf,
}

impl TestServer {
    fn start() -> Self {
        Self::start_with(&[])
    }

    /// Entries in `extra_env` are applied after the defaults, so they override them.
    fn start_with(extra_env: &[(&str, &str)]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let database = std::env::temp_dir().join(format!(
            "xindeler-web-api-http-integration-{}.db",
            rand::random::<u64>()
        ));
        let mut command = Command::new(env!("CARGO_BIN_EXE_xindeler-web-api-server"));
        command
            .env("WEB_API_DB_DIR", &database)
            .env("WEB_API_BIND_ADDR", format!("127.0.0.1:{port}"))
            .env("WEB_API_TRUSTED_PROXIES", "127.0.0.0/8")
            .env("WEB_API_HTTP_WORKERS", "2")
            // No game server listening on this port in CI — /api/status
            // should report `online: false`, not error.
            .env("WEB_API_GAME_SERVER_ADDR", "127.0.0.1:1")
            .env("WEB_API_RATE_LIMIT_MAX", "2")
            .env("WEB_API_RATE_LIMIT_WINDOW_SECS", "3600")
            .env("RUST_LOG", "off")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (key, value) in extra_env {
            command.env(key, value);
        }
        let child = command.spawn().expect("failed to spawn test server");
        let server = Self {
            child,
            base_url: format!("http://127.0.0.1:{port}"),
            database,
        };
        server.wait_until_ready();
        server
    }

    fn wait_until_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if reqwest::blocking::get(format!("{}/ping", self.base_url)).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("test server did not become ready");
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.database);
        let _ = std::fs::remove_file(format!("{}-wal", self.database.display()));
        let _ = std::fs::remove_file(format!("{}-shm", self.database.display()));
    }
}

fn body_json(response: Response) -> Value {
    response.json().expect("response body must be JSON")
}

fn waitlist_payload(email: &str) -> Value {
    json!({
        "name": "Ana Test",
        "email": email,
        "platform": "windows",
        "source": "github",
    })
}

#[test]
fn status_reports_offline_when_no_game_server_listens() {
    let server = TestServer::start();
    let response = Client::new().get(server.url("/api/status")).send().unwrap();
    assert_eq!(response.status(), 200);
    let body = body_json(response);
    assert_eq!(body["online"], false);
    assert!(body["checked_at"].as_str().unwrap().contains('T'));
}

#[test]
fn waitlist_count_starts_at_zero_and_increments_after_join() {
    let server = TestServer::start();
    let client = Client::new();

    let before = body_json(
        client
            .get(server.url("/api/waitlist/count"))
            .send()
            .unwrap(),
    );
    assert_eq!(before["count"], 0);

    let response = client
        .post(server.url("/api/waitlist"))
        .json(&waitlist_payload("count-test@example.com"))
        .send()
        .unwrap();
    assert_eq!(response.status(), 201);
    assert_eq!(body_json(response)["ok"], true);

    let after = body_json(
        client
            .get(server.url("/api/waitlist/count"))
            .send()
            .unwrap(),
    );
    assert_eq!(after["count"], 1);
}

#[test]
fn duplicate_waitlist_email_returns_ok_without_erroring() {
    let server = TestServer::start();
    let client = Client::new();
    let payload = waitlist_payload("dup-test@example.com");

    let first = client
        .post(server.url("/api/waitlist"))
        .json(&payload)
        .send()
        .unwrap();
    assert_eq!(first.status(), 201);

    let second = client
        .post(server.url("/api/waitlist"))
        .json(&payload)
        .send()
        .unwrap();
    // Same 201 + {"ok": true} as a fresh signup — anti-enumeration by
    // design, matching the Python service this replaces.
    assert_eq!(second.status(), 201);
    assert_eq!(body_json(second)["ok"], true);
}

#[test]
fn invalid_platform_is_rejected_with_422() {
    let server = TestServer::start();
    let mut payload = waitlist_payload("bad-platform@example.com");
    payload["platform"] = json!("amiga");

    let response = Client::new()
        .post(server.url("/api/waitlist"))
        .json(&payload)
        .send()
        .unwrap();
    assert_eq!(response.status(), 422);
    assert_eq!(body_json(response)["code"], "INVALID_REQUEST");
}

#[test]
fn honeypot_is_accepted_silently_without_being_stored() {
    let server = TestServer::start();
    let client = Client::new();
    let mut payload = waitlist_payload("honeypot@example.com");
    payload["honeypot"] = json!("i-am-a-bot");

    let response = client
        .post(server.url("/api/waitlist"))
        .json(&payload)
        .send()
        .unwrap();
    assert_eq!(response.status(), 201);

    let count = body_json(
        client
            .get(server.url("/api/waitlist/count"))
            .send()
            .unwrap(),
    );
    assert_eq!(count["count"], 0);
}

#[test]
fn rate_limit_kicks_in_after_configured_max() {
    // WEB_API_RATE_LIMIT_MAX=2 (see TestServer::start_with defaults).
    let server = TestServer::start();
    let client = Client::new();

    for i in 0..2 {
        let response = client
            .post(server.url("/api/waitlist"))
            .json(&waitlist_payload(&format!("rl-{i}@example.com")))
            .send()
            .unwrap();
        assert_eq!(response.status(), 201, "request {i} should succeed");
    }

    let third = client
        .post(server.url("/api/waitlist"))
        .json(&waitlist_payload("rl-3@example.com"))
        .send()
        .unwrap();
    assert_eq!(third.status(), 429);
    assert_eq!(body_json(third)["code"], "RATE_LIMITED");
}

#[test]
fn contribute_endpoint_accepts_a_valid_submission() {
    let server = TestServer::start();
    let response = Client::new()
        .post(server.url("/api/contribute"))
        .json(&json!({
            "name": "Caro Dev",
            "email": "caro@example.com",
            "skills": "rust, backend",
            "portfolio": "https://github.com/caro",
        }))
        .send()
        .unwrap();
    assert_eq!(response.status(), 201);
    assert_eq!(body_json(response)["ok"], true);
}

// --- Session tests: a hand-rolled fake xindeler-auth, just enough to serve
// fixed responses for /generate_token and /verify. No new test dependency
// (a real HTTP mock crate would be overkill for two canned JSON bodies).

struct FakeAuthServer {
    base_url: String,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl FakeAuthServer {
    /// `routes` maps a path prefix to a canned `(status, body)` — first
    /// matching prefix wins, unmatched requests get a bare 404.
    fn start(routes: &'static [(&'static str, u16, &'static str)]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let mut buf = [0_u8; 4096];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let request = String::from_utf8_lossy(&buf[..n]);
                        let path = request
                            .lines()
                            .next()
                            .and_then(|line| line.split_whitespace().nth(1))
                            .unwrap_or("");
                        let (status, body) = routes
                            .iter()
                            .find(|(prefix, _, _)| path.starts_with(prefix))
                            .map(|(_, status, body)| (*status, *body))
                            .unwrap_or((404, "{}"));
                        let response = format!(
                            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                    }
                    Err(ref err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            base_url: format!("http://{addr}"),
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for FakeAuthServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

const SERVICE_TOKEN: &str = "0123456789abcdef0123456789abcdef";
const SIGN_IN_OK: (&str, u16, &str) = (
    "/generate_token",
    200,
    r#"{"token":"0123456789abcdef0123456789abcdef"}"#,
);
const VERIFY_OK: (&str, u16, &str) = (
    "/verify",
    200,
    r#"{"uuid":"11111111-1111-1111-1111-111111111111","username":"tester"}"#,
);

fn login_and_get_cookie(server: &TestServer, client: &Client) -> String {
    let login = client
        .post(server.url("/api/session/login"))
        .json(&json!({"username": "tester", "password_prehash": "deadbeef"}))
        .send()
        .unwrap();
    assert_eq!(login.status(), 200);
    login
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

#[test]
fn login_sets_a_session_cookie_that_me_and_logout_respect() {
    let auth = FakeAuthServer::start(&[SIGN_IN_OK, VERIFY_OK]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);
    let client = Client::new();

    // No cookie yet — /me is unauthorized.
    let before = client.get(server.url("/api/session/me")).send().unwrap();
    assert_eq!(before.status(), 401);

    let login = client
        .post(server.url("/api/session/login"))
        .json(&json!({"username": "tester", "password_prehash": "deadbeef"}))
        .send()
        .unwrap();
    assert_eq!(login.status(), 200);
    let set_cookie = login
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(set_cookie.starts_with("session="));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("Secure"));
    assert!(set_cookie.contains("SameSite=Lax"));
    assert_eq!(body_json(login)["username"], "tester");

    // reqwest's Client doesn't auto-store cookies unless built with a jar —
    // forward the Set-Cookie value by hand, same as a browser would.
    let cookie_value = set_cookie.split(';').next().unwrap().to_owned();

    let me = client
        .get(server.url("/api/session/me"))
        .header("Cookie", &cookie_value)
        .send()
        .unwrap();
    assert_eq!(me.status(), 200);
    assert_eq!(body_json(me)["username"], "tester");

    let logout = client
        .post(server.url("/api/session/logout"))
        .header("Cookie", &cookie_value)
        .send()
        .unwrap();
    assert_eq!(logout.status(), 200);

    let after_logout = client
        .get(server.url("/api/session/me"))
        .header("Cookie", &cookie_value)
        .send()
        .unwrap();
    assert_eq!(after_logout.status(), 401);
}

#[test]
fn invalid_credentials_return_401_without_setting_a_cookie() {
    let auth = FakeAuthServer::start(&[("/generate_token", 401, "{}"), ("/verify", 200, "{}")]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);

    let response = Client::new()
        .post(server.url("/api/session/login"))
        .json(&json!({"username": "tester", "password_prehash": "deadbeef"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(body_json(response)["code"], "INVALID_CREDENTIALS");
}

#[test]
fn invalid_credentials_return_401_even_when_xindeler_auth_answers_400() {
    // The real, deployed xindeler-auth answers AuthError::InvalidLogin with
    // 400, not 401 (confirmed via the B-05 production smoke test) — this
    // regressed silently before because FakeAuthServer's other login tests
    // all used 401, which isn't what the real service sends. Without this,
    // every wrong-password login attempt surfaced as a 502
    // "authentication service unavailable" instead of a 401.
    let auth = FakeAuthServer::start(&[("/generate_token", 400, "{}")]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);

    let response = Client::new()
        .post(server.url("/api/session/login"))
        .json(&json!({"username": "tester", "password_prehash": "deadbeef"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(body_json(response)["code"], "INVALID_CREDENTIALS");
}

#[test]
fn login_forwards_email_verification_required_verbatim_without_a_cookie() {
    let auth = FakeAuthServer::start(&[(
        "/generate_token",
        403,
        r#"{"code":"EMAIL_VERIFICATION_REQUIRED","message":"Email verification is required before login.","deadline":1999999999,"completion_token":"legacy-completion-token"}"#,
    )]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);

    let response = Client::new()
        .post(server.url("/api/session/login"))
        .json(&json!({"username": "legacyuser", "password_prehash": "deadbeef"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 403);
    assert!(response.headers().get("set-cookie").is_none());
    let body = body_json(response);
    assert_eq!(body["code"], "EMAIL_VERIFICATION_REQUIRED");
    assert_eq!(body["completion_token"], "legacy-completion-token");
    assert_eq!(body["deadline"], 1999999999);
}

#[test]
fn login_treats_an_unparseable_403_body_as_generic_invalid_credentials() {
    let auth = FakeAuthServer::start(&[("/generate_token", 403, "not json")]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);

    let response = Client::new()
        .post(server.url("/api/session/login"))
        .json(&json!({"username": "tester", "password_prehash": "deadbeef"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(body_json(response)["code"], "INVALID_CREDENTIALS");
}

// --- C-02: /api/account/* proxy tests ---

#[test]
fn check_username_proxies_availability() {
    let auth = FakeAuthServer::start(&[("/check-username", 200, r#"{"available":true}"#)]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);

    let response = Client::new()
        .get(server.url("/api/account/check-username?username=nuevonombre"))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response)["available"], true);
}

#[test]
fn account_endpoints_require_a_session() {
    let server = TestServer::start();

    let change_username = Client::new()
        .post(server.url("/api/account/change-username"))
        .json(&json!({"new_username": "nuevo", "password_prehash": "deadbeef"}))
        .send()
        .unwrap();
    assert_eq!(change_username.status(), 401);

    let change_password = Client::new()
        .post(server.url("/api/account/change-password"))
        .json(&json!({"current_password_prehash": "a", "new_password_prehash": "b"}))
        .send()
        .unwrap();
    assert_eq!(change_password.status(), 401);

    let delete = Client::new()
        .post(server.url("/api/account/delete"))
        .json(&json!({"password_prehash": "deadbeef"}))
        .send()
        .unwrap();
    assert_eq!(delete.status(), 401);
}

#[test]
fn change_username_revokes_the_session_that_made_the_change() {
    let auth = FakeAuthServer::start(&[SIGN_IN_OK, VERIFY_OK, ("/change_username", 200, "Ok")]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);
    let client = Client::new();
    let cookie = login_and_get_cookie(&server, &client);

    let response = client
        .post(server.url("/api/account/change-username"))
        .header("Cookie", &cookie)
        .json(&json!({"new_username": "nuevonombre", "password_prehash": "deadbeef"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response)["ok"], true);

    // The session that made the change is revoked — same cookie, now 401.
    let me = client
        .get(server.url("/api/session/me"))
        .header("Cookie", &cookie)
        .send()
        .unwrap();
    assert_eq!(me.status(), 401);
}

#[test]
fn change_password_revokes_the_session() {
    let auth = FakeAuthServer::start(&[SIGN_IN_OK, VERIFY_OK, ("/change_password", 200, "Ok")]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);
    let client = Client::new();
    let cookie = login_and_get_cookie(&server, &client);

    let response = client
        .post(server.url("/api/account/change-password"))
        .header("Cookie", &cookie)
        .json(&json!({"current_password_prehash": "old", "new_password_prehash": "new"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);

    let me = client
        .get(server.url("/api/session/me"))
        .header("Cookie", &cookie)
        .send()
        .unwrap();
    assert_eq!(me.status(), 401);
}

#[test]
fn delete_account_revokes_the_session() {
    let auth = FakeAuthServer::start(&[SIGN_IN_OK, VERIFY_OK, ("/delete_account", 200, "Ok")]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);
    let client = Client::new();
    let cookie = login_and_get_cookie(&server, &client);

    let response = client
        .post(server.url("/api/account/delete"))
        .header("Cookie", &cookie)
        .json(&json!({"password_prehash": "deadbeef"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);

    let me = client
        .get(server.url("/api/session/me"))
        .header("Cookie", &cookie)
        .send()
        .unwrap();
    assert_eq!(me.status(), 401);
}

#[test]
fn change_password_with_wrong_current_password_does_not_revoke_the_session() {
    let auth = FakeAuthServer::start(&[SIGN_IN_OK, VERIFY_OK, ("/change_password", 401, "{}")]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);
    let client = Client::new();
    let cookie = login_and_get_cookie(&server, &client);

    let response = client
        .post(server.url("/api/account/change-password"))
        .header("Cookie", &cookie)
        .json(&json!({"current_password_prehash": "wrong", "new_password_prehash": "new"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 401);

    // xindeler-auth rejected the change — the session must still be alive.
    let me = client
        .get(server.url("/api/session/me"))
        .header("Cookie", &cookie)
        .send()
        .unwrap();
    assert_eq!(me.status(), 200);
}

// --- C-03: /api/account/{forgot,reset}-password proxy tests ---

const FORGOT_PASSWORD_OK: (&str, u16, &str) = (
    "/forgot-password",
    200,
    "If that account exists, a reset email was sent.",
);
const RESET_PASSWORD_OK: (&str, u16, &str) = ("/reset-password", 200, "Password updated");

#[test]
fn forgot_password_does_not_require_a_session_and_always_answers_ok() {
    let auth = FakeAuthServer::start(&[FORGOT_PASSWORD_OK]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);

    let response = Client::new()
        .post(server.url("/api/account/forgot-password"))
        .json(&json!({"email": "nobody@example.com"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response)["ok"], true);
}

#[test]
fn forgot_password_rejects_an_empty_email() {
    let server = TestServer::start();

    let response = Client::new()
        .post(server.url("/api/account/forgot-password"))
        .json(&json!({"email": ""}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 422);
}

#[test]
fn reset_password_succeeds_without_a_session() {
    let auth = FakeAuthServer::start(&[RESET_PASSWORD_OK]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);

    let response = Client::new()
        .post(server.url("/api/account/reset-password"))
        .json(&json!({"token": "sometoken", "new_password_prehash": "newhash"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response)["ok"], true);
}

#[test]
fn reset_password_rejects_empty_fields() {
    let server = TestServer::start();

    let response = Client::new()
        .post(server.url("/api/account/reset-password"))
        .json(&json!({"token": "", "new_password_prehash": "newhash"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 422);
}

#[test]
fn reset_password_propagates_an_invalid_or_expired_token_as_422() {
    let auth = FakeAuthServer::start(&[("/reset-password", 400, "{}")]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);

    let response = Client::new()
        .post(server.url("/api/account/reset-password"))
        .json(&json!({"token": "expired", "new_password_prehash": "newhash"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 422);
}

#[test]
fn reset_password_revokes_the_callers_session_when_one_is_present() {
    let auth = FakeAuthServer::start(&[SIGN_IN_OK, VERIFY_OK, RESET_PASSWORD_OK]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);
    let client = Client::new();
    let cookie = login_and_get_cookie(&server, &client);

    let response = client
        .post(server.url("/api/account/reset-password"))
        .header("Cookie", &cookie)
        .json(&json!({"token": "sometoken", "new_password_prehash": "newhash"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);

    // Same-request bonus: a session cookie carried into the reset call gets
    // revoked too, even though xindeler-auth's reset-password doesn't
    // surface the uuid needed to revoke every session for the account.
    let me = client
        .get(server.url("/api/session/me"))
        .header("Cookie", &cookie)
        .send()
        .unwrap();
    assert_eq!(me.status(), 401);
}

// --- 005: 2FA/TOTP proxy tests ---

const TOTP_CHALLENGE_ID: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn login_with_totp_confirmed_returns_a_202_challenge_without_a_cookie() {
    let auth = FakeAuthServer::start(&[(
        "/generate_token",
        202,
        r#"{"challenge_id":"0123456789abcdef0123456789abcdef","expires_in":300}"#,
    )]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);

    let response = Client::new()
        .post(server.url("/api/session/login"))
        .json(&json!({"username": "tester", "password_prehash": "deadbeef"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 202);
    assert!(response.headers().get("set-cookie").is_none());
    let body = body_json(response);
    assert_eq!(body["challenge_id"], TOTP_CHALLENGE_ID);
    assert_eq!(body["expires_in"], 300);
}

#[test]
fn login_2fa_completes_the_challenge_and_marks_the_session_totp_enabled() {
    let auth = FakeAuthServer::start(&[
        (
            "/login/2fa",
            200,
            r#"{"token":"0123456789abcdef0123456789abcdef"}"#,
        ),
        VERIFY_OK,
    ]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);
    let client = Client::new();

    let response = client
        .post(server.url("/api/session/login/2fa"))
        .json(&json!({"challenge_id": TOTP_CHALLENGE_ID, "code": "123456"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    let cookie = response
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    assert_eq!(body_json(response)["totp_enabled"], true);

    let me = client
        .get(server.url("/api/session/me"))
        .header("Cookie", &cookie)
        .send()
        .unwrap();
    assert_eq!(me.status(), 200);
    assert_eq!(body_json(me)["totp_enabled"], true);
}

#[test]
fn login_2fa_forwards_an_invalid_code_verbatim() {
    let auth = FakeAuthServer::start(&[(
        "/login/2fa",
        400,
        r#"{"code":"TOTP_INVALID_CODE","message":"The code was incorrect.","request_id":"x"}"#,
    )]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);

    let response = Client::new()
        .post(server.url("/api/session/login/2fa"))
        .json(&json!({"challenge_id": TOTP_CHALLENGE_ID, "code": "000000"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 400);
    assert!(response.headers().get("set-cookie").is_none());
    assert_eq!(body_json(response)["code"], "TOTP_INVALID_CODE");
}

#[test]
fn login_2fa_rejects_a_malformed_challenge_id() {
    let server = TestServer::start();

    let response = Client::new()
        .post(server.url("/api/session/login/2fa"))
        .json(&json!({"challenge_id": "not-hex", "code": "123456"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 422);
}

#[test]
fn totp_endpoints_require_a_session() {
    let server = TestServer::start();
    let client = Client::new();

    let enroll = client
        .post(server.url("/api/account/2fa/enroll"))
        .json(&json!({"password_prehash": "deadbeef"}))
        .send()
        .unwrap();
    assert_eq!(enroll.status(), 401);

    let confirm = client
        .post(server.url("/api/account/2fa/confirm"))
        .json(&json!({"password_prehash": "deadbeef", "code": "123456"}))
        .send()
        .unwrap();
    assert_eq!(confirm.status(), 401);

    let disable = client
        .post(server.url("/api/account/2fa/disable"))
        .json(&json!({"password_prehash": "deadbeef", "code": "123456"}))
        .send()
        .unwrap();
    assert_eq!(disable.status(), 401);

    let regenerate = client
        .post(server.url("/api/account/2fa/backup-codes/regenerate"))
        .json(&json!({"password_prehash": "deadbeef", "code": "123456"}))
        .send()
        .unwrap();
    assert_eq!(regenerate.status(), 401);
}

#[test]
fn totp_enroll_returns_the_qr_and_secret() {
    let auth = FakeAuthServer::start(&[
        SIGN_IN_OK,
        VERIFY_OK,
        (
            "/2fa/enroll",
            200,
            r#"{"secret_base32":"JBSWY3DPEHPK3PXP","otpauth_url":"otpauth://totp/Xindeler:tester","qr_png_base64":"iVBORw0KG=="}"#,
        ),
    ]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);
    let client = Client::new();
    let cookie = login_and_get_cookie(&server, &client);

    let response = client
        .post(server.url("/api/account/2fa/enroll"))
        .header("Cookie", &cookie)
        .json(&json!({"password_prehash": "deadbeef"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = body_json(response);
    assert_eq!(body["secret_base32"], "JBSWY3DPEHPK3PXP");
    assert_eq!(body["qr_png_base64"], "iVBORw0KG==");
}

#[test]
fn totp_confirm_marks_the_account_as_totp_enabled_without_revoking_the_session() {
    let auth = FakeAuthServer::start(&[
        SIGN_IN_OK,
        VERIFY_OK,
        (
            "/2fa/confirm",
            200,
            r#"{"backup_codes":["aaaa1111","bbbb2222"]}"#,
        ),
    ]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);
    let client = Client::new();
    let cookie = login_and_get_cookie(&server, &client);

    let response = client
        .post(server.url("/api/account/2fa/confirm"))
        .header("Cookie", &cookie)
        .json(&json!({"password_prehash": "deadbeef", "code": "123456"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        body_json(response)["backup_codes"],
        json!(["aaaa1111", "bbbb2222"])
    );

    // Confirming a new enrollment doesn't need to force a relogin — unlike
    // disabling it, it doesn't reduce the account's security.
    let me = client
        .get(server.url("/api/session/me"))
        .header("Cookie", &cookie)
        .send()
        .unwrap();
    assert_eq!(me.status(), 200);
    assert_eq!(body_json(me)["totp_enabled"], true);
}

#[test]
fn totp_disable_marks_totp_disabled_and_revokes_the_session() {
    let auth = FakeAuthServer::start(&[SIGN_IN_OK, VERIFY_OK, ("/2fa/disable", 200, "Ok")]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);
    let client = Client::new();
    let cookie = login_and_get_cookie(&server, &client);

    let response = client
        .post(server.url("/api/account/2fa/disable"))
        .header("Cookie", &cookie)
        .json(&json!({"password_prehash": "deadbeef", "code": "123456"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);

    let me = client
        .get(server.url("/api/session/me"))
        .header("Cookie", &cookie)
        .send()
        .unwrap();
    assert_eq!(me.status(), 401);
}

#[test]
fn totp_disable_forwards_account_locked_verbatim() {
    let auth = FakeAuthServer::start(&[
        SIGN_IN_OK,
        VERIFY_OK,
        (
            "/2fa/disable",
            423,
            r#"{"code":"ACCOUNT_2FA_LOCKED","message":"This account is locked after repeated incorrect 2FA codes. Contact support.","request_id":"x"}"#,
        ),
    ]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);
    let client = Client::new();
    let cookie = login_and_get_cookie(&server, &client);

    let response = client
        .post(server.url("/api/account/2fa/disable"))
        .header("Cookie", &cookie)
        .json(&json!({"password_prehash": "deadbeef", "code": "000000"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 423);
    assert_eq!(body_json(response)["code"], "ACCOUNT_2FA_LOCKED");

    // Locked-out rejection doesn't touch the session — it's not a
    // successful disable, so there's nothing to force a relogin over.
    let me = client
        .get(server.url("/api/session/me"))
        .header("Cookie", &cookie)
        .send()
        .unwrap();
    assert_eq!(me.status(), 200);
}

#[test]
fn totp_backup_codes_regenerate_returns_new_codes() {
    let auth = FakeAuthServer::start(&[
        SIGN_IN_OK,
        VERIFY_OK,
        (
            "/2fa/backup-codes/regenerate",
            200,
            r#"{"backup_codes":["cccc3333","dddd4444"]}"#,
        ),
    ]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);
    let client = Client::new();
    let cookie = login_and_get_cookie(&server, &client);

    let response = client
        .post(server.url("/api/account/2fa/backup-codes/regenerate"))
        .header("Cookie", &cookie)
        .json(&json!({"password_prehash": "deadbeef", "code": "123456"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        body_json(response)["backup_codes"],
        json!(["cccc3333", "dddd4444"])
    );
}

#[test]
fn change_username_forwards_totp_invalid_code_instead_of_generic_401() {
    let auth = FakeAuthServer::start(&[
        SIGN_IN_OK,
        VERIFY_OK,
        (
            "/change_username",
            400,
            r#"{"code":"TOTP_INVALID_CODE","message":"The code was incorrect.","request_id":"x"}"#,
        ),
    ]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);
    let client = Client::new();
    let cookie = login_and_get_cookie(&server, &client);

    let response = client
        .post(server.url("/api/account/change-username"))
        .header("Cookie", &cookie)
        .json(&json!({"new_username": "newname", "password_prehash": "deadbeef", "code": "000000"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(body_json(response)["code"], "TOTP_INVALID_CODE");

    // A rejected change (even a TOTP-specific one) must not revoke the
    // session — same invariant as the plain-401 case already covered by
    // change_password_with_wrong_current_password_does_not_revoke_the_session.
    let me = client
        .get(server.url("/api/session/me"))
        .header("Cookie", &cookie)
        .send()
        .unwrap();
    assert_eq!(me.status(), 200);
}

#[test]
fn change_username_forwards_the_30_day_cooldown_instead_of_generic_401() {
    let auth = FakeAuthServer::start(&[
        SIGN_IN_OK,
        VERIFY_OK,
        (
            "/change_username",
            400,
            r#"{"code":"USERNAME_CHANGE_COOLDOWN","message":"That account changed its username recently.","request_id":"x"}"#,
        ),
    ]);
    let server = TestServer::start_with(&[
        ("AUTH_PUBLIC_URL", &auth.base_url),
        ("AUTH_SERVICE_TOKEN", SERVICE_TOKEN),
    ]);
    let client = Client::new();
    let cookie = login_and_get_cookie(&server, &client);

    let response = client
        .post(server.url("/api/account/change-username"))
        .header("Cookie", &cookie)
        .json(&json!({"new_username": "newname", "password_prehash": "deadbeef"}))
        .send()
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(body_json(response)["code"], "USERNAME_CHANGE_COOLDOWN");
}

#[test]
fn every_response_carries_privacy_and_cors_headers() {
    let server = TestServer::start();
    let response = Client::new()
        .get(server.url("/api/waitlist/count"))
        .header("Origin", "https://xindeler.com")
        .send()
        .unwrap();
    let headers = response.headers();
    assert_eq!(headers.get("Cache-Control").unwrap(), "no-store");
    assert_eq!(headers.get("Referrer-Policy").unwrap(), "no-referrer");
    assert_eq!(
        headers.get("Access-Control-Allow-Origin").unwrap(),
        "https://xindeler.com"
    );
}

#[test]
fn unknown_origin_gets_no_cors_header() {
    let server = TestServer::start();
    let response = Client::new()
        .get(server.url("/api/waitlist/count"))
        .header("Origin", "https://evil.example.com")
        .send()
        .unwrap();
    assert!(response
        .headers()
        .get("Access-Control-Allow-Origin")
        .is_none());
}

#[test]
fn sigterm_shuts_down_cleanly_and_promptly() {
    let mut server = TestServer::start();

    let status = Command::new("kill")
        .arg("-TERM")
        .arg(server.child.id().to_string())
        .status()
        .expect("failed to invoke kill(1)");
    assert!(status.success(), "kill(1) itself failed to run");

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            assert!(status.success(), "server should exit 0 on SIGTERM");
            break;
        }
        assert!(Instant::now() < deadline, "server did not exit in time");
        std::thread::sleep(Duration::from_millis(50));
    }
}
