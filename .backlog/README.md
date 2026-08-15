# Backlog — xindeler-web-api

Backend propio de `xindeler-web-landing`. Reemplaza al FastAPI de Python sin versionar
(`/srv/xindeler/waitlist-api/main.py`) y agrega sesión web autenticada — origen: tarea 007 del
backlog de `xindeler-web-landing`.

## Leyenda de estados

- `done` — hecho y verificado
- `in-progress` — en curso
- `todo` — planeado, no empezado
- `idea` — sin comprometer todavía

## Prioridades actuales

| Prioridad | Objetivo | Tareas | Condición para avanzar |
|---|---|---|---|
| P0 — Fundaciones | Repo, CI, hardening, scaffold | Fase A | — |
| P1 — Paridad | Reemplazar el Python sin cambiar comportamiento observable | Fase B | Fase A `done` — **Fase B deployada y verificada en producción real (2026-08-15)** |
| P2 — Producto | Sesión web autenticada | Fase C | Fase B `done` |
| P3 — Cierre | Frontend consume la sesión | Fase D | Fase C `done` — **`done`, ver detalle abajo** |

## Fase A — Repo, fundaciones y hardening (2026-08-14)

| ID | Tarea | Estado |
|---|---|---|
| A-01 | Crear repo `Matute289/xindeler-web-api` público | `done` |
| A-02 | Hardening del repo GitHub (ruleset, secret scanning, vulnerability alerts, `SECURITY.md`, `CODEOWNERS`) | `done` — Dependabot se probó y se sacó a pedido de Matías (2026-08-14): abría PRs y mandaba mails que no aportaban valor acá. Sin Dependabot, `cargo audit`/`cargo deny` en CI siguen cubriendo vulnerabilidades conocidas en cada PR y semanalmente. |
| A-03 | Scaffold Cargo workspace (`common`/`server`), seam HTTP framework-agnóstico copiado de `xindeler-auth` | `done` — `GET /ping` funcionando, build/test/clippy/fmt en verde |
| A-04 | CI (`check.yml`) + `scripts/check-{unsafe-policy,docs,docker-context}.sh` invocados desde CI | `done` |
| A-05 | `Dockerfile` + `docker-compose.yml` hardened (paridad dev/testing, no reemplaza systemd en prod) | `done` |
| A-06 | `CLAUDE.md`/`AGENTS.md` + `.backlog/{README,SPEC,PLAN}.md` | `done` |
| A-07 | Skills (`xindeler-web-api-dev`, `xindeler-web-api-architect`) y agentes reviewers dedicados | `done` |

## Fase B — Paridad funcional (2026-08-14)

| ID | Tarea | Estado |
|---|---|---|
| B-01 | Migrar `waitlist`/`contribute`/`status`/`count` a SQLite + Rust | `done` — tablas `waitlist`/`contributors` con `COLLATE NOCASE` para dedup, `TtlCache` para status/count, `RateLimiter` con TTL real |
| B-02 | Corregir los bugs relevados del Python actual | `done` — rate limit corre antes del dedup; HTML de emails escapado + `portfolio` solo se linkea con esquema `http(s)://`; `RateLimiter` evictea subjects inactivos; SQLite + WAL evita el problema de locking de los CSV; `/api/waitlist/count` ya no comparte location de nginx con el `limit_except POST` que lo bloqueaba (queda pendiente actualizar la config real de nginx en B-05) |
| B-03 | Portar `monthly-digest.py` (systemd timer mensual) | `done` — subcomando `xindeler-web-api-server digest`, apunta a `xindeler.com` (no al dominio legacy), watermark se escribe siempre incluso si el envío falla |
| B-04 | Migrar los registros reales de `waitlist.csv`/`contributors.csv` | `done` — corrido contra los datos reales del VPS el 2026-08-15: 5 filas de waitlist migradas + 1 duplicado detectado dentro del propio CSV (6 filas totales, `count` final = 5 emails únicos), 1 fila de contributors migrada. Re-corrida confirmó idempotencia (0 migrados, todo detectado como ya presente) |
| B-05 | Deploy en puerto nuevo, smoke-test directo, corte de nginx, apagar `xindeler-waitlist.service` | `done` — corte de producción real ejecutado el 2026-08-15, ver detalle abajo |

