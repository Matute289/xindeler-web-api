FROM rust:1.96-bookworm AS builder

WORKDIR /build
COPY . .
RUN cargo build --locked --release -p xindeler-web-api-server

FROM debian:12-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10002 xindeler \
    && useradd --uid 10002 --gid 10002 --no-create-home --shell /usr/sbin/nologin xindeler \
    && install -d -o 10002 -g 10002 /opt/xindeler-web-api/data

COPY --from=builder --chown=10002:10002 /build/target/release/xindeler-web-api-server /usr/local/bin/xindeler-web-api-server

USER 10002:10002
EXPOSE 8020
ENTRYPOINT ["/usr/local/bin/xindeler-web-api-server"]
