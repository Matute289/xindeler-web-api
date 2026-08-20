# xindeler-web-api

Backend propio de [`xindeler-web-landing`](https://github.com/Matute289/xindeler-web-landing):
la lista de espera, el formulario de contribuidores, el estado del servidor de juego, y la
sesión web autenticada de la landing.

Reemplaza al backend Python (FastAPI) que corría sin versionar en el VPS — mismo contrato
observable de cara al frontend, pero compilado, versionado, con CI y tests de integración.

## Seguridad y flujo

- Todo el tráfico mutable pasa por acá — nunca directo del frontend a `xindeler-auth`. Esto
  permite revocar una sesión en el mismo request en que se confirma un cambio de contraseña o
  de 2FA (ver `.backlog/SPEC.md`).
- La sesión web (`/api/session/*`) vive acá porque `auth.xindeler.com` es cross-origin respecto
  de la landing y no puede setear una cookie que el frontend pueda usar — `xindeler-web-api` es
  same-origin (`xindeler.com/api/*`).
- `xindeler-auth` sigue siendo la única fuente de verdad de identidad (usuario, contraseña,
  2FA). Este servicio nunca guarda contraseñas ni las valida — reenvía el `password_prehash` que
  ya calcula el frontend (`netPrehash`, wire-compatible con `net_prehash()` de `xindeler-auth`)
  tal cual, como string opaco.
- Depende de `xindeler-auth-common` (tipos de wire, no lógica) vía git — **nunca** de
  `xindeler-authc`, cuyos `sign_in()`/`register()` hashean lo que reciben, y este servicio ya
  recibe el `password_prehash` calculado. Ver `server/src/authclient.rs`.

## Prerrequisito: acceso al repo privado `xindeler-auth`

`xindeler-auth-common` vive en `Matute289/xindeler-auth` (privado). Para compilar local hace
falta una clave SSH con acceso de lectura a ese repo (la tuya, si sos colaborador). El CI usa un
deploy key de solo lectura dedicado (`AUTH_REPO_SSH_KEY`, secret del repo) — mismo patrón que
`xindeler-new-horizon` y `xindeler-zuul` para la misma dependencia. `.cargo/config.toml` fuerza
`git-fetch-with-cli` porque el cliente SSH propio de Cargo falla la autenticación por ssh-agent en
algunos entornos.

## Desarrollo

```sh
# Compilar
cargo build

# Tests (unitarios + integración)
cargo test --all

# Levantar local
WEB_API_BIND_ADDR=127.0.0.1:8020 cargo run -p xindeler-web-api-server
```

### Variables de entorno

| Variable | Default | Descripción |
|---|---|---|
| `WEB_API_BIND_ADDR` | `127.0.0.1:8020` | Dirección donde escucha el servidor |
| `WEB_API_HTTP_WORKERS` | `16` | Threads del pool bloqueante (1–256) |
| `WEB_API_DB_DIR` | `/opt/xindeler-web-api/data/web-api.db` | Ruta del archivo SQLite |
| `WEB_API_GAME_SERVER_ADDR` | `127.0.0.1:14004` | Dirección del game server para `/api/status` |
| `WEB_API_TRUSTED_PROXIES` | `127.0.0.0/8,::1/128` | CIDRs desde donde se confía en `X-Forwarded-For` |
| `WEB_API_RATE_LIMIT_MAX` / `WEB_API_RATE_LIMIT_WINDOW_SECS` | `3` / `3600` | Límite de `/api/waitlist` y `/api/contribute` (por IP) |
| `WEB_API_DIGEST_STATE_PATH` | `/opt/xindeler-web-api/data/digest-last-sent.txt` | Marca de agua del digest mensual |
| `SMTP_HOST`, `SMTP_PORT`, `SMTP_USER`, `SMTP_PASS`, `MAIL_FROM` | — (opcional) | Sin configurar, el envío de mail no-opea en silencio |
| `OWNER_EMAIL` | — (opcional) | Destinatario de notificaciones de contribuidores + digest mensual |
| `AUTH_PUBLIC_URL` | `https://auth.xindeler.com` | Base URL de `xindeler-auth` para `/api/session/login` |
| `AUTH_SERVICE_TOKEN` | — (opcional*) | Mismo secreto de servicio que ya usa el game server contra `xindeler-auth` (`/verify`). *Requerido en la práctica para que el login funcione — sin él, `/api/session/login` responde 500 |
| `WEB_API_SERVICE_TOKEN` | — (opcional*) | Fase F: credencial *nueva*, nunca igual a `AUTH_SERVICE_TOKEN` (el arranque falla si coinciden), que este servicio presenta a `xindeler-auth` en `/issue-character-access-token`. *Requerido en la práctica para `/api/account/characters*` — sin él, esos endpoints responden 500 |
| `WEB_API_GAME_SERVER_PLAYER_API_URL` | `http://127.0.0.1:14005` | Base URL del router `/player_api/v1` del game server (`xindeler-new-horizon` NH-79) — puerto HTTP loopback-only, distinto de `WEB_API_GAME_SERVER_ADDR` (que es el puerto TCP crudo que prueba `/api/status`) |
| `RUST_LOG` | *(sin logs)* | Nivel de log (`env_logger`), ej. `info` |

## Endpoints

| Método | Ruta | Estado |
|---|---|---|
| `GET` | `/ping` | ✅ Fase 0 |
| `GET` | `/api/status` | ✅ Fase 1 |
| `GET` | `/api/waitlist/count` | ✅ Fase 1 |
| `POST` | `/api/waitlist` | ✅ Fase 1 |
| `POST` | `/api/contribute` | ✅ Fase 1 |
| `POST` | `/api/session/login` | ✅ Fase 2 |
| `GET` | `/api/session/me` | ✅ Fase 2 |
| `POST` | `/api/session/logout` | ✅ Fase 2 |
| `GET` | `/api/account/check-username` | ✅ Fase 2 |
| `POST` | `/api/account/change-username` | ✅ Fase 2 |
| `POST` | `/api/account/change-password` | ✅ Fase 2 |
| `POST` | `/api/account/delete` | ✅ Fase 2 |
| `POST` | `/api/account/forgot-password` | ✅ Fase 2 (C-03) |
| `POST` | `/api/account/reset-password` | ✅ Fase 2 (C-03) |
| `POST` | `/api/session/login/2fa` | ✅ Fase 2 (005) |
| `POST` | `/api/account/2fa/enroll` | ✅ Fase 2 (005) |
| `POST` | `/api/account/2fa/confirm` | ✅ Fase 2 (005) |
| `POST` | `/api/account/2fa/disable` | ✅ Fase 2 (005) |
| `POST` | `/api/account/2fa/backup-codes/regenerate` | ✅ Fase 2 (005) |
| `GET` | `/api/account/characters` | ✅ Fase F (NH-79) |
| `POST` | `/api/account/characters/{character_id}/rename` | ✅ Fase F (NH-79) |

Los endpoints de `/api/account/*` que exigen sesión activa (`change-username`, `change-password`,
`delete`, los cuatro de `2fa/*`) usan el `username` de la sesión — nunca uno provisto por el
cliente — y **revocan todas las sesiones de la cuenta** en el mismo request en que `xindeler-auth`
confirma el cambio, forzando un relogin (la excepción es `2fa/confirm`: activar 2FA no reduce la
seguridad de la cuenta, así que no fuerza relogin). Si `xindeler-auth` rechaza el cambio
(contraseña actual incorrecta, código TOTP inválido, etc.), la sesión sigue viva sin tocar.

`check-username`, `forgot-password`, `reset-password` y `session/login/2fa` **no** requieren
sesión — son los flujos de "todavía no puedo loguearme" (incluido el segundo factor del login).
`forgot-password` siempre responde `200 {ok:true}`, exista o no la cuenta (anti-enumeración, igual
que `xindeler-auth`). `reset-password` tiene una limitación conocida: `xindeler-auth` resuelve el
`uuid` de la cuenta internamente para aplicar el reset pero no lo devuelve, así que este servicio
no puede revocar *todas* las sesiones de la cuenta en ese request como sí hace con
`change-password` — forzarle ese cambio de contrato a `xindeler-auth` queda fuera de alcance (ver
`.backlog/SPEC.md`). El TTL de 7 días de la cookie es la mitigación real de este hueco; como bonus
sin costo, si el llamado a `reset-password` todavía trae una cookie de sesión válida (p. ej.
reseteando desde una pestaña que ya estaba logueada), esa sesión puntual sí se revoca.

**2FA (Fase L de `xindeler-auth`, tarea 005 de `xindeler-web-landing`):** cuando la cuenta tiene
TOTP confirmado, `POST /api/session/login` responde `202 { challenge_id, expires_in }` en vez de
crear la sesión directamente — recién `POST /api/session/login/2fa` (con el código de la app
autenticadora) completa el login y deja la cookie. `GET /api/session/me` expone `totp_enabled` —
estado **derivado**, no consultado a `xindeler-auth` (Fase L no expone ningún endpoint de "estado
de TOTP"): se infiere de si el login pasó por el challenge, y se actualiza en cada
`2fa/confirm`/`2fa/disable` exitoso a través de este proxy (ver `totp_status.rs`). Los errores
específicos de TOTP (`TOTP_INVALID_CODE`, `ACCOUNT_2FA_LOCKED`, etc.) se reenvían con el mismo
`code`/`message` que devuelve `xindeler-auth`, en vez de colapsarse a un error genérico — mismo
criterio que ya se usaba para `EMAIL_VERIFICATION_REQUIRED`.

**Personajes (Fase F, NH-79):** ambos endpoints exigen sesión activa y siguen un flujo de tres
saltos por request, sin cachear nada entre acciones — `resolve_session` → pedirle a `xindeler-auth`
un `CharacterAccessToken` acotado (60s, un solo uso) vía `WEB_API_SERVICE_TOKEN` → reenviar ese
token al game server (`player_api/v1`, loopback-only por NH-75). El game server responde texto
plano en sus rechazos (no el `{code, message}` de `xindeler-auth`), así que un 409 de rename
(nombre repetido, personaje inexistente/ajeno, nombre inválido) se reenvía con `code
CHARACTER_ACTION_REJECTED` y el mensaje textual tal cual, sin intentar distinguir el motivo exacto
por más granularidad server-side.

## Subcomandos CLI

```sh
# Digest mensual (systemd timer) — no-opea si falta SMTP/OWNER_EMAIL
xindeler-web-api-server digest

# Migración one-shot de los CSV del servicio Python — idempotente
xindeler-web-api-server migrate-csv waitlist.csv contributors.csv
```

## Docker

```sh
docker compose up --build
```

Corre local con la misma imagen hardened (`read_only`, `cap_drop: ALL`, usuario no-root) que se
usaría en cualquier entorno containerizado. **En producción corre bajo systemd, no Docker** —
mismo criterio que `xindeler-auth` (ver `docs/OPERACION.md` de ese repo); Docker acá es para
paridad de desarrollo/testing, no para el deploy real.
