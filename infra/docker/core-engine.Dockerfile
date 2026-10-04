# syntax=docker/dockerfile:1.7
# core-engine — çok aşamalı imaj: cargo-chef bağımlılık önbelleği + distroless/cc, root olmayan.
# Bağlam: depo kökü.  docker build -f infra/docker/core-engine.Dockerfile -t fxvps-core-engine .
# DURUM: BEKLEMEDE (pending) — services/core-engine başka bir dalda geliştiriliyor; main'e
# girene kadar bu Dockerfile derlenmez. docker.yml dosyası dizin yoksa bu imajı atlar.
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
ARG CARGO_FEATURES=""
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json -p core-engine ${CARGO_FEATURES:+--features $CARGO_FEATURES}
COPY . .
RUN cargo build --release --locked -p core-engine --bin core-engine ${CARGO_FEATURES:+--features $CARGO_FEATURES}

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
WORKDIR /app
COPY --from=builder /src/target/release/core-engine /app/core-engine
COPY --from=builder /src/services/core-engine/config /app/config
USER nonroot:nonroot
ENV RUST_LOG=info
ENTRYPOINT ["/app/core-engine"]
CMD ["/app/config/default.toml"]
