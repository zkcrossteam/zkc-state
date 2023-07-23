FROM docker.io/rust:1.66-alpine as builder

RUN apk add --no-cache musl-dev protoc protobuf-dev

WORKDIR /usr/src/app

# Cache rust dependencies
# https://stackoverflow.com/questions/58473606/cache-rust-dependencies-with-docker-build
COPY ./rust-toolchain* ./Cargo.* ./
RUN mkdir ./src && echo 'fn main() { println!("Dummy!"); }' > ./src/main.rs && cargo build --release && rm -rf ./src

COPY ./ ./
RUN cargo install --frozen --offline --path .

FROM docker.io/alpine:3.18
COPY --from=builder /usr/local/cargo/bin/zkc_state_manager /usr/local/bin/myapp
EXPOSE 50051
CMD ["myapp"]

