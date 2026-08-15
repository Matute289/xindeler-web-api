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
| `RUST_LOG` | *(sin logs)* | Nivel de log (`env_logger`), ej. `info` |

Fase 2 agrega `AUTH_SERVICE_TOKEN` para hablar con `xindeler-auth`.

## Endpoints

| Método | Ruta | Estado |
|---|---|---|
| `GET` | `/ping` | ✅ Fase 0 |
| `GET` | `/api/status` | ✅ Fase 1 |
| `GET` | `/api/waitlist/count` | ✅ Fase 1 |
| `POST` | `/api/waitlist` | ✅ Fase 1 |
| `POST` | `/api/contribute` | ✅ Fase 1 |
| `POST` | `/api/session/login`, `/logout`, `GET /me` | ⏳ Fase 2 |
| `POST` | `/api/account/*` (proxy autenticado a `xindeler-auth`) | ⏳ Fase 2 |

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
