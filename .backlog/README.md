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
| P1 — Paridad | Reemplazar el Python sin cambiar comportamiento observable | Fase B | Fase A `done` |
| P2 — Producto | Sesión web autenticada | Fase C | Fase B deployada y verificada en producción |
| P3 — Cierre | Frontend consume la sesión | Fase D | Fase C `done` |

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
| B-04 | Migrar los 7 registros reales de `waitlist.csv`/`contributors.csv` | `parcial` — subcomando `migrate-csv` implementado e idempotente (probado con fixtures), **falta correrlo contra los datos reales del VPS** (parte de B-05, requiere acceso a producción) |
| B-05 | Deploy en puerto nuevo, smoke-test directo, corte de nginx, apagar `xindeler-waitlist.service` | `todo` — bloqueado a propósito hasta confirmación explícita de Matías (toca producción real) |

**Verificación de B-01/B-02/B-03:** 38 tests en verde (28 unitarios + 10 de integración con
`TestServer`, mismo patrón que `xindeler-auth/server/tests/http_security.rs` — binario real,
puerto libre, DB temporal). Cubren: contrato de status codes (201/422/429), anti-enumeración en
dedup, honeypot, CORS/privacy headers, rate limiting real, shutdown graceful ante SIGTERM.

## Fase C — Sesión web autenticada (2026-08-15)

| ID | Tarea | Estado |
|---|---|---|
| C-01 | Tabla `sessions` + `POST /api/session/login`, `GET /me`, `POST /logout` | `done` — `/login/2fa` queda afuera hasta que Fase L de `xindeler-auth` (2FA) exista; `login()` ya deja el comentario de dónde entra ese branch. Cookie `HttpOnly`+`Secure`+`SameSite=Lax`, TTL 7 días absoluto |
| C-02 | Proxy autenticado `/api/account/*` hacia `xindeler-auth` | `done` — `authclient.rs` suma `check_username`/`change_username`/`change_password`/`delete_account` (los cuatro son endpoints públicos sin service-token del lado de `xindeler-auth`). `account.rs` nuevo: los tres mutantes exigen sesión, usan el `username` de la sesión (nunca el del cliente), y **revocan todas las sesiones de la cuenta** en el mismo request que `xindeler-auth` confirma el cambio — si lo rechaza, la sesión no se toca |
| C-03 | Reroute `ForgotPasswordPage`/`ResetPasswordPage` de `xindeler-web-landing` a través de acá | `parcial` — backend `done`: `authclient.rs` suma `forgot_password`/`reset_password` (endpoints públicos sin service-token, igual que los cuatro de C-02); `account.rs` suma `POST /api/account/forgot-password` (sin sesión, siempre `200 {ok:true}`, anti-enumeración) y `POST /api/account/reset-password` (sin sesión; revoca la sesión del llamador si trae una, pero no puede revocar *todas* las sesiones de la cuenta — `xindeler-auth`'s `reset_password` no devuelve el `uuid` que resuelve internamente, ver `.backlog/SPEC.md`). **Falta**: actualizar `ForgotPasswordPage.jsx`/`ResetPasswordPage.jsx` en `xindeler-web-landing` para llamar acá en vez de `auth.xindeler.com` directo |
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

## Fase D — Frontend consume la sesión (planeada)

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

Detalle completo de decisiones de diseño en `SPEC.md`, plan de trabajo con próximos pasos
concretos en `PLAN.md`.
