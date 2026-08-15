use crate::db::db;
use crate::error::ApiError;
use crate::http::Response;
use crate::mailer;
use crate::state::AppState;
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::net::{IpAddr, TcpStream};
use std::time::Duration;
use xindeler_web_api_common::{
    ContributePayload, CountResponse, OkResponse, StatusResponse, WaitlistPayload,
};

const VALID_PLATFORMS: [&str; 3] = ["windows", "linux", "macos"];
const VALID_SOURCES: [&str; 5] = ["github", "social", "friend", "search", "other"];
const STATUS_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Trims, strips control characters (defense against header injection if
/// this value ever ends up in an email subject — the Python service this
/// replaces didn't guard against that), and prefixes a leading
/// `=`/`+`/`-`/`@` with a quote — a CSV-formula-injection guard, kept in
/// case this data is ever exported to a spreadsheet.
fn sanitize(value: &str) -> String {
    let trimmed: String = value.trim().chars().filter(|c| !c.is_control()).collect();
    if trimmed.starts_with(['=', '+', '-', '@']) {
        format!("'{trimmed}")
    } else {
        trimmed
    }
}

fn is_valid_email(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() || value.len() > 254 || !value.is_ascii() {
        return false;
    }
    if value.matches('@').count() != 1 {
        return false;
    }
    let mut parts = value.splitn(2, '@');
    let (Some(local), Some(domain)) = (parts.next(), parts.next()) else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
}

fn now_unix() -> i64 {
    Utc::now().timestamp()
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, false)
}

// --- GET /api/status ---

fn probe_game_server() -> bool {
    let addr = crate::config::get().game_server_addr;
    TcpStream::connect_timeout(&addr, STATUS_PROBE_TIMEOUT).is_ok()
}

pub fn server_status(state: &AppState) -> Result<Response, ApiError> {
    if let Some((online, checked_at)) = state.status_cache.get() {
        return Ok(Response::json(&StatusResponse { online, checked_at }));
    }
    let online = probe_game_server();
    let checked_at = now_iso();
    state.status_cache.set((online, checked_at.clone()));
    Ok(Response::json(&StatusResponse { online, checked_at }))
}

// --- GET /api/waitlist/count ---

fn count_waitlist(db: &Connection) -> Result<u64, ApiError> {
    let count: i64 = db.query_row("SELECT COUNT(*) FROM waitlist", [], |row| row.get(0))?;
    Ok(count as u64)
}

pub fn waitlist_count(state: &AppState) -> Result<Response, ApiError> {
    if let Some(count) = state.count_cache.get() {
        return Ok(Response::json(&CountResponse { count }));
    }
    let count = count_waitlist(&db()?)?;
    state.count_cache.set(count);
    Ok(Response::json(&CountResponse { count }))
}

// --- POST /api/waitlist ---

fn email_exists(db: &Connection, table: &str, email: &str) -> Result<bool, ApiError> {
    // `table` is never user input — always one of the two literals below.
    let query = format!("SELECT 1 FROM {table} WHERE email = ?1 COLLATE NOCASE LIMIT 1");
    Ok(db
        .query_row(&query, params![email], |_| Ok(()))
        .optional()?
        .is_some())
}

