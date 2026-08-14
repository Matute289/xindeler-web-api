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
| `RUST_LOG` | *(sin logs)* | Nivel de log (`env_logger`), ej. `info` |

Fase 1 agrega variables de SMTP y base de datos; Fase 2 agrega `AUTH_SERVICE_TOKEN` para hablar
con `xindeler-auth`. Se documentan acá a medida que existan.

## Endpoints

| Método | Ruta | Estado |
|---|---|---|
| `GET` | `/ping` | ✅ Fase 0 |
| `GET` | `/api/status` | ⏳ Fase 1 |
| `GET` | `/api/waitlist/count` | ⏳ Fase 1 |
| `POST` | `/api/waitlist` | ⏳ Fase 1 |
| `POST` | `/api/contribute` | ⏳ Fase 1 |
| `POST` | `/api/session/login`, `/logout`, `GET /me` | ⏳ Fase 2 |
| `POST` | `/api/account/*` (proxy autenticado a `xindeler-auth`) | ⏳ Fase 2 |

## Docker

```sh
docker compose up --build
```

Corre local con la misma imagen hardened (`read_only`, `cap_drop: ALL`, usuario no-root) que se
usaría en cualquier entorno containerizado. **En producción corre bajo systemd, no Docker** —
mismo criterio que `xindeler-auth` (ver `docs/OPERACION.md` de ese repo); Docker acá es para
paridad de desarrollo/testing, no para el deploy real.
