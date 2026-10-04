# Demo bidder, demo SSP and goose load test, in one image.
# Build context: the repository root (see examples/docker-compose.yml).
FROM rust:1-slim-bookworm AS build
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p demo-bidder -p demo-ssp -p load-test \
 && mkdir -p /out \
 && cp target/release/demo-bidder target/release/demo-ssp target/release/load-test /out/

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl libssl3 \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /out/ /usr/local/bin/
COPY fixtures /fixtures
ENV LOADTEST_FIXTURES=/fixtures
EXPOSE 8080
CMD ["demo-bidder"]
