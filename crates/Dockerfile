FROM rust:1.89-slim-trixie AS builder

WORKDIR /build

COPY . .
RUN cargo build --release

FROM debian:trixie-slim

RUN mkdir -p /app/data

WORKDIR /app

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# Add relay binary
COPY --from=builder /build/target/release/relay /app/relay
COPY ./run.sh /app/run.sh
COPY ./relay.toml /app/relay.toml
RUN chmod +x /app/run.sh

ENTRYPOINT ["./run.sh"]
