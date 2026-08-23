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
| — | Proxy de 2FA/TOTP para la pantalla de cuenta | Fase E | `done` (2026-08-15), en producción |
| P2 — Producto | Proxy de personajes (`xindeler-new-horizon` NH-79) | Fase F | `done` (2026-08-20) — implementado, mergeado (PR #14, re-pin PR #15) **y deployado a producción el mismo día**. Ver detalle abajo. |

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
| D-03 | **`authclient.rs` no reenvía la IP real del caller a `xindeler-auth` en ninguna de sus llamadas** — todo el tráfico de la landing (login, 2FA, forgot/reset-password, change_password/username, delete_account, 2fa/*) le llega a auth como si viniera de una sola IP (la de este servicio) | `done` — ver detalle abajo |

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

**D-03, resuelto (2026-08-23).** Hallazgo cross-repo desde `xindeler-auth`, 2026-08-22 (detalle
original del hallazgo abajo). Se resolvió threadeando la IP real del caller (`web.rs::remote()`, ya
existente del lado de entrada) hasta cada llamada saliente de `authclient.rs`:

- Los 18 métodos públicos de `AuthClient` que le pegan a `xindeler-auth` ganan un parámetro
  `caller_ip: IpAddr` (primero después de `&self`) y agregan `.header("X-Real-IP",
  caller_ip.to_string())` a la request. Sin excepciones — incluye `verify()` e
  `issue_character_access_token()` (server-to-server, con `service_token`/
  `character_service_token`), no solo las rutas públicas.
- `session::create_session` gana el mismo parámetro (lo necesita para pasárselo a `verify()`);
  `session::login_2fa` pasa de no recibir IP a recibirla como segundo parámetro (mismo lugar que
  `session::login`/`oauth_login` ya la reciben).
- Los 16 handlers de `account.rs` ganan `remote_ip: IpAddr` como parámetro nuevo — ninguno lo
  recibía antes de este cambio.
- `web.rs::dispatch` pasa el `remote_ip` que ya calculaba (para logging) a cada llamada de
  `account::*`/`session::login_2fa` que antes no lo recibía.
- Sin cambios de contrato público hacia el frontend — es un cambio interno de `xindeler-web-api` →
  `xindeler-auth`, invisible para `xindeler-web-landing`.

Tests nuevos (`server/tests/http_integration.rs`): `FakeAuthServer` gana `last_x_real_ip()`
(captura el último request crudo recibido y extrae el header, case-insensitive). Tres tests
end-to-end confirman el forwarding real vía `X-Forwarded-For` en la request del test cliente
(`TestServer`'s `WEB_API_TRUSTED_PROXIES` por default confía en el peer loopback del test, igual
que un nginx real confiaría en `$remote_addr`): `login_forwards_the_real_caller_ip_as_x_real_ip`,
`check_username_forwards_the_real_caller_ip_as_x_real_ip`,
`register_forwards_the_real_caller_ip_as_x_real_ip`. Total del repo: 101 tests (38 unitarios + 63
de integración).

**Pendiente, fuera de este cambio:** si `xindeler-web-api` no corre en el mismo VPS que
`xindeler-auth` (o su IP no cae en el default `127.0.0.0/8,::1/128` de `AUTH_TRUSTED_PROXIES`),
hay que sumarla ahí explícitamente del lado de `xindeler-auth` — sin eso, `xindeler-auth` sigue
ignorando el header aunque ahora sí lo reciba, porque solo confía en `X-Real-IP` de proxies
declarados. Coordinar con Mati/`xindeler-auth` antes de asumir que ya está cubierto en producción.

**D-03, hallazgo original (2026-08-22, cross-repo desde `xindeler-auth`).**

`server/src/authclient.rs` — **ninguna** de sus llamadas a `xindeler-auth` manda `X-Real-IP` (ni
ningún otro header equivalente): `sign_in` (`:206`, `/generate_token`), `submit_2fa_code` (`:291`,
`/login/2fa`), `verify` (`:313`), `issue_character_access_token` (`:343`), `change_username`
(`:395`), `change_password` (`:422`), `delete_account` (`:445`), `forgot_password`/`reset_password`
(`:465`/`:486`), y las cuatro de `2fa/*` (`:513`–`:582`). Todas van con `.json(&payload).send()` sin
tocar los headers salientes.

`xindeler-auth` rate-limita `/generate_token`/`/login/2fa`/etc. por IP (60 req/10min,
`AUTH_RATE_LIMIT_MAX`/`WINDOW_SECS`) leyendo `X-Real-IP` **solo si** el caller está en
`AUTH_TRUSTED_PROXIES` — si no manda ese header, usa la IP del socket, que para toda la landing es
siempre la de este servicio. Resultado: **todo el login/2FA/recovery/cambio de cuenta de la landing
hoy comparte un solo balde de rate limit contra auth**, sin importar cuántos usuarios reales estén
pegando. No es teórico — está así en producción ahora mismo. (El lockout de cuenta por intentos
fallidos, G-08, no se ve afectado — ese se indexa por uuid de cuenta, no por IP; lo que sí se ve
afectado es el límite genérico por IP y el rastro de auditoría de `xindeler-auth`, donde cada línea
`[AUDIT] login_fail ip=...` termina siendo la misma IP para cualquier intento que pase por acá.)

**Este mismo repo ya resuelve el problema simétrico del lado de entrada** — `web.rs::remote()`
(`:49`) ya confía en `X-Real-IP` que pone nginx (`$remote_addr`, pisando cualquier valor del
cliente) cuando el peer está en la config de proxies confiables de este servicio. La IP real del
caller ya está disponible en cada handler que llama a `authclient.rs` — el trabajo es threadearla
hacia esas llamadas salientes (agregar `.header("X-Real-IP", ip.to_string())` a cada una, mismo
valor que `remote()` ya resolvió para ese request) y agregar este servicio a
`AUTH_TRUSTED_PROXIES` del lado de `xindeler-auth` si no corre en una IP ya cubierta por el default
(`127.0.0.0/8,::1/128` — si `xindeler-web-api` corre en el mismo VPS que auth, loopback, puede que
ya alcance sin tocar nada del lado de auth).

**Prioridad:** Matías pidió que se tome apenas termine el trabajo en curso de este repo. Mismo
patrón exacto que `xindeler-vinz-clortho` (el gateway nuevo para el cliente nativo del juego) ya
está implementando desde cero — puede servir de referencia directa para el fix acá.

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

**Deploy a producción (2026-08-15, mismo día).** `xindeler-web-api` no tiene CD automático (a
diferencia de `xindeler-web-landing`, que sí deploya en cada push a `main`) — el binario del VPS se
actualizó manualmente con el mismo patrón de B-05: `git pull` + `cargo build --release` en
`/opt/xindeler-web-api/src`, binario anterior respaldado como `.previous2`, `systemctl restart`
(con confirmación explícita de Matías, mismo criterio que toda acción con `sudo` sobre el VPS).
Verificado end-to-end contra `https://xindeler.com` real: `/api/status`/`/api/waitlist/count` sin
regresión, `POST /api/session/login/2fa` y `POST /api/account/2fa/enroll` responden `422`/`401`
(no `404`) — confirma que las rutas nuevas están activas. La migración `V4__totp_status.sql` se
aplicó sola al arrancar el binario nuevo, mismo mecanismo que toda migración anterior.

## Fase F — Proxy de personajes para la pantalla de cuenta (`xindeler-new-horizon` NH-79, 2026-08-16)

Relayado desde `xindeler-new-horizon` NH-79 (worksheet §10 resuelto por Matías 2026-08-16) y
`xindeler-auth` Fase N (diseño del token acotado, PR #38 de ese repo). Este repo es el gateway
público elegido en NH-79 §10 [Q2] — mismo criterio de siempre: nada mutable ni server-to-server se
llama directo desde el frontend, todo pasa por acá.

**Flujo de tres saltos, distinto del que ya usa `verify()`:**
1. La landing ya está logueada acá (sesión propia, `resolve_session`).
2. Este servicio pide a `xindeler-auth` un `CharacterAccessToken` acotado para el `uuid` de la
   sesión — **credencial nueva, `WEB_API_SERVICE_TOKEN`, nunca `AUTH_SERVICE_TOKEN`** (esa sigue
   siendo exclusiva de `verify()`/`username_to_uuid`/`uuid_to_username`; `xindeler-auth` la rechaza
   explícitamente para este endpoint nuevo — ver Fase N de ese repo, la asimetría es la garantía
   central del diseño ahí).
3. Este servicio reenvía ese token al game server (nuevo endpoint loopback, NH-79 spec §4/§9 de
   ese repo), que lo canjea contra `xindeler-auth` usando **su propia** credencial existente
   (`AUTH_SERVICE_TOKEN`, la que ya tiene para `/verify`) — este servicio nunca habla directo con
   el game server con una credencial de *login*, solo reenvía el token acotado ya emitido.

- `authclient.rs` suma `character_service_token: Option<String>` (segundo campo en `AuthClient`,
  mismo patrón `Option`+validación de `service_token`) y
  `issue_character_access_token(uuid) -> Result<CharacterAccessToken, AuthClientError>` — mismo
  shape que `verify()` (`.bearer_auth(character_service_token)`, mapea `RejectedWithBody`) pero
  contra `POST /issue-character-access-token` y con el bearer nuevo, no el existente.
- **Nuevo cliente, `game_server_client.rs`** (no reusa `AuthClient` — habla con un servicio
  distinto, con su propio contrato, no con `xindeler-auth`): `list_characters(token) ->
  Result<Vec<CharacterSummary>, GameServerClientError>` (`GET
  /player_api/v1/characters`, `Authorization: Bearer <CharacterAccessToken>`) y
  `rename_character(token, character_id, new_alias) -> Result<(), GameServerClientError>` (`POST
  /player_api/v1/characters/{id}/rename`, mismo bearer). Nueva config `GAME_SERVER_URL` (default
  `http://127.0.0.1:<player_api_port>`, puerto exacto a confirmar contra el plan real de NH-79 en
  `xindeler-new-horizon`).
- **Nuevos handlers en `account.rs`**: `list_characters(request, state)` y
  `rename_character(body, request, state)` — mismo patrón que `totp_enroll`: `resolve_session`,
  pedir el `CharacterAccessToken` fresco (uno nuevo **por llamada**, nunca cacheado ni reusado
  entre acciones — TTL 60s de un solo uso, spec de Fase N de `xindeler-auth`), reenviar al game
  server, mapear errores.
- Rutas nuevas: `GET /api/account/characters`, `POST
  /api/account/characters/{character_id}/rename`.
- **Riesgo operacional, no de diseño, sin confirmar todavía**: el `web_address` del game server es
  loopback-only por decisión explícita de `xindeler-new-horizon` (NH-75, "nunca debe bindear a
  nada que no sea loopback") — esto solo funciona sin túnel/VPN si este servicio y el game server
  terminan corriendo en el **mismo host**. El game server todavía no está deployado
  (`CLAUDE.md` de `xindeler-web-landing`: "el servidor aún no está deployado"), así que esta
  topología no está decidida — flagged para confirmar con Matías antes del deploy real de esta
  fase, no algo que este documento pueda asumir. **Resuelto 2026-08-20** (ver sección "Deploy a
  producción" más abajo): mismo VPS, mismo host — el riesgo listado acá ya no aplica.

**Estado:** 🟢 `done` (2026-08-20) — implementado y mergeado, PR [#14](https://github.com/Matute289/xindeler-web-api/pull/14) más el re-pin [#15](https://github.com/Matute289/xindeler-web-api/pull/15) (una vez que N-01 mergeó en `xindeler-auth`, este repo dejó de apuntar a esa rama de feature y volvió a `main`). Orden de dispatch
cumplido: `xindeler-new-horizon` Fase 1 ([PR #197](https://github.com/Matute289/xindeler-new-horizon/pull/197)) → `xindeler-auth` N-01 ([PR #39](https://github.com/Matute289/xindeler-auth/pull/39)/[#40](https://github.com/Matute289/xindeler-auth/pull/40)) → este repo → `xindeler-new-horizon` Fase 2 ([PR #198](https://github.com/Matute289/xindeler-new-horizon/pull/198), reemplaza el stub de auth por la verificación real). Implementado: `AuthClient::issue_character_access_token` (nueva
credencial `WEB_API_SERVICE_TOKEN`, nunca la existente), `game_server_client.rs` nuevo (cliente
propio, respuestas de rechazo en texto plano, no el envelope JSON de `xindeler-auth`),
`GET /api/account/characters` y `POST /api/account/characters/{id}/rename`, guard de arranque que
rechaza si `AUTH_SERVICE_TOKEN` y `WEB_API_SERVICE_TOKEN` coinciden. 84 tests en verde (38
unitarios + 46 de integración, 7 nuevos de Fase F).

**El bloqueador de seguridad real que impedía el deploy ya se resolvió** — la Fase 2 de NH-79
mergeó (`xindeler-new-horizon` PR #198), así que `player_api/v1` ya no confía en un bearer crudo
como `uuid`, verifica de verdad contra `xindeler-auth`.

**Validado end-to-end en local 2026-08-20**: los tres servicios corriendo juntos (este +
`xindeler-auth` + `xindeler-new-horizon`), cuenta real: se creó un personaje por el protocolo real
del juego, se lo listó y renombró vía `GET/POST /api/account/characters*`, se probaron los tres
casos de rechazo (nombre vacío, caracteres inválidos, personaje inexistente), y se lo eliminó — con
la cadena de auth real de punta a punta. De paso se encontró y arregló (ajeno a esta fase) un bug
que crasheaba el arranque del game server (`EventBus<DismissSummonEvent>` sin registrar,
`xindeler-new-horizon` [PR #199](https://github.com/Matute289/xindeler-new-horizon/pull/199)).
**Deploy a producción (2026-08-20, mismo día).** Mismo patrón manual que Fase B/E (sin CD
automático): `git pull` + `cargo build --release --locked` en `/opt/xindeler-web-api/src`, binario
anterior respaldado como `.previous3`, reemplazo atómico y `systemctl restart` (confirmación
explícita de Matías). 0 migraciones nuevas que aplicar (Fase F no agrega tablas). Smoke test contra
`http://127.0.0.1:8020` sin regresión: `/api/status` y `/api/waitlist/count` en `200`,
`/api/session/me` en `401` como siempre; `GET /api/account/characters` (ruta nueva) responde `401`
también — confirma que la ruta está activa, no `404`. Logs de arranque limpios.

**A propósito, sin resolver todavía en este deploy — el feature no funciona de punta a punta
en producción hasta que se complete lo siguiente:**
1. El game server de `xindeler-new-horizon` está deployado en el VPS pero con una **versión
   vieja**, sin NH-79 — Matías priorizó una tarea nueva en ese repo para actualizarlo (ver su
   backlog), después de la que ya está en curso ahí. **Actualización 2026-08-20 (más tarde el
   mismo día):** esa tarea (BL-83) mergeó su deploy script
   ([PR #203](https://github.com/Matute289/xindeler-new-horizon/pull/203)), pero, según el propio
   PR, todavía no se corrió contra producción — sigue sin confirmar si el binario que está
   realmente corriendo en el VPS ya incluye NH-79 o sigue siendo la versión vieja.
2. `WEB_API_SERVICE_TOKEN` todavía no está en el `.env` de este servicio en el VPS (confirmado
   por SSH, `grep` de las claves — el valor no se tocó desde acá). Tiene que ser el mismo valor
   ya configurado del lado de `xindeler-auth`.
3. `WEB_API_GAME_SERVER_PLAYER_API_URL` no está seteada — cae al default
   `http://127.0.0.1:14005`, que hay que confirmar contra el puerto real que use el game server
   nuevo en producción.
4. ~~El riesgo de topología (el game server es loopback-only por NH-75, necesita compartir host
   con este servicio) sigue sin resolver.~~ **Resuelto 2026-08-20**: una verificación de
   infraestructura confirmó `xindeler-server-cli` corriendo en el mismo VPS que este servicio —
   `0.0.0.0:14004` (protocolo del juego, público) y `127.0.0.1:14005` (`player_api/v1`, loopback,
   tal como exige NH-75). Mismo host, loopback intacto — ya no es un riesgo abierto.

Hasta que se resuelvan 1-3, `GET/POST /api/account/characters*` va a devolver error ante cualquier
llamada real — esperado y aceptado por Matías para este deploy ("por ahora cuando se quiera
consumir ese servicio va a tirar error").

---

Detalle completo de decisiones de diseño en `SPEC.md`, plan de trabajo con próximos pasos
concretos en `PLAN.md`.
