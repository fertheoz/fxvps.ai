# syntax=docker/dockerfile:1.7
# identity — çok aşamalı imaj: cargo-chef bağımlılık önbelleği + distroless/cc, root olmayan.
# Bağlam: depo kökü.  docker build -f infra/docker/identity.Dockerfile -t fxvps-identity .
# Yapılandırma tamamen ortam değişkenleriyle (services/identity/README.md); imaja
# yapılandırma dizini ya da anahtar kopyalanmaz. Göçler (migrations/) ikiliye gömülüdür.
ARG RUST_VERSION=1.90.0

FROM lukemathwalker/cargo-chef:latest-rust-${RUST_VERSION}-bookworm AS chef
ARG RUST_VERSION
ENV RUSTUP_TOOLCHAIN=${RUST_VERSION}
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json -p identity
COPY . .
RUN cargo build --release --locked -p identity --bin identity

FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f AS runtime
WORKDIR /app
COPY --from=builder /src/target/release/identity /app/identity
USER nonroot:nonroot
ENV RUST_LOG=info \
    IDENTITY_LISTEN=0.0.0.0:8090
EXPOSE 8090
# Üretimde zorunlu: DATABASE_URL, IDENTITY_SIGNING_KEY_FILE (secret mount), IDENTITY_ISSUER,
# IDENTITY_RP_ID / IDENTITY_RP_ORIGIN / IDENTITY_ALLOWED_ORIGINS, IDENTITY_SERVICE_TOKEN.
ENTRYPOINT ["/app/identity"]
