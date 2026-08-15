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
- Hardening del repo GitHub (ruleset, secret scanning, vulnerability alerts, `SECURITY.md`) —
  Dependabot se probó y se sacó a pedido de Matías (abría PRs y mandaba mails sin aportar valor
  acá); `cargo audit`/`cargo deny` en CI siguen cubriendo vulnerabilidades conocidas
- Skills (`xindeler-web-api-dev`, `xindeler-web-api-architect`) y agentes reviewers
  (`xindeler-web-api-security-reviewer`, `xindeler-web-api-quality-reviewer`)

✅ **Fase 1 — Paridad funcional (código)** (esta pasada)
- Tablas `waitlist`/`contributors` (SQLite + WAL, `COLLATE NOCASE` para dedup case-insensitive),
  migraciones refinery
- `GET /api/status` (probe TCP cacheado 30s) y `GET /api/waitlist/count` (cacheado 60s,
  invalidado al insertar)
- `POST /api/waitlist` y `POST /api/contribute` — rate limit **antes** del dedup (bug #1
  corregido), HTML de emails escapado + `portfolio` solo se linkea con esquema seguro (bug #3
  corregido), `RateLimiter` con TTL real (bug #4 corregido)
- Subcomando `digest` (port de `monthly-digest.py`, apunta a `xindeler.com`) y subcomando
  `migrate-csv` (idempotente, probado con fixtures)
- 38 tests en verde (28 unitarios + 10 de integración `TestServer` contra el binario real)
- `WEB_API_TRUSTED_PROXIES` + resolución de IP real vía `X-Forwarded-For` (nuestro nginx no manda
  `X-Real-IP` como `xindeler-auth` — hubo que adaptar el patrón, no copiarlo literal)

**Pendiente de Fase 1 — requiere producción real, no se hizo esta pasada:**
- Correr `migrate-csv` contra los CSV reales del VPS
- Deploy en puerto nuevo, smoke-test directo, corte de `proxy_pass` en nginx, apagar
  `xindeler-waitlist.service` — bloqueado a propósito hasta que Matías confirme explícitamente

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
