# syntax=docker/dockerfile:1.7

# Multi-stage build for atl. The runtime image is distroless/cc so the
# attack surface is minimal (no shell, nonroot user). Authentication in a
# container relies on the ATL_API_TOKEN env var — the keyring backend
# returns Ok(None) on headless Linux and falls through to the env var.
#
# Build acceleration:
#   - BuildKit cache mounts persist cargo registry/git and target/ across
#     builds, exported via cache-to=type=gha,mode=max in the workflow.
#   - The binary is copied out of the cache mount before the layer ends
#     so the final image stays tiny (~48 MB).

ARG RUST_VERSION=1.95.0
ARG DEBIAN_CODENAME=bookworm

FROM rust:${RUST_VERSION}-slim-${DEBIAN_CODENAME} AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

COPY Cargo.toml Cargo.lock rust-toolchain.toml LICENSE ./
COPY src ./src

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked \
    && cp target/release/atl /usr/local/bin/atl \
    && strip /usr/local/bin/atl

FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /usr/local/bin/atl /usr/local/bin/atl
COPY --from=builder /src/LICENSE /LICENSE

USER nonroot:nonroot

ENTRYPOINT ["/usr/local/bin/atl"]
