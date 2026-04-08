FROM rust:1.93-bookworm AS builder

WORKDIR /app
COPY . .

RUN cargo build --release -p crw-server --features cdp -p crw-mcp -p crw-cli

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/crw /usr/local/bin/crw
COPY --from=builder /app/target/release/crw-server /usr/local/bin/crw-server
COPY --from=builder /app/target/release/crw-mcp /usr/local/bin/crw-mcp
COPY config.default.toml /app/config.default.toml
COPY config.docker.toml /app/config.docker.toml

WORKDIR /app

LABEL io.modelcontextprotocol.server.name="io.github.us/crw"

EXPOSE 3000

CMD ["crw-server"]
