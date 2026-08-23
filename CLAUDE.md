# xindeler-web-api — CLAUDE.md

## Proyecto

Backend propio de la landing de **Xindeler** (`xindeler-web-landing`): lista de espera,
contribuidores, estado del servidor de juego, y sesión web autenticada.

- URL de producción (futura, tras el corte de Fase 1): `https://xindeler.com/api/*`
- Repo GitHub: `Matute289/xindeler-web-api` (público)
- VPS: `greenmountain.dev` (`216.238.126.97`) — mismo VPS que `xindeler-auth` y
  `xindeler-new-horizon`
- **Reemplaza**: `/srv/xindeler/waitlist-api/main.py` (FastAPI en Python, nunca versionado en
  git, persistencia en CSV) — ver `.backlog/SPEC.md` para el detalle de la migración

## Contexto de diseño

Existe por la tarea 007 del backlog de `xindeler-web-landing` ("sesión web autenticada"):
`xindeler-auth` es stateless por diseño (su `AuthToken` dura 15s y se consume una sola vez) y
además es cross-origin respecto de la landing, así que **no puede** setear una cookie de sesión
utilizable por el frontend. La sesión tiene que vivir en un backend same-origin
(`xindeler.com/api/*`) — ese es este repo.

**`netPrehash` es contrato de cable y nunca se toca acá.** El frontend calcula
`Argon2i(password, salt=FxHash64(password))` en JS (`src/lib/netPrehash.js` en
`xindeler-web-landing`) antes de mandar la contraseña — este servicio recibe ese string ya
hasheado y lo reenvía tal cual a `xindeler-auth`, nunca lo recalcula ni lo valida.

## Stack

- **Rust** — mismo lenguaje que `xindeler-auth` y `xindeler-new-horizon`; cero precedente Go en
  el ecosistema.
- **axum + tokio**, handlers síncronos corridos en `spawn_blocking` — mismo patrón que
  `xindeler-auth` (ver su `server/src/http/axum.rs`): mantiene el trabajo bloqueante (SQLite,
  llamadas a `xindeler-auth`) fuera del reactor async sin escribir ningún handler como `async fn`.
- **rusqlite + refinery** — SQLite sin pool, conexión por request, mismo patrón que
  `xindeler-auth` (`journal_mode=WAL`, migraciones secuenciales embebidas).
- **`xindeler-auth-common`** (git, repo privado) para los tipos de wire hacia `xindeler-auth` —
  **nunca `xindeler-authc`**: sus `sign_in()`/`register()` calculan `net_prehash()` sobre lo que
  reciben, y este servicio siempre recibe el `password_prehash` ya calculado por el frontend.
  Usarlo hashearía dos veces y rompería todos los logins. Ver `server/src/authclient.rs`.

## Dependencia privada — `xindeler-auth-common`

Vive en `Matute289/xindeler-auth` (privado); este repo es público. Mismo patrón que
`xindeler-new-horizon`/`xindeler-zuul` para la misma dependencia:
- `.cargo/config.toml` fuerza `git-fetch-with-cli` (el cliente SSH propio de Cargo falla
  ssh-agent en algunos entornos).
- CI usa un deploy key de **solo lectura** (`AUTH_REPO_SSH_KEY`, secret de este repo) agregado a
  `xindeler-auth` — nunca push access.
- El `rev` en `server/Cargo.toml` se bumpea a mano cuando `xindeler-auth` publica algo que hace
  falta — nunca sigue `main` automáticamente.

## Estructura de crates

```
common/   → tipos de wire compartidos (requests/responses de /api/*)
server/   → el binario
  src/
    http/{mod.rs,axum.rs}  → seam framework-agnóstico, único lugar que conoce axum
    web.rs                 → router a mano (match method+path), NO el router de axum
    error.rs                → ApiError interno + status_code() + public_fields() → {code, message, request_id}
    config.rs               → OnceLock<AppConfig>, from_iter, validación de rangos
    state.rs
    db.rs                    → conexión SQLite + migraciones refinery
    cache.rs                 → TtlCache genérico (status/count)
    ratelimit.rs              → RateLimiter con TTL real, por IP
    waitlist.rs               → lógica de waitlist/contribute/status/count
    mailer.rs                 → SMTP, templates HTML escapados
    digest.rs                 → subcomando CLI, digest mensual
    migrate_csv.rs             → subcomando CLI, migración one-shot CSV→SQLite
    authclient.rs              → cliente HTTP propio hacia xindeler-auth (NO xindeler-authc)
    session.rs                 → login/login_2fa/logout/me + resolve_session/revoke_all_sessions (reusados por account.rs)
    account.rs                  → proxy /api/account/* (check-username/change-username/change-password/delete/forgot-password/reset-password/2fa/*)
    totp_status.rs              → estado derivado de 2FA por uuid (Fase L no expone ningún endpoint de estado)
```

