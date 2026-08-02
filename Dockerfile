FROM rust:1.97-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY web ./web
RUN cargo build --locked --release

FROM debian:bookworm-slim AS runtime
LABEL org.opencontainers.image.source="https://github.com/josusanmartin/basis-lab" \
      org.opencontainers.image.title="Basis Lab" \
      org.opencontainers.image.description="Cross-venue OHLC premium and discount explorer" \
      org.opencontainers.image.licenses="MIT"
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home basis
WORKDIR /app
COPY --from=builder /app/target/release/basis-lab /usr/local/bin/basis-lab
COPY --from=builder /app/web ./web
USER 10001
ENV PORT=8080 RUST_LOG=basis_lab=info,tower_http=warn
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 CMD curl -fsS http://127.0.0.1:8080/api/v1/health || exit 1
ENTRYPOINT ["basis-lab"]
