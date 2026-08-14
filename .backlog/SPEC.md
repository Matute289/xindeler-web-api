# Spec — xindeler-web-api

## Objetivo

Un backend versionado, compilado y auditable que reemplace por completo al FastAPI de Python que
hoy vive sin versionar en el VPS (`/srv/xindeler/waitlist-api/main.py`), y que además resuelva la
sesión web autenticada de `xindeler-web-landing` — origen: tarea 007 del backlog de ese repo.

## Por qué existe (contexto completo en `xindeler-web-landing/.backlog/tasks/007-sesion-web-autenticada.md`)

- `xindeler-auth` es stateless por diseño: su `AuthToken` dura 15s, se consume una sola vez, y no
  fue pensado como credencial de sesión de browser.
- `auth.xindeler.com` es cross-origin respecto de la landing y no manda
  `Access-Control-Allow-Credentials` — estructuralmente no puede setear una cookie que el
  frontend pueda usar.
- El backend actual de la landing (`waitlist-api`) sí es same-origin (`xindeler.com/api/*`), pero
  vive sin versionar, sin CI, y persiste en CSVs sin locking.

Decisión (2026-08-14, Matías): en vez de parchear el Python en producción, reescribir en Rust,
repo propio, absorbiendo tanto los endpoints existentes como la sesión nueva.

## Decisiones de diseño

### Por qué Rust y no Go

`xindeler-auth` (el otro backend del ecosistema) y `xindeler-new-horizon` (el motor del juego)
son ambos Rust. `~/Workspace/RustroverProjects/` tiene 7 proyectos Xindeler, todos Rust;
`GolandProjects/` no tiene ninguno. Rust además permite consumir `xindeler-authc` directamente
como dependencia de Cargo para hablar con `xindeler-auth`, sin reimplementar HTTP a mano.

### Por qué clonar la arquitectura de `xindeler-auth` casi 1:1

No es solo "un ejemplo similar" — es el mismo Matías administrando el mismo ecosistema, y sus
decisiones ya están batalladas contra producción real (su Fase H de backlog es una auditoría de
seguridad de 26 ítems, todos `done`). El seam HTTP framework-agnóstico
(`http/mod.rs`+`http/axum.rs`), el split de errores interno/público, la config con `OnceLock` y
validación de rangos, y el patrón de tests de integración con `TestServer` se copian casi
textuales — ver `xindeler-web-api-dev` (skill) para el detalle archivo por archivo.

### Por qué `netPrehash` nunca se toca acá

El frontend ya calcula el prehash (Argon2i + salt FxHash64) en JS antes de mandar la contraseña,
con un vector dorado verificado en ambos lados (`xindeler-web-landing/src/lib/netPrehash.test.js`
↔ `xindeler-auth/common/src/lib.rs`). Recalcularlo o validarlo acá sería una tercera
implementación del mismo algoritmo — puro riesgo de drift sin ningún beneficio. Este servicio lo
recibe como string opaco y lo reenvía.

### Por qué SQLite (Fase 1+) y no otra cosa

Volumen real medido en el VPS: 6 filas en `waitlist.csv`, 1 en `contributors.csv`. Cientos de
sesiones concurrentes como mucho para la Fase 2. Trivial para SQLite en el VPS actual
(2 vCPU / 4 GB) — mismo patrón que `xindeler-auth` ya usa en producción, sin agregar un
componente de infraestructura nuevo (no Postgres, no Redis).

## Requerimientos no funcionales

- Contrato de comportamiento observable idéntico al Python actual para los endpoints migrados
  (mismos status codes, mismo shape de respuesta) — el frontend no debe notar el cambio.
- `cargo build`, `cargo test --all`, `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt --all -- --check` en verde en cada PR.
- `#![forbid(unsafe_code)]` en todos los crates.

## Integraciones externas

### `xindeler-web-landing` (frontend)

Único consumidor de `/api/*`. Ver Restricciones en el plan de Fase 0 (netPrehash, sin cookies
hoy, URLs hardcodeadas — Fase 3 introduce env vars + proxy de Vite).

### `xindeler-auth` (identidad)

Fase 2 consume `xindeler-authc` (crate) para `register`/`sign_in`/`validate_full`/
`username_to_uuid`/`uuid_to_username`, autenticado con `AUTH_SERVICE_TOKEN` (mismo secreto que ya
usa el game server — ver "Coordinación cross-repo" en el plan de la Fase 0). Los endpoints que
`authc` no cubre (`check-username`, `forgot-password`, `reset-password`, `account-email`,
`resend-verification`) se llaman por HTTP directo, mismo patrón server-to-server.

## Gaps conocidos vs. producción robusta

- **Fase 0** (actual): sin lógica de negocio, sin persistencia, sin secrets. Solo `/ping`.
- Bugs conocidos del Python actual que la reescritura corrige, no porta — ver la sección
  "Migración desde el FastAPI actual" del plan de Fase 0 (dedup antes de rate-limit,
  `/api/waitlist/count` roto en nginx hoy, HTML sin escapar en emails, rate limiter que no
  evictea, I/O bloqueante, sin locking de CSV).
- `AUTH_SERVICE_TOKEN` compartido con el game server (Fase 2): revisar si conviene una
  credencial separada y revocable una vez que haga falta revocación independiente — hoy
  `xindeler-auth` solo soporta un secreto de servicio.
