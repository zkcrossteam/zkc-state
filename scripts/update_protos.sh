#!/usr/bin/env bash

script_dir="$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
top_dir="$(dirname "$script_dir")"

# Generate the gRPC code for gateway.
# Note that we must copy the files instead of making symlinks because, otherwise,
# we may not be able to build the container with only the files in "$dir"
pushd "$top_dir/gateway" || exit 1
protoc -I "../proto"  --go_out gen --go_opt paths=source_relative --go-grpc_out gen --go-grpc_opt paths=source_relative --grpc-gateway_out gen --grpc-gateway_opt paths=source_relative --grpc-gateway_opt generate_unbound_methods=true "../proto/kvpair.proto"
popd || exit 1