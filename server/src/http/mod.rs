//! Framework-agnostic HTTP request and response types.
//!
//! Everything above this module is written against these types, and only the
//! adapter below (`axum.rs`) knows which HTTP framework is actually serving
//! requests. That makes the framework a swappable implementation detail —
//! ported from the same pattern in `xindeler-auth`, where it was proven out
//! swapping rouille for axum without touching a single handler.

use serde::Serialize;
use std::borrow::Cow;
use std::net::SocketAddr;

mod axum;

pub use axum::serve;

/// Limit all request bodies to max 8 KiB.
///
/// The largest payload today is `POST /api/contribute` (name ≤100, skills
/// ≤300, portfolio ≤200 chars, JSON overhead) — well under this cap, with
/// headroom for the session/account payloads Fase 2 adds.
///
/// Bodies are read with a cap of `MAX_REQUEST_SIZE + 1` so that exactly
/// `MAX_REQUEST_SIZE` bytes still pass.
pub const MAX_REQUEST_SIZE: u64 = 1024 * 8;

/// Why a request body could not be produced.
///
/// The body is read up front by the adapter, but any failure is carried in
/// the `Request` rather than reported immediately, so handlers observe it at
/// exactly the point they ask for it — after rate-limit/auth checks have had
/// a chance to run first. An oversized unauthenticated request should stay a
/// 401 where relevant, not jump ahead to 413.
// Fase 0 has no handler that reads a request body yet (only `/ping`), so
// this whole type and the `body`/`get_param` accessors below are dead code
// until Fase 1 adds POST handlers. Kept now so the seam is complete and
// Fase 1 doesn't need to touch this file.
#[derive(Debug)]
#[allow(dead_code)]
pub enum BodyError {
    TooLarge,
    Io(std::io::Error),
}

pub struct Request {
    method: String,
    /// Raw request target, query string included.
    url: String,
    headers: Vec<(String, String)>,
    #[allow(dead_code)]
    body: Result<Vec<u8>, BodyError>,
    remote_addr: SocketAddr,
}

impl Request {
    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn raw_url(&self) -> &str {
        &self.url
    }

    pub fn remote_addr(&self) -> &SocketAddr {
        &self.remote_addr
    }

    #[allow(dead_code)]
    pub fn body(&self) -> Result<&[u8], &BodyError> {
        match &self.body {
            Ok(body) => Ok(body),
            Err(err) => Err(err),
        }
    }

    /// Case-insensitive header lookup; the first occurrence wins.
    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    }

    /// Reads a single cookie by name out of the `Cookie` header. The browser
    /// sends all cookies as one `name1=value1; name2=value2` header — this
    /// does not decode percent-encoding, since every cookie this service
    /// sets is an opaque hex token that never needs it.
    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.header("Cookie")?.split(';').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key.trim() == name).then(|| value.trim())
        })
    }

    #[allow(dead_code)]
    fn raw_query_string(&self) -> &str {
        match self.url.bytes().position(|byte| byte == b'?') {
            Some(pos) => self.url.split_at(pos + 1).1,
            None => "",
        }
    }

    /// Reads a query-string parameter.
    ///
    /// The first occurrence of a repeated key wins, `+` decodes to a space,
    /// and because the value is taken with `split('=').nth(1)` an embedded
    /// `=` truncates it (`?u=a=b` yields `a`). Covered by
    /// `query_parameters_are_parsed_consistently` below.
    #[allow(dead_code)]
    pub fn get_param(&self, param_name: &str) -> Option<String> {
        let name_pattern = &format!("{param_name}=");
        let param_pairs = self.raw_query_string().split('&');
        param_pairs
            .filter(|pair| pair.starts_with(name_pattern) || pair == &param_name)
            .map(|pair| pair.split('=').nth(1).unwrap_or(""))
            .next()
            .map(|value| {
                percent_encoding::percent_decode(value.replace('+', " ").as_bytes())
                    .decode_utf8_lossy()
                    .into_owned()
            })
    }
}

pub struct Response {
    pub status_code: u16,
    headers: Vec<(Cow<'static, str>, Cow<'static, str>)>,
    data: Vec<u8>,
}

impl Response {
    pub fn text<S: Into<String>>(text: S) -> Response {
        Response {
            status_code: 200,
            headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
            data: text.into().into_bytes(),
        }
    }

    pub fn json<T: Serialize>(content: &T) -> Response {
        match serde_json::to_vec(content) {
            Ok(data) => Response {
                status_code: 200,
                headers: vec![(
                    "Content-Type".into(),
                    "application/json; charset=utf-8".into(),
                )],
                data,
            },
            // Unreachable for the types we serialize, but `unwrap` is not
            // allowed in production code. Emit a literal envelope rather than
            // recursing into error::response, which would serialize again.
            Err(err) => {
                log::error!("failed to serialize response body: {err}");
                Response {
                    status_code: 500,
                    headers: vec![(
                        "Content-Type".into(),
                        "application/json; charset=utf-8".into(),
                    )],
                    data: br#"{"code":"INTERNAL_ERROR","message":"Internal server error."}"#
                        .to_vec(),
                }
            }
        }
    }

