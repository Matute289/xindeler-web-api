#![forbid(unsafe_code)]

//! Wire types shared between crates in this workspace.
//!
//! Empty for now (Fase 0 — no business logic yet). Fase 1 adds the
//! waitlist/contribute/status payloads; Fase 2 adds the session/account
//! payloads. Keeping this crate separate from `server` from the start means
//! a future consumer (e.g. a test harness, or a Rust client) never needs to
//! pull in axum/rusqlite just to see the wire shapes.