## Esquema de base de datos

- **Fase 1**: `waitlist`, `contributors` (migradas desde los CSV del Python actual — 7 filas
  totales).
- **Fase 2**: `sessions` (`session_id` = `SHA-256(cookie)`, `uuid`, `username`, `created_at`,
  `expires_at`, `revoked_at`) — índice por `uuid` para poder revocar todas las sesiones de una
  cuenta en el mismo request que un cambio de contraseña (hallazgo 8 de la tarea 007).
- **Fase 2 (005)**: `totp_status` (`uuid` PK, `confirmed_at`) — estado de 2FA derivado, nunca
  consultado a `xindeler-auth` (Fase L no expone ningún endpoint de "estado de TOTP"). Una fila
  existe solo si la cuenta tiene TOTP confirmado; se actualiza cuando un login resuelve un
  challenge de 2FA, o cuando `2fa/confirm`/`2fa/disable` tienen éxito a través de este proxy.

## API Endpoints

Ver tabla completa en `README.md`. Convención de status codes (mismo split que `xindeler-auth`):
4xx nunca revela detalle interno, 5xx siempre se loguea con un `request_id` que también viaja en
el body de la respuesta.

## Variables de entorno

Ver tabla completa en `README.md`. Resumen por fase: bind/workers (0), DB/game-server/rate-limit/
digest (1), SMTP/OWNER_EMAIL opcionales (1), `AUTH_PUBLIC_URL`/`AUTH_SERVICE_TOKEN` (2 — el
segundo es el mismo secreto que ya usa el game server contra `xindeler-auth`, `/verify`).

## Seguridad — notas críticas

- Ninguna llamada mutable a `xindeler-auth` (`change_username`, `change_password`,
  `delete_account`, `2fa/*`) se hace directo desde el frontend — pasan por proxy acá
  (`/api/account/*`), autenticadas por la cookie de sesión, usando el `username` de la sesión
  (nunca uno provisto por el cliente).
- Fase L de `xindeler-auth` (2FA/TOTP) está implementada y deployada (flag `AUTH_2FA_ENABLED`
  apagado por default en xindeler-auth hasta que game/landing/overlord lo consuman). Cuando la
  cuenta tiene TOTP confirmado, el login responde `202 { challenge_id }` en vez de crear sesión
  directo — `POST /api/session/login/2fa` completa el segundo factor. Los errores TOTP-específicos
  (`TOTP_INVALID_CODE`, `ACCOUNT_2FA_LOCKED`, etc.) se reenvían con el `code`/`message` real de
  `xindeler-auth` en vez de colapsarse a un error genérico (`authclient.rs::is_totp_specific`).
- Toda mutación exitosa de cuenta **revoca todas las sesiones de esa cuenta** en el mismo
  request (hallazgo 8 de la tarea 007) — nunca solo la actual, nunca en un job aparte. Si
  `xindeler-auth` rechaza el cambio, la sesión no se toca.
- Cookie de sesión: `HttpOnly` + `Secure` + `SameSite=Lax`. Nunca en `localStorage` ni legible
  desde JS.
- CORS: allowlist exacta de origins (`https://xindeler.com`, `https://www.xindeler.com`, más los
  dos puertos de dev), nunca wildcard — mismo criterio que `cors_origin()` en `xindeler-auth`.
- Anti-enumeración donde `xindeler-auth` ya lo aplica: nunca revelar por early-return si una
  cuenta existe o tiene 2FA activo. `forgot-password` siempre responde `200 {ok:true}`.
