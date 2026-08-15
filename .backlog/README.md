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
| C-02 | Proxy autenticado `/api/account/*` hacia `xindeler-auth` | `todo` — **decisión tomada**: HTTP propio vía `authclient.rs` (ya existe, `sign_in`/`verify`), nunca `xindeler-authc` (hashea internamente). Falta agregar `change_username`/`change_password`/`delete_account`/`check_username` al cliente y los handlers proxy |
| C-03 | Reroute `ForgotPasswordPage`/`ResetPasswordPage` de `xindeler-web-landing` a través de acá | `todo` |
| C-04 | Coordinar `AUTH_SERVICE_TOKEN` con `xindeler-auth` | `done` — se reusa el mismo secreto del game server (decisión confirmada, sin objeción). Deploy key de solo lectura (`AUTH_REPO_SSH_KEY`) agregado a `xindeler-auth` para que el CI de este repo (público) pueda fetchear `xindeler-auth-common` (privado) — mismo patrón que `xindeler-new-horizon`/`xindeler-zuul` |

**Verificación de C-01:** 2 tests de integración nuevos (12 en total, los 10 de Fase B + estos)
contra un `xindeler-auth` falso (servidor HTTP mínimo hecho a mano, sin dependencia nueva) —
cubren login exitoso con cookie→`/me`→logout→`/me` 401, y credenciales inválidas sin setear
cookie. Total del repo: 44 tests (32 unitarios + 12 de integración).

## Fase D — Frontend consume la sesión (planeada)

| ID | Tarea | Estado |
|---|---|---|
| D-01 | Env vars + proxy de Vite en `xindeler-web-landing` para desarrollo local real | `todo` |
| D-02 | `AuthModal.jsx` deja de descartar el resultado del login, primera versión de "hay alguien logueado" | `todo` |

---

Detalle completo de decisiones de diseño en `SPEC.md`, plan de trabajo con próximos pasos
concretos en `PLAN.md`.
