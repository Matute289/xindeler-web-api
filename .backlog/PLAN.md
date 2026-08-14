# Plan — xindeler-web-api

## Estado actualizado al 2026-08-14

✅ **Fase 0 — Repo y fundaciones** (esta pasada)
- Repo `Matute289/xindeler-web-api` creado, público
- Cargo workspace (`common`/`server`), seam HTTP framework-agnóstico (`http/mod.rs`+`axum.rs`)
  copiado de `xindeler-auth`, router a mano, `error.rs`/`config.rs` con el mismo patrón
- `GET /ping` funcionando de punta a punta (build + test + clippy + fmt en verde, binario
  levantado y probado en local)
- CI (`check.yml`) + `scripts/check-{unsafe-policy,docs,docker-context}.sh` — invocados desde CI,
  a diferencia de `xindeler-auth` donde quedaron sueltos
- `Dockerfile` + `docker-compose.yml` hardened (usuario no-root, `read_only`, `cap_drop: ALL`) —
  paridad de dev/testing; producción sigue siendo systemd
- `CLAUDE.md` == `AGENTS.md`, `.backlog/{README,SPEC,PLAN}.md`
- Hardening del repo GitHub (ruleset, secret scanning, dependabot, `SECURITY.md`)
- Skills (`xindeler-web-api-dev`, `xindeler-web-api-architect`) y agentes reviewers
  (`xindeler-web-api-security-reviewer`, `xindeler-web-api-quality-reviewer`)

## Próximos pasos

### Fase 1 — Paridad funcional

Portar `waitlist`/`contribute`/`status`/`count` a SQLite + Rust, corrigiendo los bugs relevados
del Python actual (dedup antes de rate-limit, `count` roto en nginx, HTML sin escapar, rate
limiter que no evictea). Portar `monthly-digest.py`. Migrar los 7 registros de los CSV. Deploy en
puerto nuevo, smoke-test directo, corte de nginx, apagar `xindeler-waitlist.service`.

Shape esperado de las tablas nuevas:

```sql
CREATE TABLE waitlist (
    id INTEGER PRIMARY KEY,
    created_at INTEGER NOT NULL,
    name TEXT NOT NULL,
    email TEXT NOT NULL UNIQUE,
    platform TEXT NOT NULL,
    source TEXT NOT NULL
);

CREATE TABLE contributors (
    id INTEGER PRIMARY KEY,
    created_at INTEGER NOT NULL,
    name TEXT NOT NULL,
    email TEXT NOT NULL UNIQUE,
    skills TEXT NOT NULL,
    portfolio TEXT NOT NULL DEFAULT ''
);
```

### Fase 2 — Sesión web

Tabla `sessions`, endpoints `/api/session/*` y `/api/account/*` proxeando a `xindeler-auth` vía
`xindeler-authc`, reroute de `ForgotPasswordPage`/`ResetPasswordPage` del frontend.

### Fase 3 — Frontend consume la sesión

Env vars + proxy de Vite en `xindeler-web-landing`, `AuthModal.jsx` deja de descartar el
resultado del login. Base concreta para 005 (pantalla de cuenta) y 006 (personajes).

## Orden de prioridad actual

1. Fase 1 (paridad funcional) — bloquea el corte de producción, sin ella no hay nada que ganar
   con este repo todavía.
2. Fase 2 (sesión) — el objetivo real de todo esto, desbloquea 005/006 en `xindeler-web-landing`.
3. Fase 3 (frontend) — cierra el círculo.
