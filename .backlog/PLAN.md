# Plan — xindeler-web-api

## Estado actualizado al 2026-08-15 — backlog 007 completo, en producción

Las cuatro fases están cerradas y el corte de producción real ya se ejecutó: `xindeler-web-api`
sirve `xindeler.com/api/*` desde el VPS (puerto 8020, systemd), `xindeler-waitlist.service`
(Python) está parado y deshabilitado. Detalle completo del corte en `.backlog/README.md`, sección
B-05. Lo único que queda abierto es lo que ya estaba fuera de alcance a propósito:
`/api/session/login/2fa` (bloqueado hasta que Fase L de `xindeler-auth` exista) y la UI de "sesión
activa" en la landing (navbar/logout — pertenece a la pantalla de cuenta, 005, todavía sin
diseñar).

## Estado al 2026-08-14 (histórico, previo al corte de producción)

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

✅ **Fase 1 — B-04/B-05, corte de producción real (2026-08-15)**
- `migrate-csv` corrido contra los datos reales del VPS: 5 filas de waitlist + 1 de contributors
  migradas (6 filas totales en el CSV, 1 duplicado detectado); idempotencia confirmada re-corriendo
- Deploy en `/opt/xindeler-web-api/` (puerto 8020), systemd hardened igual que
  `xindeler-auth.service`, smoke-test directo contra el puerto (sin nginx) antes de tocar nada real
- Bug real encontrado en el smoke-test, no en tests: `xindeler-auth` responde `400` (no `401`) a
  login inválido — `map_sign_in_error` lo colapsaba a `502`. Corregido y mergeado (`#9`) antes de
  seguir — ver `SPEC.md`
- Corte de nginx (`/api/*` de `127.0.0.1:8010` a `127.0.0.1:8020`, con backup + `nginx -t` +
  rollback automático) + fix del bug de `/api/waitlist/count` bloqueado, en el mismo cambio
- Verificación end-to-end contra `xindeler.com` real, `xindeler-waitlist.service` parado y
  deshabilitado (no borrado)

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

✅ **Fase 2 — Proxy de cuenta (C-02)** (esta pasada)
- `authclient.rs` suma `check_username`/`change_username`/`change_password`/`delete_account` —
  los cuatro son endpoints públicos de `xindeler-auth` (sin service-token, mismo tier
  rate-limited-por-IP que `/generate_token`)
- `account.rs` nuevo: `GET /api/account/check-username` (sin sesión, igual que hoy),
  `POST /api/account/{change-username,change-password,delete}` (requieren sesión, usan el
  `username` de la sesión — nunca uno que mande el cliente)
- Toda mutación exitosa **revoca todas las sesiones de la cuenta** en el mismo request
  (hallazgo 8 de la tarea 007) — nunca solo la sesión actual, nunca en background. Si
  `xindeler-auth` rechaza el cambio, la sesión sigue viva
- `session.rs` refactorizado: `resolve_session`/`revoke_all_sessions`/`clear_cookie` ahora
  `pub(crate)`, reusados por `account.rs` en vez de duplicar la lógica de leer/hashear la cookie
- 50 tests en verde (32 unitarios + 18 de integración, 8 nuevos de C-02)

✅ **Fase 2 — Reroute forgot/reset-password (C-03)**
- `authclient.rs` suma `forgot_password`/`reset_password`; `account.rs` suma
  `POST /api/account/{forgot-password,reset-password}` (sin sesión, `forgot-password` siempre
  `200 {ok:true}` anti-enumeración; `reset-password` revoca la sesión del llamador si trae una,
  limitación conocida y documentada de no poder revocar *todas* las sesiones — `xindeler-auth`'s
  `reset_password` no expone el `uuid`)
- `xindeler-web-landing`: `ForgotPasswordPage.jsx`/`ResetPasswordPage.jsx` pasan a llamar acá en
  vez de `auth.xindeler.com` directo
- 56 tests en verde (32 unitarios + 24 de integración, 6 nuevos de C-03)

**Pendiente de Fase 2:**
- `/api/session/login/2fa` — bloqueado hasta que Fase L (2FA) de `xindeler-auth` exista (fuera de
  alcance del backlog 007, no es un pendiente de esta pasada)

✅ **Fase 3 — Frontend consume la sesión (D-01, D-02)**
- D-01: env vars + proxy de Vite en `xindeler-web-landing` (`vite.config.js` suma
  `server.proxy['/api']`, target configurable vía `VITE_API_PROXY_TARGET`); las 5 llamadas
  same-origin pasan de URL absoluta a ruta relativa `/api/...`
- D-02: `AuthModal.jsx` pasa su login de `auth.xindeler.com/generate_token` (descartaba el
  resultado) a `POST /api/session/login` (establece una sesión real). Requirió primero arreglar
  `session::login` para reenviar el `403 EMAIL_VERIFICATION_REQUIRED` tal cual (si no, el modal de
  cuentas legacy se hubiera roto en silencio) — ver hallazgo/fix en `.backlog/README.md`
- Deliberadamente fuera de alcance: ningún indicador de "sesión activa" en la UI (navbar, logout) —
  pertenece a la pantalla de cuenta (005), todavía sin diseñar

## Orden de prioridad — completado (2026-08-15)

Decidido por Matías (2026-08-15): terminar todo el backlog de código primero, deploy al final —
así el corte de producción sale con todo andando de una, no en pedazos. Las cuatro fases se
completaron en ese orden y el corte de producción (B-05) se ejecutó al final, con confirmación
explícita de Matías en cada paso que tocaba el VPS real (instalar systemd, tocar nginx, apagar el
servicio Python).
