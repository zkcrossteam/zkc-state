FROM docker.io/rust:1.66-alpine as builder
ARG CRATESIOMIRROR=
ARG CARGOPROFILE=release
RUN apk add --no-cache musl-dev protoc protobuf-dev

WORKDIR /usr/src/app

RUN if [ -z "$CRATESIOMIRROR" ]; then exit 0; fi; \
  mkdir -vp "${CARGO_HOME:-$HOME/.cargo}" && \
  printf '[source.crates-io]\nreplace-with = "mirror"\n[source.mirror]\nregistry = "%s"\n' "$CRATESIOMIRROR" | tee -a "${CARGO_HOME:-$HOME/.cargo}/config"

# Cache rust dependencies
# https://stackoverflow.com/questions/58473606/cache-rust-dependencies-with-docker-build
COPY ./rust-toolchain* ./Cargo.* ./
RUN mkdir ./src && echo 'fn main() { println!("Dummy!"); }' > ./src/main.rs && cargo build --profile "$CARGOPROFILE" && rm -rf ./src

COPY ./ ./
RUN cargo install --frozen --offline --profile "$CARGOPROFILE" --path .

FROM docker.io/alpine:3.18
COPY --from=builder /usr/local/cargo/bin/zkc_state_manager /usr/local/bin/myapp
EXPOSE 50051
CMD ["myapp"]

