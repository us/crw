# Cross-compiling multi-arch build with a durable cargo-chef dependency layer.
#
# The builder runs on the NATIVE build platform (`--platform=$BUILDPLATFORM`)
# and cross-compiles to the requested target arch. Previously the builder ran
# under QEMU for linux/arm64, which emulated the *entire* Rust compile and took
# ~2h per release. Cross-compiling on the native runner brings arm64 back to
# minutes; only the tiny runtime layer (ca-certificates) still touches QEMU.
#
# cargo-chef splits the expensive external-crate compile (aws-lc-sys, chromium/
# CDP, tokio, rustls, ...) into its own `cacher` stage keyed ONLY on the Cargo
# manifests (recipe.json). A source-only change leaves recipe.json byte-
# identical -> the cook is reused and only the workspace crates recompile.
#
# DURABILITY: prod (rolling-deploy.sh) bakes the `cacher` stage as a TAGGED
# image `crw-api-deps:<fp>` and overrides DEPS_STAGE to it, so `builder` runs
# `FROM crw-api-deps:<fp>`. Tagged image layers are referenced (not dangling)
# and are NOT build cache, so the nightly `docker image prune -f` +
# `docker builder prune --keep-storage 1GB` cannot evict them. CI leaves
# DEPS_STAGE=cacher (cooks inline; gha mode=max caches the cacher layer).

# Global ARG (before the first FROM) so it is usable in the `builder` FROM line.
ARG DEPS_STAGE=cacher

# ---- shared toolchain base --------------------------------------------------
FROM --platform=$BUILDPLATFORM rust:1.96-bookworm@sha256:a339861ae23e9abb272cea45dfafde21760d2ce6577a70f8a926153677902663 AS chef

# Provided automatically by buildx: amd64 | arm64.
ARG TARGETARCH
WORKDIR /app

# Rust target + (arm64) cross linker toolchain; record the target triple.
RUN set -eux; \
    case "$TARGETARCH" in \
      amd64) RUST_TARGET=x86_64-unknown-linux-gnu ;; \
      arm64) RUST_TARGET=aarch64-unknown-linux-gnu; \
             apt-get update; \
             # crossbuild-essential-arm64 = the aarch64 gcc/g++ AND the target
             # libc dev headers (libc6-dev-arm64-cross). The bare cross gcc
             # alone lacks sys/types.h etc., which broke aws-lc-sys's C build.
             apt-get install -y --no-install-recommends crossbuild-essential-arm64; \
             rm -rf /var/lib/apt/lists/* ;; \
      *) echo "unsupported TARGETARCH=$TARGETARCH" >&2; exit 1 ;; \
    esac; \
    rustup target add "$RUST_TARGET"; \
    echo "$RUST_TARGET" > /rust_target

# cargo-chef as a host (BUILDPLATFORM) tool. Pinned so the install layer is
# deterministic; must be a release that parses this edition-2024 / resolver-2
# workspace (verify `cargo chef prepare` succeeds before merge — see PR checklist).
RUN cargo install cargo-chef --locked --version 0.1.77

# Shared build env — MUST be identical in `cacher` (cook) and `builder` (final
# compile) or the cooked deps get a different cargo fingerprint and recompile.
# Baked into `chef` so both inherit it verbatim (and so the tagged deps image
# carries it too).
#   - Linker for the aarch64 cross target (ignored on native amd64).
#   - Workspace release profile is fat LTO + codegen-units=1, whose final link
#     of crw-server (aws-lc-sys + full graph) needs several GB and OOM-killed
#     the build (#90). thin LTO + 16 CGUs: far lower peak memory, faster link,
#     negligible runtime difference.
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CARGO_PROFILE_RELEASE_LTO=thin \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16

# ---- planner: derive the dependency recipe from the manifests only ----------
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path /recipe.json

# ---- cacher: compile ONLY the dependency graph (the durable, taggable layer) -
# Keyed solely on /recipe.json; SAME --target/--features/-p set as the real
# build so the cooked artifacts fingerprint-match and are reused, not recompiled.
FROM chef AS cacher
COPY --from=planner /recipe.json /recipe.json
RUN set -eux; \
    RUST_TARGET="$(cat /rust_target)"; \
    cargo chef cook --release --target "$RUST_TARGET" \
      -p crw-server --features cdp -p crw-mcp -p crw-cli \
      --recipe-path /recipe.json

# ---- builder: compile the workspace crates on top of the cooked deps --------
# DEPS_STAGE defaults to the in-tree `cacher` (CI / any plain `docker build`
# cooks deps inline). Prod overrides it to the pinned crw-api-deps:<fp> image.
FROM ${DEPS_STAGE} AS builder
COPY . .
RUN set -eux; \
    RUST_TARGET="$(cat /rust_target)"; \
    cargo build --release --target "$RUST_TARGET" \
      -p crw-server --features cdp -p crw-mcp -p crw-cli; \
    mkdir -p /out; \
    cp "target/${RUST_TARGET}/release/crw" \
       "target/${RUST_TARGET}/release/crw-server" \
       "target/${RUST_TARGET}/release/crw-mcp" /out/

# ---- runtime (unchanged) ----------------------------------------------------
FROM debian:bookworm-slim@sha256:60eac759739651111db372c07be67863818726f754804b8707c90979bda511df

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /out/crw /usr/local/bin/crw
COPY --from=builder /out/crw-server /usr/local/bin/crw-server
COPY --from=builder /out/crw-mcp /usr/local/bin/crw-mcp
COPY config.default.toml /app/config.default.toml
COPY config.docker.toml /app/config.docker.toml

WORKDIR /app

LABEL io.modelcontextprotocol.server.name="io.github.us/crw"

EXPOSE 3000

CMD ["crw-server"]
