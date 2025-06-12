FROM rust:slim AS rust

FROM rust AS base
RUN cargo install cargo-chef 

ENV SKIP_CLIENT_BUILD=true
WORKDIR /usr/src/webnerf

# prepare deps
FROM base AS plan
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM base AS build-server
COPY --from=plan /usr/src/webnerf/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .

# compile server
RUN cargo build --bin webnerf --release

FROM debian:bookworm-slim AS runtime
WORKDIR /webnerf
COPY --from=build-server /usr/src/webnerf/target/release/webnerf webnerf
COPY --from=build-server /usr/src/webnerf/static static

EXPOSE 8080
CMD ["./webnerf"]
