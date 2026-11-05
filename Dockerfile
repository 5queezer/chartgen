# Stage 1: Build (Debian for glibc + freetype/fontconfig dev libs).
# Same Debian release as the runtime stage so the binary's glibc matches.
FROM rust:1.95-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev libfontconfig1-dev libfreetype-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY tests/ tests/

RUN cargo build --release && strip target/release/chartgen

# Stage 2: Web app (SolidJS, ADR-0027)
FROM node:22-bookworm-slim AS web

WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
# Empty base URL: the bundle calls MCP/OAuth on its own origin.
ENV VITE_CHARTGEN_BASE_URL=""
RUN npm run build

# Stage 3: Slim runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 libfontconfig1 libfreetype6 fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/chartgen /usr/local/bin/chartgen
COPY --from=web /web/dist /usr/share/chartgen/web

# Serve the web app at / for browsers (ADR-0027).
ENV CHARTGEN_WEB_DIR=/usr/share/chartgen/web

EXPOSE 9315

ENTRYPOINT ["chartgen"]
CMD ["--serve"]
