# syntax=docker/dockerfile:1.7

# Multi-stage build for atl. The runtime image is distroless/cc so the
# attack surface is minimal (no shell, nonroot user). Authentication in a
# container relies on the ATL_API_TOKEN env var — the keyring backend
# returns Ok(None) on headless Linux and falls through to the env var.

ARG RUST_VERSION=1.95.0
ARG DEBIAN_CODENAME=bookworm

FROM rust:${RUST_VERSION}-slim-${DEBIAN_CODENAME} AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config \
        libdbus-1-dev \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Cache dependencies separately from the source tree.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
RUN mkdir src \
    && echo 'fn main() {}' > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src target/release/atl target/release/atl.d target/release/deps/atl-*

COPY src ./src
COPY LICENSE ./

RUN cargo build --release --locked \
    && strip target/release/atl

FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /src/target/release/atl /usr/local/bin/atl
COPY --from=builder /src/LICENSE /LICENSE

USER nonroot:nonroot

ENTRYPOINT ["/usr/local/bin/atl"]