pub fn join_waitlist(
    body: &[u8],
    remote_ip: IpAddr,
    state: &AppState,
) -> Result<Response, ApiError> {
    let payload: WaitlistPayload =
        serde_json::from_slice(body).map_err(|err| ApiError::InvalidRequest(err.to_string()))?;

    // Honeypot: silently accept without writing or charging the rate limit
    // — same as the Python service, so a bot filling the hidden field
    // can't distinguish this from a real submission.
    if !payload.honeypot.is_empty() {
        return Ok(Response::json(&OkResponse { ok: true }).with_status_code(201));
    }

    let name = sanitize(&payload.name);
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::InvalidRequest("invalid name".into()));
    }
    let email = sanitize(&payload.email);
    if !is_valid_email(&email) {
        return Err(ApiError::InvalidRequest("invalid email".into()));
    }
    if !VALID_PLATFORMS.contains(&payload.platform.as_str()) {
        return Err(ApiError::InvalidRequest("invalid platform".into()));
    }
    if !VALID_SOURCES.contains(&payload.source.as_str()) {
        return Err(ApiError::InvalidRequest("invalid source".into()));
    }

    // Rate limit BEFORE the dedup check — the Python service checks dedup
    // first, so a known-good email can be replayed for free, each replay
    // triggering a full table scan. This is the fix (bug #1 of the
    // migration plan).
    if !state.waitlist_requests.check(remote_ip) {
        return Err(ApiError::RateLimit);
    }

    let conn = db()?;
    if email_exists(&conn, "waitlist", &email)? {
        return Ok(Response::json(&OkResponse { ok: true }).with_status_code(201));
    }

    conn.execute(
        "INSERT INTO waitlist (created_at, name, email, platform, source) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![now_unix(), name, email, payload.platform, payload.source],
    )?;
    state.count_cache.clear();

    mailer::send_html_email(
        email,
        "¡Ya estás en la lista de espera de Xindeler!".to_owned(),
        mailer::waitlist_html(&name),
    );

    Ok(Response::json(&OkResponse { ok: true }).with_status_code(201))
}

// --- POST /api/contribute ---

pub fn join_contribute(
    body: &[u8],
    remote_ip: IpAddr,
    state: &AppState,
) -> Result<Response, ApiError> {
    let payload: ContributePayload =
        serde_json::from_slice(body).map_err(|err| ApiError::InvalidRequest(err.to_string()))?;

    if !payload.honeypot.is_empty() {
        return Ok(Response::json(&OkResponse { ok: true }).with_status_code(201));
    }

    let name = sanitize(&payload.name);
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::InvalidRequest("invalid name".into()));
    }
    let email = sanitize(&payload.email);
    if !is_valid_email(&email) {
        return Err(ApiError::InvalidRequest("invalid email".into()));
    }
    let skills = sanitize(&payload.skills);
    if skills.is_empty() || skills.chars().count() > 300 {
        return Err(ApiError::InvalidRequest("invalid skills".into()));
    }
    let portfolio = sanitize(&payload.portfolio);
    if portfolio.chars().count() > 200 {
        return Err(ApiError::InvalidRequest("invalid portfolio".into()));
    }

    if !state.contribute_requests.check(remote_ip) {
        return Err(ApiError::RateLimit);
    }

    let conn = db()?;
    if email_exists(&conn, "contributors", &email)? {
        return Ok(Response::json(&OkResponse { ok: true }).with_status_code(201));
    }

    conn.execute(
        "INSERT INTO contributors (created_at, name, email, skills, portfolio) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![now_unix(), name, email, skills, portfolio],
    )?;

    mailer::send_html_email(
        email.clone(),
        "¡Gracias por querer sumarte a Xindeler!".to_owned(),
        mailer::contribute_user_html(&name),
    );

    if let Some(owner_email) = crate::config::get().owner_email.clone() {
        mailer::send_html_email(
            owner_email,
            format!("[Xindeler] Nuevo colaborador — {name}"),
            mailer::contribute_owner_html(&name, &email, &skills, &portfolio, &now_iso()),
        );
    }

    Ok(Response::json(&OkResponse { ok: true }).with_status_code(201))
}

#[cfg(test)]
mod tests {
    use super::{is_valid_email, sanitize};

    #[test]
    fn sanitize_trims_and_strips_control_chars() {
        assert_eq!(sanitize("  hello\r\nworld  "), "helloworld");
    }

    #[test]
    fn sanitize_guards_csv_formula_injection() {
        assert_eq!(sanitize("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(sanitize("+1"), "'+1");
        assert_eq!(sanitize("@mention"), "'@mention");
        assert_eq!(sanitize("normal name"), "normal name");
    }

    #[test]
    fn email_validation() {
        assert!(is_valid_email("user@example.com"));
        assert!(!is_valid_email("no-at-sign"));
        assert!(!is_valid_email("two@at@signs.com"));
        assert!(!is_valid_email("user@nodot"));
        assert!(!is_valid_email(""));
        assert!(!is_valid_email("@example.com"));
        assert!(!is_valid_email("user@"));
    }
}
