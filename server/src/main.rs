#![forbid(unsafe_code)]

mod account;
mod authclient;
mod cache;
mod config;
mod db;
mod digest;
mod error;
mod http;
mod mailer;
mod migrate_csv;
mod ratelimit;
mod session;
mod state;
mod waitlist;
mod web;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // CLI subcommands: invoked by a systemd timer (`digest`, monthly) or
    // manually once (`migrate-csv`, the CSV→SQLite cutover) — neither
    // starts the HTTP server.
    match args.get(1).map(String::as_str) {
        Some("digest") => digest::run(),
        Some("migrate-csv") => {
            let waitlist_csv = args.get(2).unwrap_or_else(|| {
                panic!(
                    "usage: {} migrate-csv <waitlist.csv> <contributors.csv>",
                    args[0]
                )
            });
            let contributors_csv = args.get(3).unwrap_or_else(|| {
                panic!(
                    "usage: {} migrate-csv <waitlist.csv> <contributors.csv>",
                    args[0]
                )
            });
            migrate_csv::run(
                std::path::Path::new(waitlist_csv),
                std::path::Path::new(contributors_csv),
            );
        }
        _ => {}
    }

    env_logger::init();
    config::initialize().expect("Invalid web-api server configuration");
    mailer::initialize().expect("Invalid mail configuration");
    {
        let mut db = db::db().expect("Failed to create database connection for migrations");
        db::init_db(&mut db).expect("Failed to initialize database");
    }
    web::start();
}
