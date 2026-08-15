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

✅ **Fase 2 — Sesión web (C-01)** (esta pasada)
- Tabla `sessions` (`session_id` = SHA-256 del cookie crudo, índice por `uuid`)
- `POST /api/session/login`, `GET /api/session/me`, `POST /api/session/logout`
- `authclient.rs`: cliente HTTP propio hacia `xindeler-auth` — **decisión de arquitectura
  importante**: depende de `xindeler-auth-common` (solo tipos de wire, git a repo privado), nunca
  de `xindeler-authc`. Ese crate calcula `net_prehash()` internamente en `sign_in()`/`register()`;
  como este servicio ya recibe el `password_prehash` calculado por el frontend, usar `authc`
  hubiera hasheado dos veces y roto todos los logins. Se detectó leyendo el código fuente real de
  `authc`, no por documentación.
- Dependencia privada resuelta con el mismo patrón que `xindeler-new-horizon`/`xindeler-zuul`:
  deploy key de solo lectura (`AUTH_REPO_SSH_KEY`) + `.cargo/config.toml` con
  `git-fetch-with-cli` (el SSH propio de Cargo falla ssh-agent en algunos entornos)
- Cookie `HttpOnly`+`Secure`+`SameSite=Lax`, TTL 7 días absoluto, sin renovación deslizante
- 44 tests en verde (32 unitarios + 12 de integración, incluidos 2 nuevos contra un
  `xindeler-auth` falso hecho a mano)

**Pendiente de Fase 2 — no se hizo esta pasada:**
- C-02: proxy `/api/account/*` (`change_username`, `change_password`, `delete_account`,
  `check_username`) — `authclient.rs` ya tiene la base, falta agregar estos métodos y los
  handlers
- C-03: reroute de `ForgotPasswordPage`/`ResetPasswordPage` de `xindeler-web-landing`
- `/api/session/login/2fa` — bloqueado hasta que Fase L (2FA) de `xindeler-auth` exista

### Fase 3 — Frontend consume la sesión

Env vars + proxy de Vite en `xindeler-web-landing`, `AuthModal.jsx` deja de descartar el
resultado del login. Base concreta para 005 (pantalla de cuenta) y 006 (personajes).

## Orden de prioridad actual

1. Fase 1 B-05 (corte de producción) — bloqueado a propósito hasta confirmación explícita de
   Matías, toca datos reales de usuarios.
2. Fase 2 C-02/C-03 (proxy de cuenta + reroute) — completa lo que hace falta para que 005/006 de
   `xindeler-web-landing` tengan de verdad todo lo que necesitan.
3. Fase 3 (frontend) — cierra el círculo.