**Verificación de B-01/B-02/B-03:** 38 tests en verde (28 unitarios + 10 de integración con
`TestServer`, mismo patrón que `xindeler-auth/server/tests/http_security.rs` — binario real,
puerto libre, DB temporal). Cubren: contrato de status codes (201/422/429), anti-enumeración en
dedup, honeypot, CORS/privacy headers, rate limiting real, shutdown graceful ante SIGTERM.

**B-05, corte de producción (2026-08-15).** Ejecutado con confirmación explícita de Matías en cada
paso sensible (instalar el unit de systemd, tocar nginx, apagar el servicio Python), siguiendo el
plan tal como estaba escrito:

- Toolchain de Rust ya estaba instalado en el VPS (`~/.cargo/bin`, fuera del `PATH` de sesiones SSH
  no interactivas) y ya tenía acceso SSH de lectura al repo privado `xindeler-auth` (mismo patrón
  que ya usan `xindeler-server`/`xindeler-zuul` para compilar en el VPS directamente) — no hizo
  falta ninguna infraestructura nueva de CI/deploy key para este paso.
- Deploy en `/opt/xindeler-web-api/` (binario + `.env` chmod 600 + `data/`), mismo patrón que
  `/opt/xindeler-auth`. Unit de systemd con el mismo hardening que `xindeler-auth.service` y
  `xindeler-waitlist.service` (`NoNewPrivileges`, `PrivateTmp`, `ProtectSystem=strict`,
  `ReadWritePaths=/opt/xindeler-web-api/data`). Puerto `8020` (coincide con el default ya
  documentado de `WEB_API_BIND_ADDR`).
- `AUTH_SERVICE_TOKEN` reusado del `.env` del game server, `SMTP_*`/`OWNER_EMAIL` copiados del
  `.env` del servicio Python, `MAIL_FROM` compuesto como `"{FROM_NAME} <{SMTP_USER}>"` (mismo
  formato que el `From:` que ya armaba el Python) — todo copiado server-side vía un script remoto,
  nunca expuesto en la conversación.
- **Bug real encontrado durante el smoke-test directo (puerto 8020, antes de tocar nginx):**
  `xindeler-auth` responde `AuthError::InvalidLogin` con status **`400`**, no `401` como asumía
  `map_sign_in_error` en `session.rs` desde C-01 — cualquier login con credenciales incorrectas a
  través del proxy devolvía `502 UPSTREAM_ERROR` en vez de `401 INVALID_CREDENTIALS`. Nunca lo
  atrapó ningún test porque todos usaban `FakeAuthServer` con `401`, no el `400` real. Corregido y
  mergeado como `#9` antes de seguir con el corte — mismo criterio que `map_account_error` de
  `account.rs` ya aplicaba correctamente desde C-02. Ver `SPEC.md` para el detalle.
- Corte de nginx: `proxy_pass` de los tres `location` de `/api/*` movido de `127.0.0.1:8010` a
  `127.0.0.1:8020`, con backup automático + `nginx -t` antes de aplicar + rollback automático si el
  test de sintaxis fallaba. De paso se agregó `location = /api/waitlist/count` (exact match,
  prioridad sobre el `location /api/waitlist` con `limit_except POST` que lo bloqueaba) — bug #2 de
  la migración original, corregido en el mismo cambio.
- Verificación end-to-end contra `https://xindeler.com` real (no contra el puerto directo):
  `/api/status`, `/api/waitlist/count` (ya no bloqueado), `/api/account/check-username`,
  `/api/session/login` con credenciales inválidas (`401`, confirma el fix del punto anterior en
  producción real), `/api/session/me` sin cookie (`401`), headers CORS/privacy presentes.
- `xindeler-waitlist.service` (Python) parado y deshabilitado — **no borrado**. Código en
  `/srv/xindeler/waitlist-api/` y CSVs originales en `/srv/xindeler/data/` quedan de referencia en
  el VPS; se pueden limpiar en una pasada futura una vez confirmado que no hace falta volver atrás.
- Nota operativa, no bloqueante: agregar `(root) NOPASSWD: /usr/bin/systemctl restart
  xindeler-web-api.service` a los sudoers del VPS (mismo patrón que ya existe para
  `auth`/`server-cli`/`zuul`) simplificaría los próximos deploys — hoy cada restart vía systemd
  necesita la contraseña de sudo de Matías.

