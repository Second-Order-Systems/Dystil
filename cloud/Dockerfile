# syntax=docker/dockerfile:1

FROM rust:bookworm AS rust-builder

WORKDIR /app/cloud

RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential ca-certificates pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY . /app

RUN cargo build --release --locked \
    -p work-insights-ingest-api --bin work-insights-api

FROM oven/bun:1.2.9 AS bun-runtime

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libgcc-s1 curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -r app && useradd -r -g app -m app

WORKDIR /app

COPY --from=rust-builder /app/cloud/target/release/work-insights-api /usr/local/bin/work-insights-api
COPY --from=bun-runtime /usr/local/bin/bun /usr/local/bin/bun

COPY cloud/services/auth/package.json cloud/services/auth/bun.lock cloud/services/auth/tsconfig.json /app/cloud/services/auth/
RUN cd /app/cloud/services/auth && bun install --frozen-lockfile --production
COPY cloud/services/auth/src /app/cloud/services/auth/src

COPY cloud/start.sh /start.sh
RUN chmod +x /start.sh

USER app

ENV RUST_LOG=info
ENV AUTH_HOST=127.0.0.1
ENV AUTH_PORT=3001
ENV AUTH_INTERNAL_URL=http://127.0.0.1:3001
ENV WORK_INSIGHTS_BIND_ADDR=0.0.0.0:8089
ENV MEMORY_INTERNAL_URL=http://127.0.0.1:1
ENV MEMORY_INTERNAL_API_TOKEN=placeholder-token-at-least-32-chars

EXPOSE 8089

HEALTHCHECK --interval=30s --timeout=3s --start-period=20s --retries=3 \
    CMD curl -fsS http://localhost:8089/health/auth || exit 1

CMD ["/start.sh"]