- Limitación conocida y aceptada (C-03): `xindeler-auth`'s `reset_password` no devuelve el `uuid`
  que resuelve internamente, así que ese endpoint no puede revocar *todas* las sesiones de la
  cuenta como sí hace `change-password` — el TTL de 7 días es la mitigación real; se revoca la
  sesión actual si el llamado la trae. No pedirle a `xindeler-auth` que cambie ese contrato
  unilateralmente (ver `.backlog/SPEC.md`).

## Convenciones

- Sin comentarios salvo que el WHY sea no obvio
- `#![forbid(unsafe_code)]` en todos los crates (verificado por
  `scripts/check-unsafe-policy.sh` en CI)
- clippy + rustfmt obligatorios (CI los verifica con `-D warnings`)
- Errores: siempre `ApiError` enum + el split interno/público de `error.rs`, nunca `unwrap` en
  código de producción
- DB (Fase 1+): siempre parameterized queries via rusqlite (`params![]`), nunca string
  interpolation
- Migraciones (Fase 1+): secuenciales y **nunca se modifican** — solo se agregan nuevas
- Tokens sensibles (Fase 2+): siempre almacenar `SHA-256(token)`, nunca el raw

## Deploy

Mismo patrón que `xindeler-auth`/`xindeler-zuul` (mismo VPS, mismo operador, misma historia "sin
Docker, systemd + reemplazo atómico de binario") — una sola forma de operar entre los tres.

- VPS: `216.238.126.97` (`greenmountain.dev`), deployado por el usuario `mgrinberg` (no un usuario
  de servicio dedicado, a diferencia de `xindeler-zuul`).
- Deploy path: `/opt/xindeler-web-api/`. El servicio corre como `xindeler-web-api.service`
  (systemd), bindeado a `127.0.0.1:8020` — nginx expone `/api/*` de `xindeler.com` proxeando ahí.
- **Deploys son tag-based, cortados solo de `main`.** Cada merge a `main` recibe un tag SemVer
  (`vX.Y.Z`) en el merge commit — no opcional, `deploy/deploy.sh` lo exige y rechaza cualquier tag
  que no sea alcanzable desde `origin/main` (`git merge-base --is-ancestor`). Un tag es también la
  unidad de rollback: para volver atrás, correr el script de nuevo con un tag más viejo (reconstruye
  e instala desde esa fuente exacta, no confía en qué binario haya quedado tirado).
- **Qué número subir (criterio de Mati, 2026-08-23):** `X` (mayor) solo ante cambios
  estructurales/de arquitectura o forma de trabajar (ej. v2.0.0: cierre de `xindeler-auth` al
  exterior + todo el circuito de proxies nuevo). `Y` (el del medio) es el release normal — todo lo
  que no es un cambio estructural sube acá, aunque incluya features. `Z` (último) se reserva para
  bugfixes puntuales sobre un release ya cortado. Este servicio ya tuvo ~4 deploys manuales sin tag
  antes de que existiera este flujo — el primer tag no arranca en `v1.0.0`, arranca en `v2.0.0`
  porque ya veníamos de esos deploys previos y este release en particular es, además, un cambio
  estructural grande.
- Deploy: `bash /opt/xindeler-web-api/src/deploy/deploy.sh vX.Y.Z` → fetch de tags → verifica que
  el tag esté en `main` → backup de `data/web-api.db` (+ `-wal`/`-shm`) → `cargo build --release
  --locked` → guarda el binario actual como `.previous` → lo reemplaza → `systemctl restart
  xindeler-web-api.service` → health-check contra `/ping` (60s de margen) → si no responde,
  rollback automático al binario anterior.
- **NUNCA deployar al VPS ni reiniciar `xindeler-web-api.service` sin que Mati lo pida
  explícitamente en la sesión actual** — mismo criterio que los otros dos repos backend.

## Comandos útiles

```sh
# Compilar
cargo build

# Compilar release
cargo build --release

# Tests
cargo test --all

# Correr local
WEB_API_BIND_ADDR=127.0.0.1:8020 RUST_LOG=info cargo run -p xindeler-web-api-server
```

## Backlog

Ver `.backlog/README.md` para el estado fase por fase, `.backlog/SPEC.md` para las decisiones de
diseño, `.backlog/PLAN.md` para el plan de trabajo.
