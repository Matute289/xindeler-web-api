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
    /// `sign_in` and `verify` are each `(status, body)` for their endpoint.
    fn start(sign_in: (u16, &'static str), verify: (u16, &'static str)) -> Self {
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
                        let (status, body) = if path.starts_with("/generate_token") {
                            sign_in
                        } else if path.starts_with("/verify") {
                            verify
                        } else {
                            (404, "{}")
                        };
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

#[test]
fn login_sets_a_session_cookie_that_me_and_logout_respect() {
    let auth = FakeAuthServer::start(
        (200, r#"{"token":"0123456789abcdef0123456789abcdef"}"#),
        (
            200,
            r#"{"uuid":"11111111-1111-1111-1111-111111111111","username":"tester"}"#,
        ),
    );
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
    let auth = FakeAuthServer::start((401, "{}"), (200, "{}"));
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
