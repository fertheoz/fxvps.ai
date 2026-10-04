# syntax=docker/dockerfile:1.7
# lp-simulator — çok aşamalı imaj: cargo-chef bağımlılık önbelleği + distroless/cc, root olmayan.
# Bağlam: depo kökü.  docker build -f infra/docker/lp-simulator.Dockerfile -t fxvps-lp-simulator .
ARG RUST_VERSION=1.87

FROM lukemathwalker/cargo-chef:latest-rust-${RUST_VERSION}-bookworm AS chef
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG CARGO_FEATURES=""
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json -p lp-simulator ${CARGO_FEATURES:+--features $CARGO_FEATURES}
COPY . .
RUN cargo build --release --locked -p lp-simulator --bin lp-simulator ${CARGO_FEATURES:+--features $CARGO_FEATURES}

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
WORKDIR /app
COPY --from=builder /src/target/release/lp-simulator /app/lp-simulator
COPY --from=builder /src/services/lp-simulator/config /app/config
USER nonroot:nonroot
ENV RUST_LOG=info
ENTRYPOINT ["/app/lp-simulator"]
CMD ["/app/config/default.toml"]
