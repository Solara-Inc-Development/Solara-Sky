FROM rust:1.85-bookworm AS builder
WORKDIR /build
COPY . .
RUN cargo build --release --bin solara-sky

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /data
COPY --from=builder /build/target/release/solara-sky /usr/local/bin/solara-sky
EXPOSE 25565/tcp
ENTRYPOINT ["/usr/local/bin/solara-sky", "--config-dir", "/data"]
