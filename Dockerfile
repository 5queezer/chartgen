# Stage 1: Build (Debian for glibc + freetype/fontconfig dev libs)
FROM rust:1.86-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev libfontconfig1-dev libfreetype-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY tests/ tests/

RUN cargo build --release && strip target/release/chartgen

# Stage 2: Slim runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 libfontconfig1 libfreetype6 fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/chartgen /usr/local/bin/chartgen

EXPOSE 9315

ENV CHARTGEN_BASE_URL=https://chartgen.vasudev.xyz

ENTRYPOINT ["chartgen"]
CMD ["--serve"]
