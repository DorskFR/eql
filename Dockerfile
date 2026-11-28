FROM rust:1.96-alpine AS rust-builder
RUN apk add --no-cache musl-dev
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p eqls

FROM node:26.12.1-slim AS web-builder
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web ./
RUN npm run build

FROM gcr.io/distroless/static-debian13:nonroot AS runtime
COPY --from=rust-builder /build/target/release/eqls /usr/local/bin/eqls
COPY --from=web-builder /web/build /app/web/build
EXPOSE 8080
ENV LISTEN_ADDR=0.0.0.0:8080
ENV WEB_DIST=/app/web/build
CMD ["eqls"]
