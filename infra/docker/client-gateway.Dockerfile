# syntax=docker/dockerfile:1.7
# client-gateway — çok aşamalı imaj: cargo-chef bağımlılık önbelleği + distroless/cc, root olmayan.
# Bağlam: depo kökü.  docker build -f infra/docker/client-gateway.Dockerfile -t fxvps-client-gateway .
# DURUM: BEKLEMEDE (pending) — services/client-gateway başka bir dalda geliştiriliyor; main'e
# girene kadar bu Dockerfile derlenmez. docker.yml dosyası dizin yoksa bu imajı atlar.
ARG RUST_VERSION=1.87

FROM lukemathwalker/cargo-chef:latest-rust-${RUST_VERSION}-bookworm AS chef
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG CARGO_FEATURES=""
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json -p client-gateway ${CARGO_FEATURES:+--features $CARGO_FEATURES}
COPY . .
RUN cargo build --release --locked -p client-gateway --bin client-gateway ${CARGO_FEATURES:+--features $CARGO_FEATURES}

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
WORKDIR /app
COPY --from=builder /src/target/release/client-gateway /app/client-gateway
COPY --from=builder /src/services/client-gateway/config /app/config
USER nonroot:nonroot
ENV RUST_LOG=info
ENTRYPOINT ["/app/client-gateway"]
CMD ["/app/config/default.toml"]
