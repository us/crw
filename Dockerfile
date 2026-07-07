# Cross-compiling multi-arch build.
#
# The builder runs on the NATIVE build platform (`--platform=$BUILDPLATFORM`)
# and cross-compiles to the requested target arch. Previously the builder ran
# under QEMU for linux/arm64, which emulated the *entire* Rust compile and took
# ~2h per release. Cross-compiling on the native runner brings arm64 back to
# minutes; only the tiny runtime layer (ca-certificates) still touches QEMU.
FROM --platform=$BUILDPLATFORM rust:1.93-bookworm@sha256:7c4ae649a84014c467d79319bbf17ce2632ae8b8be123ac2fb2ea5be46823f31 AS builder

# Provided automatically by buildx: amd64 | arm64.
ARG TARGETARCH

WORKDIR /app

# Install the Rust target + (for arm64) the cross linker, and record the
# rustc target triple for the build step.
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

COPY . .

# Linker for the aarch64 cross target (ignored when building amd64 natively).
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc

# The workspace release profile uses fat LTO + codegen-units=1, whose final
# link of crw-server (aws-lc-sys + the full dep graph) needs several GB and
# OOM-killed the docker build (see #90's 4 GB OOM warning). The container
# binary doesn't need max LTO, so use thin LTO across more codegen units —
# far lower peak memory and a faster link, negligible runtime difference.
ENV CARGO_PROFILE_RELEASE_LTO=thin \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16

RUN set -eux; \
    RUST_TARGET="$(cat /rust_target)"; \
    cargo build --release --target "$RUST_TARGET" \
      -p crw-server --features cdp -p crw-mcp -p crw-cli; \
    mkdir -p /out; \
    cp "target/${RUST_TARGET}/release/crw" \
       "target/${RUST_TARGET}/release/crw-server" \
       "target/${RUST_TARGET}/release/crw-mcp" /out/

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
