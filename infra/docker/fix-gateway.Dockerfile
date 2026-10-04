# syntax=docker/dockerfile:1.7
# fix-gateway — çok aşamalı imaj: cargo-chef bağımlılık önbelleği + distroless/cc, root olmayan.
# Bağlam: depo kökü.  docker build -f infra/docker/fix-gateway.Dockerfile -t fxvps-fix-gateway .
# Tam sürüm: rust-toolchain.toml "stable" der; RUSTUP_TOOLCHAIN onu ezer ki derleme
# imajdaki pinli toolchain ile, ağdan kanal güncellemesi çekmeden koşsun.
ARG RUST_VERSION=1.90.0

FROM lukemathwalker/cargo-chef:latest-rust-${RUST_VERSION}-bookworm AS chef
ARG RUST_VERSION
ENV RUSTUP_TOOLCHAIN=${RUST_VERSION}
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG CARGO_FEATURES="nats"
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json -p fix-gateway ${CARGO_FEATURES:+--features $CARGO_FEATURES}
COPY . .
RUN cargo build --release --locked -p fix-gateway --bin fix-gateway ${CARGO_FEATURES:+--features $CARGO_FEATURES}

FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f AS runtime
WORKDIR /app
COPY --from=builder /src/target/release/fix-gateway /app/fix-gateway
COPY --from=builder /src/services/fix-gateway/config /app/config
USER nonroot:nonroot
ENV RUST_LOG=info
ENTRYPOINT ["/app/fix-gateway"]
CMD ["/app/config/default.toml"]
