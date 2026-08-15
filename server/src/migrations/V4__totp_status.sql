-- Derived state, not a source of truth: xindeler-auth exposes no "is TOTP
-- confirmed" endpoint, only the six mutation endpoints of Fase L. A row's
-- presence here is entirely inferred from what this service already
-- observes — a login that resolved through /login/2fa, or a successful
-- 2fa/confirm proxied through this service — never queried from
-- xindeler-auth directly. See 005's backlog for why: no new contract is
-- requested from that repo for this.
CREATE TABLE totp_status (
    uuid         TEXT PRIMARY KEY,
    confirmed_at INTEGER NOT NULL
);
