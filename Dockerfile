FROM rust:1.83-bookworm AS builder

WORKDIR /app
COPY . .

RUN cargo build --release --bin crw-server --features crw-server/cdp

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/crw-server /usr/local/bin/crw-server
COPY config.default.toml /app/config.default.toml

WORKDIR /app

EXPOSE 3000

CMD ["crw-server"]