    pub fn empty_204() -> Response {
        Response {
            status_code: 204,
            headers: Vec::new(),
            data: Vec::new(),
        }
    }

    pub fn empty_404() -> Response {
        Response {
            status_code: 404,
            headers: Vec::new(),
            data: Vec::new(),
        }
    }

    pub fn with_status_code(mut self, code: u16) -> Response {
        self.status_code = code;
        self
    }

    /// Sets a header, replacing any existing occurrences (first position
    /// kept, later duplicates dropped).
    pub fn with_unique_header<H, V>(mut self, header: H, value: V) -> Response
    where
        H: Into<Cow<'static, str>>,
        V: Into<Cow<'static, str>>,
    {
        let header = header.into();
        let mut found_one = false;
        self.headers.retain(|(name, _)| {
            if name.eq_ignore_ascii_case(&header) {
                let keep = !found_one;
                found_one = true;
                keep
            } else {
                true
            }
        });

        match self
            .headers
            .iter_mut()
            .find(|(name, _)| name.eq_ignore_ascii_case(&header))
        {
            Some(existing) => existing.1 = value.into(),
            None => self.headers.push((header, value.into())),
        }
        self
    }

    #[cfg(test)]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_ref())
    }
}

#[cfg(test)]
impl Request {
    /// Builds a request for tests. Default peer is `127.0.0.1:12345`.
    pub fn fake<M: Into<String>, U: Into<String>>(method: M, url: U) -> Request {
        Request {
            method: method.into(),
            url: url.into(),
            headers: Vec::new(),
            body: Ok(Vec::new()),
            remote_addr: "127.0.0.1:12345".parse().expect("valid socket address"),
        }
    }

    pub fn with_header<K: Into<String>, V: Into<String>>(mut self, key: K, value: V) -> Request {
        self.headers.push((key.into(), value.into()));
        self
    }

    #[allow(dead_code)]
    pub fn with_body<B: Into<Vec<u8>>>(mut self, body: B) -> Request {
        self.body = Ok(body.into());
        self
    }

    pub fn with_peer(mut self, peer: SocketAddr) -> Request {
        self.remote_addr = peer;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{Request, Response};

    #[test]
    fn header_lookup_is_case_insensitive_and_takes_the_first_match() {
        let request = Request::fake("GET", "/")
            .with_header("Authorization", "Bearer first")
            .with_header("authorization", "Bearer second");

        assert_eq!(request.header("AUTHORIZATION"), Some("Bearer first"));
        assert_eq!(request.header("missing"), None);
    }

    #[test]
    fn cookie_finds_a_named_value_among_several() {
        let request =
            Request::fake("GET", "/").with_header("Cookie", "other=1; session=abc123; third=xyz");
        assert_eq!(request.cookie("session"), Some("abc123"));
        assert_eq!(request.cookie("missing"), None);
    }

    #[test]
    fn query_parameters_are_parsed_consistently() {
        let param = |url: &str| Request::fake("GET", url).get_param("username");

        assert_eq!(param("/c?username=freename"), Some("freename".to_owned()));
        assert_eq!(param("/c?username=%76alid"), Some("valid".to_owned()));
        assert_eq!(param("/c?username=a+b"), Some("a b".to_owned()));
        assert_eq!(param("/c?x=1&username=second"), Some("second".to_owned()));
        // First occurrence wins.
        assert_eq!(
            param("/c?username=one&username=two"),
            Some("one".to_owned())
        );
        assert_eq!(param("/c?username="), Some(String::new()));
        // Embedded '=' truncates the value.
        assert_eq!(param("/c?username=a=b"), Some("a".to_owned()));
        assert_eq!(param("/c"), None);
        assert_eq!(param("/c?other=1"), None);
    }

    #[test]
    fn with_unique_header_replaces_in_place_and_drops_duplicates() {
        let response = Response::text("body")
            .with_unique_header("X-Test", "first")
            .with_unique_header("x-test", "second");

        assert_eq!(response.header("X-TEST"), Some("second"));
        assert_eq!(
            response
                .headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("x-test"))
                .count(),
            1
        );
        // The pre-existing Content-Type is untouched.
        assert_eq!(
            response.header("Content-Type"),
            Some("text/plain; charset=utf-8")
        );
    }

    #[test]
    fn empty_responses_carry_no_headers() {
        assert_eq!(Response::empty_204().status_code, 204);
        assert!(Response::empty_204().headers.is_empty());
        assert_eq!(Response::empty_404().status_code, 404);
        assert!(Response::empty_404().headers.is_empty());
    }
}