## Fase C — Sesión web autenticada (2026-08-15)

| ID | Tarea | Estado |
|---|---|---|
| C-01 | Tabla `sessions` + `POST /api/session/login`, `GET /me`, `POST /logout` | `done` — `/login/2fa` queda afuera hasta que Fase L de `xindeler-auth` (2FA) exista; `login()` ya deja el comentario de dónde entra ese branch. Cookie `HttpOnly`+`Secure`+`SameSite=Lax`, TTL 7 días absoluto |
| C-02 | Proxy autenticado `/api/account/*` hacia `xindeler-auth` | `done` — `authclient.rs` suma `check_username`/`change_username`/`change_password`/`delete_account` (los cuatro son endpoints públicos sin service-token del lado de `xindeler-auth`). `account.rs` nuevo: los tres mutantes exigen sesión, usan el `username` de la sesión (nunca el del cliente), y **revocan todas las sesiones de la cuenta** en el mismo request que `xindeler-auth` confirma el cambio — si lo rechaza, la sesión no se toca |
| C-03 | Reroute `ForgotPasswordPage`/`ResetPasswordPage` de `xindeler-web-landing` a través de acá | `done` — backend: `authclient.rs` suma `forgot_password`/`reset_password` (endpoints públicos sin service-token, igual que los cuatro de C-02); `account.rs` suma `POST /api/account/forgot-password` (sin sesión, siempre `200 {ok:true}`, anti-enumeración) y `POST /api/account/reset-password` (sin sesión; revoca la sesión del llamador si trae una, pero no puede revocar *todas* las sesiones de la cuenta — `xindeler-auth`'s `reset_password` no devuelve el `uuid` que resuelve internamente, ver `.backlog/SPEC.md`). Frontend: `Matute289/xindeler-web-landing#40` — `ForgotPasswordPage.jsx`/`ResetPasswordPage.jsx` llaman acá en vez de `auth.xindeler.com` directo |
| C-04 | Coordinar `AUTH_SERVICE_TOKEN` con `xindeler-auth` | `done` — se reusa el mismo secreto del game server (decisión confirmada, sin objeción). Deploy key de solo lectura (`AUTH_REPO_SSH_KEY`) agregado a `xindeler-auth` para que el CI de este repo (público) pueda fetchear `xindeler-auth-common` (privado) — mismo patrón que `xindeler-new-horizon`/`xindeler-zuul` |

**Verificación de C-01:** 2 tests de integración contra un `xindeler-auth` falso (servidor HTTP
mínimo hecho a mano, sin dependencia nueva) — cubren login exitoso con
cookie→`/me`→logout→`/me` 401, y credenciales inválidas sin setear cookie.

**Verificación de C-02:** 8 tests de integración nuevos — `check-username` proxea disponibilidad;
los tres endpoints mutantes rechazan sin sesión (401); cada uno revoca la sesión que hizo el
cambio (`/me` con la cookie vieja pasa a 401); un `change_password` rechazado por `xindeler-auth`
(contraseña actual incorrecta) **no** revoca la sesión. Total del repo: 50 tests (32 unitarios +
18 de integración).

**Verificación de C-03 (backend):** 6 tests de integración nuevos con `FakeAuthServer` — `forgot-password`
responde `200 {ok:true}` sin sesión y valida email no vacío; `reset-password` responde `200 {ok:true}`
sin sesión, valida `token`/`new_password_prehash` no vacíos, propaga un rechazo de `xindeler-auth`
(token inválido/expirado) como `422`, y revoca la sesión del llamador cuando el request trae una
cookie de sesión válida. Total del repo: 56 tests (32 unitarios + 24 de integración).

## Fase D — Frontend consume la sesión (2026-08-15)

| ID | Tarea | Estado |
|---|---|---|
| D-01 | Env vars + proxy de Vite en `xindeler-web-landing` para desarrollo local real | `done` — `Matute289/xindeler-web-landing#41`. Las 5 llamadas same-origin (`waitlist`, `contribute`, `status`, `account/forgot-password`, `account/reset-password`) pasan de URL absoluta a ruta relativa `/api/...`; `vite.config.js` suma `server.proxy['/api']` con target configurable vía `VITE_API_PROXY_TARGET` (default: producción, mismo comportamiento de hoy) |
| D-02 | `AuthModal.jsx` deja de descartar el resultado del login, primera versión de "hay alguien logueado" | `done` — ver detalle abajo |

**D-02, resuelto (2026-08-15).** El hallazgo original (`POST /api/session/login` colapsaba el `403
EMAIL_VERIFICATION_REQUIRED` de `xindeler-auth` a un error genérico, rompiendo el modal de cuentas
legacy de `AuthModal.jsx` si se migraba el login sin arreglarlo primero) se resolvió sin cambiar el
contrato público del endpoint para nadie más:

- `authclient.rs::sign_in` inspecciona un `403` de `/generate_token`: si el body parsea como
  `xindeler_auth_common::EmailVerificationRequiredResponse` **y** `code == "EMAIL_VERIFICATION_REQUIRED"`,
  devuelve el nuevo `AuthClientError::EmailVerificationRequired(body)`; cualquier otro `403` (o un
  body que no parsea) sigue siendo el `Rejected(403)` genérico de siempre.
- `session::login` intercepta esa variante **antes** de `map_sign_in_error` y reenvía el body de
  `xindeler-auth` tal cual, `403`, sin `Set-Cookie` — mismo shape (`code`, `message`, `deadline`,
  `completion_token`) que `AuthModal.jsx` ya sabía parsear cuando le pegaba directo a
  `auth.xindeler.com`, así que ese branch del frontend no necesitó cambiar una sola línea.
- `map_sign_in_error` gana un brazo de exhaustividad para la nueva variante (nunca alcanzado en la
  práctica, porque `login()` la intercepta primero) — igual en `map_account_error`/`map_recovery_error`
  de `account.rs` (ninguno de esos cuatro proxies llama `sign_in`, así que tampoco es alcanzable ahí).
- `account-email`/`resend-verification` (los dos requests que sigue haciendo el modal legacy después
  de abrirse) **no** se proxearon — usan un bearer `completion_token` de un solo uso, no una sesión de
  cookie, así que no hay ninguna revocación que este servicio necesite coordinar ahí (a diferencia de
  `change_username`/`change_password`/`delete_account`). `AuthModal.jsx` los sigue llamando directo
  contra `auth.xindeler.com`.
- `AuthModal.jsx`: el login pasa de `POST ${AUTH_API}/generate_token` (descartaba el `AuthToken`) a
  `POST /api/session/login`, mismo dominio que el resto del proxy, estableciendo una cookie de
  sesión real en éxito en vez de tirar el resultado. El body cambia la clave `password` →
  `password_prehash` para matchear `LoginPayload`. Todo el manejo de errores existente (401, 403
  legacy, catch de red) queda igual — el 403 ahora sí trae el body real.
- Deliberadamente fuera de alcance: ningún indicador de "sesión activa" nuevo en la UI (navbar,
  avatar, botón de logout). Eso pertenece a la pantalla de cuenta (005), todavía sin diseñar —
  inventar esa UI acá hubiera sido pisar una decisión de producto sin que Matías la vea primero.
  Lo que sí queda resuelto es la parte de infraestructura: la sesión ahora se establece de verdad y
  `GET /api/session/me` está listo para que 005/006 la consuman.

Tests nuevos: `login_forwards_email_verification_required_verbatim_without_a_cookie` (403, sin
cookie, `completion_token`/`deadline` intactos) y
`login_treats_an_unparseable_403_body_as_generic_invalid_credentials` (un 403 sin ese shape sigue
siendo el genérico de siempre). Total del repo: 58 tests (32 unitarios + 26 de integración).

---

## Fase E — Proxy de 2FA/TOTP para la pantalla de cuenta (005 de `xindeler-web-landing`, 2026-08-15)

`xindeler-auth` shippeó Fase L (G-03, 2FA/TOTP) el mismo día — PR #29, mergeado y desplegado en
producción con el kill switch `AUTH_2FA_ENABLED` apagado por default. Esto desbloqueó la mitad del
alcance de la tarea 005 de `xindeler-web-landing` (pantalla de cuenta) que estaba pendiente de ese
contrato. Mismo criterio de siempre: nada mutable se llama directo desde el frontend a
`auth.xindeler.com`, todo pasa por acá.

- `authclient.rs` suma `totp_login`/`totp_enroll`/`totp_confirm`/`totp_disable`/
  `totp_regenerate_backup_codes`, y `sign_in()` cambia de firma (`Result<SignInOutcome, _>`) para
  distinguir un token directo de un `202` challenge de 2FA.
- **Refactor transversal de manejo de errores**: `AuthClientError::Rejected(u16)` se reemplaza por
  `RejectedWithBody{status, code, message}` en *todos* los métodos del cliente (no solo los
  nuevos) — necesario para que `change_username`/`delete_account`/los cuatro de `2fa/*` puedan
  reenviar códigos TOTP-específicos (`TOTP_INVALID_CODE`, `ACCOUNT_2FA_LOCKED`, etc.) en vez de
  colapsarlos al error genérico de siempre. `map_sign_in_error`/`map_account_error`/
  `map_recovery_error` se reescriben sobre el nuevo shape, preservando exactamente el mismo mapeo
  status→error que ya tenían para los casos no-TOTP — verificado con los 62 tests que ya existían,
  todos siguieron pasando sin tocarlos.
- `error.rs` suma `forwarded_response(status, code, message)` — reenvía un error tal cual llegó de
  `xindeler-auth` en vez de pasar por el catálogo fijo de `public_fields()`; `PublicErrorBody`
  pasa de `&'static str` a `String` para poder llevar esos valores dinámicos.
- `session::login` cambia: si la cuenta tiene TOTP confirmado, responde `202 { challenge_id,
  expires_in }` sin sesión — hallazgo 2 de backlog 007, ahora real en vez de hipotético. Nuevo
  `POST /api/session/login/2fa` completa el segundo factor y recién ahí crea la sesión.
- `account.rs` suma `POST /api/account/2fa/{enroll,confirm,disable,backup-codes/regenerate}`
  (requieren sesión, usan el `username` de la sesión). `change_username`/`delete_account` ganan un
  campo `code` opcional, reenviado tal cual a `xindeler-auth` (no-op si la cuenta no tiene TOTP
  confirmado — verificado leyendo `require_step_up_if_confirmed` en `xindeler-auth`, no asumido).
- **Estado de 2FA derivado** (`totp_status.rs`, tabla `totp_status`): no existe ningún endpoint de
  "estado de TOTP" en Fase L, así que se infiere de lo que este servicio ya observa — un login que
  resolvió un challenge, o un `2fa/confirm`/`2fa/disable` exitoso a través de acá — en vez de
  pedirle a `xindeler-auth` un contrato nuevo. Expuesto en `GET /api/session/me` como
  `totp_enabled`.
- `2fa/disable` revoca todas las sesiones de la cuenta (reduce seguridad, mismo criterio que
  `change_password`); `2fa/confirm` no lo hace (activar 2FA no reduce seguridad).
- Dependencia `xindeler-auth-common` actualizada al commit que incluye Fase L (`c40c3eb5...`,
  antes pineada a un commit previo a esa PR).
- **Hallazgo adicional, mismo PR:** `change_username` puede fallar con `400` por cuatro razones
  *distintas* — contraseña incorrecta, nombre ya tomado (`USERNAME_UNAVAILABLE`), nombre reservado
  (`USERNAME_RESERVED`), o cambiado hace menos de 30 días (`USERNAME_CHANGE_COOLDOWN`) — confirmado
  leyendo `auth::change_username` en `xindeler-auth`, no asumido. Solo la primera es genuinamente
  "credenciales inválidas"; las otras tres se sumaron a la lista de códigos que se reenvían
  verbatim (`should_forward_verbatim`, renombrada desde `is_totp_specific` para reflejar que ya no
  es solo sobre TOTP) — antes las cuatro colapsaban al mismo `INVALID_CREDENTIALS` genérico.

Tests nuevos: 17 tests de integración (challenge de login, forwarding de errores TOTP-específicos y
del cooldown de username en login/2fa y en change-username, los cuatro endpoints de `2fa/*`, estado
derivado tras confirm/disable) + 3 unitarios de `totp_status`. Total del repo: 74 tests (35
unitarios + 39 de integración).

---

Detalle completo de decisiones de diseño en `SPEC.md`, plan de trabajo con próximos pasos
concretos en `PLAN.md`.
