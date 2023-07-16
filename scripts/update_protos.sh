#!/usr/bin/env bash

script_dir="$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
top_dir="$(dirname "$script_dir")"
proto_dir="$top_dir/proto"

cd "$top_dir"

# Update descriptor sets for envoy.
"${PROTOC:-protoc}" -Iproto -I. --include_imports --include_source_info --descriptor_set_out=server/envoy/proto/kvpair.pb proto/kvpair.proto

for dir in services/*; do
    cp -r "$proto_dir" "$dir";
done
