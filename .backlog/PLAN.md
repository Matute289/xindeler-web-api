# Plan — xindeler-web-api

## Estado actualizado al 2026-08-19 — Fase F implementada, no mergeada

Fase E (proxy de 2FA) cerrada y en producción desde 2026-08-15 (ver `.backlog/README.md`). **Fase
F — proxy de personajes** (relayada desde `xindeler-new-horizon` NH-79): implementación completa
en la rama `feat/fase-f-character-proxy`, esperando revisión de PR. Orden real de dispatch
cumplido: `xindeler-new-horizon` (PR #197, mergeado) → `xindeler-auth` (PR #39, N-01, todavía sin
mergear — este repo pineó `xindeler-auth-common` directo a esa rama de feature de puente, pendiente
de re-pinear a `main` cuando ese PR mergee) → este repo.

- `AuthClient::issue_character_access_token(uuid)` — nueva credencial `WEB_API_SERVICE_TOKEN`,
  nunca `AUTH_SERVICE_TOKEN` (que `xindeler-auth` rechaza explícitamente para ese endpoint).
- `game_server_client.rs` (nuevo, no reusa `AuthClient`): `list_characters`/`rename_character`
  contra `player_api/v1` del game server — sus rechazos son texto plano, no el envelope JSON
  `{code, message, request_id}` de `xindeler-auth`, así que el cliente y el mapeo de errores lo
  reflejan (`GameServerClientError::Rejected { status, message }`, sin `code`).
- `GET /api/account/characters` y `POST /api/account/characters/{character_id}/rename` en
  `account.rs`/`web.rs` — mismo patrón `resolve_session` → token fresco por llamada (nunca
  cacheado, TTL 60s de un solo uso) → reenvío al game server.
- Guard nuevo en `config.rs`: el arranque falla si `AUTH_SERVICE_TOKEN` y `WEB_API_SERVICE_TOKEN`
  coinciden (más allá de lo que pedía el spec).
- 84 tests en verde (38 unitarios + 46 de integración, 7 nuevos de Fase F, incluido un fake game
  server reusando la misma infraestructura `FakeAuthServer` ya existente).

**Riesgo sin resolver, no bloqueante para el código**: la topología real de red entre este servicio
y el game server (loopback-only por decisión de NH-75 en ese repo) sigue sin confirmar — el game
server todavía no está deployado en ningún lado. Bloquea el deploy real de esta fase, no el merge
del PR.

**Bloqueador real de seguridad para el deploy (hallazgo de la revisión de seguridad,
2026-08-19), separado del riesgo de topología de arriba**: el código de `player_api/v1` que ya está
mergeado en `xindeler-new-horizon` (PR #197, Fase 1 de NH-79) usa un stub de auth deliberado y
documentado — confía en el valor crudo del header `Authorization: Bearer` como si fuera el `uuid`,
sin canjearlo contra `xindeler-auth` (`/verify-character-access-token`, que solo existe en el PR
#39 de `xindeler-auth`, todavía sin mergear). Esto es exactamente lo que la Fase 2 de NH-79 ("Wire
the real auth scheme") tiene que reemplazar, y ya está gateado ahí mismo detrás de una env var de
opt-in obligatoria (`XINDELER_PLAYER_API_DEBUG_AUTH=1`) para que no pueda arrancar por accidente en
producción. Consecuencia concreta: **el código de este repo (Fase F) es correcto y está listo para
mergear, pero no puede deployarse a producción con datos reales de jugadores hasta que la Fase 2 de
NH-79 mergee en `xindeler-new-horizon`** — antes de eso, cualquiera con acceso directo al puerto
loopback del game server podría listar/renombrar personajes de cualquier cuenta con un
`Authorization: Bearer <cualquier-string>`. El límite de exposición real hoy es que ese puerto es
loopback-only y el game server ni siquiera está deployado todavía — pero no depender de eso es
justamente el motivo de la Fase 2. Bloqueador a trackear en el backlog de `xindeler-new-horizon`
(NH-79 Fase 2), no algo que este repo pueda resolver unilateralmente.

**Actualización 2026-08-20 — bloqueador resuelto.** La Fase 2 de NH-79 mergeó en
`xindeler-new-horizon` ([PR #198](https://github.com/Matute289/xindeler-new-horizon/pull/198)):
`player_api/v1` reemplazó el stub por la verificación real contra `xindeler-auth`, y
`XINDELER_PLAYER_API_DEBUG_AUTH` se eliminó por completo del código — ya no queda ningún modo
inseguro que gatear. `xindeler-auth` N-01 ([PR #39](https://github.com/Matute289/xindeler-auth/pull/39)/[#40](https://github.com/Matute289/xindeler-auth/pull/40)) también mergeó, y este repo re-pineó
su dependencia a `main` real ([PR #15](https://github.com/Matute289/xindeler-web-api/pull/15)). El
riesgo de topología de la línea 26-29 sigue sin resolver.

**Validado end-to-end en local, mismo día.** Los tres servicios (este + `xindeler-auth` +
`xindeler-new-horizon`) corriendo juntos, con una cuenta de prueba real: se creó un personaje por
el protocolo real del juego (no un insert a mano), se lo listó y renombró a través de
`GET/POST /api/account/characters*` de este repo, se probaron los casos de rechazo (nombre vacío →
`422` sin tocar la red, caracteres inválidos → `409` reenviado con el mensaje real del game server,
personaje inexistente → `409`), y se lo eliminó — todo con la cadena de auth real (sin el stub de
debug). El bug encontrado en el camino (`EventBus<DismissSummonEvent>` sin registrar, crasheaba
el game server al arrancar) era ajeno a esta Fase F, ya se arregló en
[PR #199](https://github.com/Matute289/xindeler-new-horizon/pull/199) de `xindeler-new-horizon`.
Único pendiente real: decidir la topología de deploy (`xindeler-new-horizon` todavía no está
deployado en ningún lado).

**Actualización 2026-08-20 (más tarde el mismo día) — topología resuelta.** `xindeler-new-horizon`
ya no está sin deployar: una verificación de infraestructura confirmó `xindeler-server-cli`
corriendo en el mismo VPS que este servicio — `0.0.0.0:14004` (protocolo del juego, público) y
`127.0.0.1:14005` (`player_api/v1`, loopback como exige NH-75). Mismo host, loopback intacto: el
riesgo de topología de la línea 26-29 queda cerrado. `xindeler-new-horizon` también mergeó su
primer deploy script (BL-83, [PR #203](https://github.com/Matute289/xindeler-new-horizon/pull/203),
modelado en el `deploy/deploy.sh` de este repo) — pero, según ese mismo PR, todavía no se corrió
contra producción, y el proceso visto en el VPS corre como background process suelto, no vía
systemd todavía. Qué revisión de `development` tiene el binario que está realmente corriendo (y si
ya incluye NH-79) no se pudo confirmar en esta pasada — no asumir que el feature ya funciona de
punta a punta en producción sin volver a chequear `xindeler-new-horizon`'s BL-83.

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
