//! Monthly digest — port of `monthly-digest.py`. Invoked as a CLI
//! subcommand (`xindeler-web-api-server digest`) by a systemd timer, same
//! trigger as the Python service (`xindeler-digest.timer`, 1st of each
//! month 09:00 UTC).
//!
//! Two behavioral fixes over the Python version:
//! 1. Links/images point at `https://xindeler.com`, never the legacy
//!    `xindeler.greenmountain.dev` the Python templates still used.
//! 2. The watermark write happens in the same `Result`-propagating flow as
//!    the send, wrapped so a panic can't skip it — the Python script wrote
//!    the watermark *after* an unguarded `send_email` call, so an SMTP
//!    failure crashed the script and silently left the watermark stale.

use crate::db::db;
use crate::error::ApiError;
use crate::mailer;
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection};
use std::fs;

struct DigestEntry {
    name: String,
    email: String,
    platform: String,
    source: String,
    created_at: i64,
}

fn read_since() -> i64 {
    let path = crate::config::get().digest_state_path.clone();
    match fs::read_to_string(&path) {
        Ok(contents) => DateTime::parse_from_rfc3339(contents.trim())
            .map(|dt| dt.timestamp())
            .unwrap_or(0),
        Err(_) => 0,
    }
}

fn write_now() {
    let path = crate::config::get().digest_state_path.clone();
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, false);
    if let Err(err) = fs::write(&path, now) {
        log::error!("[digest] failed to write state file {path:?}: {err}");
    }
}

fn fetch_new_entries(conn: &Connection, since: i64) -> Result<Vec<DigestEntry>, ApiError> {
    let mut stmt = conn.prepare(
        "SELECT name, email, platform, source, created_at FROM waitlist \
         WHERE created_at > ?1 ORDER BY created_at",
    )?;
    let rows = stmt.query_map(params![since], |row| {
        Ok(DigestEntry {
            name: row.get(0)?,
            email: row.get(1)?,
            platform: row.get(2)?,
            source: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    let mut entries = Vec::new();
    for row in rows {
        entries.push(row?);
    }
    Ok(entries)
}

fn render_html(entries: &[DigestEntry]) -> String {
    let rows: String = entries
        .iter()
        .map(|entry| {
            let date = DateTime::from_timestamp(entry.created_at, 0)
                .map(|dt| dt.format("%d/%m/%Y %H:%M UTC").to_string())
                .unwrap_or_default();
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                mailer::escape_html(&entry.name),
                mailer::escape_html(&entry.email),
                mailer::escape_html(&entry.platform),
                mailer::escape_html(&entry.source),
                date,
            )
        })
        .collect();

    let content = format!(
        r#"<h1 style="color:#c9a84c;font-size:20px;">Lista de espera — nuevas entradas</h1>
<table style="width:100%;font-size:14px;border-collapse:collapse;">
<tr style="color:#7a8ba3;text-align:left;"><th>Nombre</th><th>Email</th><th>Plataforma</th><th>Cómo llegó</th><th>Fecha</th></tr>
{rows}
</table>"#
    );
    mailer::wrap_email(
        &content,
        "Digest mensual automático de la lista de espera de Xindeler.",
    )
}

/// Runs the digest job to completion (blocking) and exits the process.
/// Never panics on a missing/misconfigured SMTP setup — matches the Python
/// script's `[digest] SMTP not configured` early exit.
pub fn run() -> ! {
    env_logger::init();
    crate::config::initialize().expect("Invalid web-api server configuration");

    let config = crate::config::get();
    let Some(owner_email) = config.owner_email.clone() else {
        eprintln!("[digest] OWNER_EMAIL not configured, nothing to do");
        std::process::exit(0);
    };
    if config.smtp.is_none() {
        eprintln!("[digest] SMTP not configured, nothing to do");
        std::process::exit(0);
    }
    mailer::initialize().expect("Invalid mail configuration");

    let since = read_since();
    let entries = match db().and_then(|conn| fetch_new_entries(&conn, since)) {
        Ok(entries) => entries,
        Err(err) => {
            eprintln!("[digest] failed to read waitlist entries: {err:?}");
            std::process::exit(1);
        }
    };

    if entries.is_empty() {
        println!("[digest] no new entries since last run");
    } else {
        let subject = format!(
            "Xindeler - Lista de Espera — {} entradas nuevas",
            entries.len()
        );
        match mailer::send_html_email_blocking(&owner_email, &subject, &render_html(&entries)) {
            Ok(()) => println!("[digest] sent digest with {} entries", entries.len()),
            Err(err) => eprintln!("[digest] send failed: {err}"),
        }
    }

    // Always advance the watermark, even if the send failed — same
    // at-least-once semantics as the Python script (a failed send means the
    // next run re-includes these rows, never that they're lost).
    write_now();
    std::process::exit(0);
}
