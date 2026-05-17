# Build stage
FROM rust:1.93-slim AS builder
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release -p teramind-sync-server

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/target/release/teramind-sync-server /usr/local/bin/
RUN useradd -r -u 10001 teramind
USER teramind
EXPOSE 8443
ENTRYPOINT ["/usr/local/bin/teramind-sync-server"]
CMD ["serve"]
