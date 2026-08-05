FROM rust:1.96-slim AS rust-builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p eqls

FROM node:24-slim AS web-builder
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web ./
RUN npm run build

FROM debian:trixie-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 eqls
COPY --from=rust-builder /build/target/release/eqls /usr/local/bin/eqls
COPY --from=web-builder /web/build /app/web/build
USER eqls
EXPOSE 8080
ENV LISTEN_ADDR=0.0.0.0:8080
ENV WEB_DIST=/app/web/build
CMD ["eqls"]
