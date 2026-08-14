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
| A-02 | Hardening del repo GitHub (ruleset, secret scanning, dependabot, vulnerability alerts, `SECURITY.md`, `CODEOWNERS`) | `in-progress` |
| A-03 | Scaffold Cargo workspace (`common`/`server`), seam HTTP framework-agnóstico copiado de `xindeler-auth` | `done` — `GET /ping` funcionando, build/test/clippy/fmt en verde |
| A-04 | CI (`check.yml`) + `scripts/check-{unsafe-policy,docs,docker-context}.sh` invocados desde CI | `done` |
| A-05 | `Dockerfile` + `docker-compose.yml` hardened (paridad dev/testing, no reemplaza systemd en prod) | `done` |
| A-06 | `CLAUDE.md`/`AGENTS.md` + `.backlog/{README,SPEC,PLAN}.md` | `in-progress` |
| A-07 | Skills (`xindeler-web-api-dev`, `xindeler-web-api-architect`) y agentes reviewers dedicados | `todo` |

## Fase B — Paridad funcional (planeada)

| ID | Tarea | Estado |
|---|---|---|
| B-01 | Migrar `waitlist`/`contribute`/`status`/`count` a SQLite + Rust | `todo` |
| B-02 | Corregir los bugs relevados del Python actual (dedup antes de rate-limit, `count` roto en nginx, HTML sin escapar en emails, rate limiter que no evictea claves, I/O bloqueante, sin locking de CSV) | `todo` |
| B-03 | Portar `monthly-digest.py` (systemd timer mensual) | `todo` |
| B-04 | Migrar los 7 registros reales de `waitlist.csv`/`contributors.csv` | `todo` |
| B-05 | Deploy en puerto nuevo, smoke-test directo, corte de nginx, apagar `xindeler-waitlist.service` | `todo` |

## Fase C — Sesión web autenticada (planeada)

| ID | Tarea | Estado |
|---|---|---|
| C-01 | Tabla `sessions` + `POST /api/session/login`, `/login/2fa`, `GET /me`, `POST /logout` | `todo` |
| C-02 | Proxy autenticado `/api/account/*` hacia `xindeler-auth` vía `xindeler-authc` | `todo` |
| C-03 | Reroute `ForgotPasswordPage`/`ResetPasswordPage` de `xindeler-web-landing` a través de acá | `todo` |
| C-04 | Coordinar `AUTH_SERVICE_TOKEN` con `xindeler-auth` (reusar el del game server, o pedir credencial separada) | `todo` |

## Fase D — Frontend consume la sesión (planeada)

| ID | Tarea | Estado |
|---|---|---|
| D-01 | Env vars + proxy de Vite en `xindeler-web-landing` para desarrollo local real | `todo` |
| D-02 | `AuthModal.jsx` deja de descartar el resultado del login, primera versión de "hay alguien logueado" | `todo` |

---

Detalle completo de decisiones de diseño en `SPEC.md`, plan de trabajo con próximos pasos
concretos en `PLAN.md`.
