//! One-shot migration of the Python service's data — `waitlist.csv` /
//! `contributors.csv` — into the SQLite tables this service reads from.
//!
//! CLI subcommand: `xindeler-web-api-server migrate-csv <waitlist.csv> <contributors.csv>`.
//! Idempotent: re-running it just skips rows whose email already exists
//! (the `UNIQUE ... COLLATE NOCASE` index does the dedup), so it's safe to
//! run again if the first attempt was interrupted.

use crate::db::db;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, ErrorCode};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct WaitlistRow {
    timestamp: String,
    name: String,
    email: String,
    platform: String,
    source: String,
}

#[derive(Deserialize)]
struct ContributorRow {
    timestamp: String,
    name: String,
    email: String,
    skills: String,
    #[serde(default)]
    portfolio: String,
}

/// Falls back to "now" on an unparseable timestamp rather than aborting the
/// whole migration over one bad row — the Python CSVs are hand-written by a
/// service that's been running for weeks, so a malformed row is more likely
/// than a corrupt file.
fn parse_timestamp(value: &str) -> i64 {
    match DateTime::parse_from_rfc3339(value.trim()) {
        Ok(dt) => dt.timestamp(),
        Err(_) => {
            eprintln!("[migrate-csv] unparseable timestamp {value:?}, using current time");
            Utc::now().timestamp()
        }
    }
}

fn is_duplicate(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(e, _) if e.code == ErrorCode::ConstraintViolation
    )
}

fn migrate_waitlist(conn: &Connection, path: &Path) -> (usize, usize) {
    let mut migrated = 0;
    let mut skipped = 0;
    let mut reader = match csv::Reader::from_path(path) {
        Ok(reader) => reader,
        Err(err) => {
            eprintln!("[migrate-csv] {path:?} not readable, skipping: {err}");
            return (0, 0);
        }
    };
    for result in reader.deserialize::<WaitlistRow>() {
        let row = result.expect("malformed row in waitlist.csv");
        let created_at = parse_timestamp(&row.timestamp);
        let outcome = conn.execute(
            "INSERT INTO waitlist (created_at, name, email, platform, source) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![created_at, row.name, row.email, row.platform, row.source],
        );
        match outcome {
            Ok(_) => migrated += 1,
            Err(ref err) if is_duplicate(err) => {
                eprintln!(
                    "[migrate-csv] waitlist: skipping duplicate email {}",
                    row.email
                );
                skipped += 1;
            }
            Err(err) => panic!("failed to insert waitlist row for {}: {err}", row.email),
        }
    }
    (migrated, skipped)
}

fn migrate_contributors(conn: &Connection, path: &Path) -> (usize, usize) {
    let mut migrated = 0;
    let mut skipped = 0;
    let mut reader = match csv::Reader::from_path(path) {
        Ok(reader) => reader,
        Err(err) => {
            eprintln!("[migrate-csv] {path:?} not readable, skipping: {err}");
            return (0, 0);
        }
    };
    for result in reader.deserialize::<ContributorRow>() {
        let row = result.expect("malformed row in contributors.csv");
        let created_at = parse_timestamp(&row.timestamp);
        let outcome = conn.execute(
            "INSERT INTO contributors (created_at, name, email, skills, portfolio) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![created_at, row.name, row.email, row.skills, row.portfolio],
        );
        match outcome {
            Ok(_) => migrated += 1,
            Err(ref err) if is_duplicate(err) => {
                eprintln!(
                    "[migrate-csv] contributors: skipping duplicate email {}",
                    row.email
                );
                skipped += 1;
            }
            Err(err) => panic!("failed to insert contributor row for {}: {err}", row.email),
        }
    }
    (migrated, skipped)
}

pub fn run(waitlist_csv: &Path, contributors_csv: &Path) -> ! {
    env_logger::init();
    crate::config::initialize().expect("Invalid web-api server configuration");

    let mut conn = db().expect("Failed to open database");
    crate::db::init_db(&mut conn).expect("Failed to initialize database");

    let (w_migrated, w_skipped) = migrate_waitlist(&conn, waitlist_csv);
    let (c_migrated, c_skipped) = migrate_contributors(&conn, contributors_csv);

    println!(
        "[migrate-csv] waitlist: {w_migrated} migrated, {w_skipped} skipped (already present)"
    );
    println!(
        "[migrate-csv] contributors: {c_migrated} migrated, {c_skipped} skipped (already present)"
    );
    std::process::exit(0);
}
