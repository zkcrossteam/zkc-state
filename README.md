`zkc_state_manager` is a rust program to manage zkcross states.

Users may use [gRPC](https://grpc.io/) or [REST](https://en.wikipedia.org/wiki/Representational_state_transfer) interfaces to store their data.

The following components are implemented. The user-facing proxy envoy is used to transcode gRPC protobuf (which is prevailing in the microservice world)
to json (which is more friendly to front-end developers) and authorize API accesses. The [`auth`](./services/auth) package is a go program called by envoy
to check the validity of API accesses. We use [hyperium/tonic](https://github.com/hyperium/tonic)
to implement a gRPC server which ideally saves uses data into [data availability committees](https://ethereum.org/en/developers/docs/data-availability/).
But we have only immplemented a data storage which uses MongoDB under the hood.

# Build and deploy
The simplest way to deploy `zkc_state_manager` is to use [Docker Compose](https://docs.docker.com/compose/).

```
docker-compose up
```

# Client API accesses
Both the gRPC and REST API accesses are processed by the same underlying backend server.
The data structure and API methods are defined in the [./proto](./proto) folder.
Refer to [Introduction to gRPC](https://grpc.io/docs/what-is-grpc/introduction/) for a introduction on gRPC and 
[Language Guide (proto 3)](https://protobuf.dev/programming-guides/proto3/) for a comprehensive reference of protobuf file format.

## Merkle tree convention
The height of the Merkle tree we are using is currently hard coded to be 20. Pictorially the indexes of its nodes are laballed as follows.

```
0
1 2
3 4 5 6
7 8 9 10 11 12 13 14
...
...
...
2^20-1 2^20 ... 2^21-2
```
Here the top level index `0` represents the Merkle tree root, and the numbers `1` and `2` below it are the indexes of its left and right children.
Other none-leaf nodes are labelled in the same vein. The numbers in the lowest level are the indexes of the leaves.
There are `2^20` leaves in total. The first leave uses the index `2^20-1 = 1048575`, while the latest leave has index `2^21-2 =2097150`.

## gRPC
We have enabled [gRPC server reflection](https://github.com/grpc/grpc/blob/master/doc/server-reflection.md) to make it more
easier for gRPC clients to introspect which methods and data structures that the servers provides/requries.
As an result, interactively exploring the gRPC with [ktr0731/evans](https://github.com/ktr0731/evans) is quite easier.
We can run `evans -r` to start a `evans` repl shell with reflection enabled. And then type in `desc` and press table to
view all the data structures and services defined in the server.

Users are encouraged to visit [Supported languages | gRPC](https://grpc.io/docs/languages/) for programtically access to gRPC services.

## REST
The same functions are available from RESTful server started by enovy. By default of the [./docker-compose.yml](./docker-compose.yml)
file, the REST server can be accessed at port `50000`. The HTTP routes are defined in the file [./proto/kvpair.proto](./proto/kvpair.proto).
Below are two API access examples with [curl](https://curl.se/).

### Encoding/decoding
#### Bytes
All the message fields with type `bytes` are serialized/deserialized with the base64 encoding scheme.
For example, the 32 bytes array `[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]`
is encoded as the string "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE=".

This can be done with the command
```
printf "$(echo '[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]' | jq '.[]' | xargs -n 1 printf '\\x%s')" | base64
```

#### Enums
`enum`s can be serialized/deserialized with the string liternal of the `enum` branch to use.
For example, when we need to set the `proof_type` field with type `ProofType` and value `ProofEmpty`, we can use 
```
{
  ... // other fields
  "proof_type": "ProofEmpty",
  ... // other fields
}
```

### Get Merkle tree root hash
```bash
curl -v -H token:abc "http://localhost:50000/v1/root"
```
returns
```
{
 "root": "SVNXWlYM9cwac67SR5Unp7sDYcpklUFlOwvvXZZ+IQs="
}
```

#### Note: We don't have the set root hash API

In zkWasm kvpair code there is a kvpair_setroot() API which is actually used to:
1) initialize an empty Merkle tree.
2) Save `contract_id` parameter as `contract_address` which will be used for collection_name
```
      fn get_collection_name(&self) -> String {
        format!("MERKLEDATA_{}", hex::encode(&self.contract_address))
    } 
```

While in our case, 
1) The initialization will be done when the server is started.
2) The `contract_address` parameter can be passed as an optional parameter in GetLeaft/SetLeaf request.
```
message GetLeafRequest {
  optional bytes contract_id = 1;
  uint32 index = 2;
  optional bytes hash = 3;
  ProofType proof_type = 4;
}

message SetLeafRequest {
  optional bytes contract_id = 1;
  uint32 index = 2;
  bytes leaf_data = 3;
  ProofType proof_type = 4;
}
```

### Get nonleaf node children hashes
Given the above Merkle tree root, we can obtain the hashes of its children with
```bash
curl -v -H token:abc "http://localhost:50000/v1/nonleaves?index=0&hash=SVNXWlYM9cwac67SR5Unp7sDYcpklUFlOwvvXZZ+IQs="
```
returns
```
{
 "node": {
  "index": 0,
  "hash": "SVNXWlYM9cwac67SR5Unp7sDYcpklUFlOwvvXZZ+IQs=",
  "node_type": "NodeNonLeaf",
  "children": {
   "left_child_hash": "qQmS05drlx5BhgBhNsSt/FOXBdpZ338JRzXGW+InNBU=",
   "right_child_hash": "qQmS05drlx5BhgBhNsSt/FOXBdpZ338JRzXGW+InNBU="
  }
 }
}
```

### Get leaf node data
```bash
curl -v -H token:abc "http://localhost:50000/v1/leaves?index=1048575"
```
returns
```
{
 "node": {
  "index": 1048575,
  "hash": "iktQjC9pJoboIgTSMKnMHk9sVjo387AHQoNAvHHkIRA=",
  "node_type": "NodeLeaf",
  "data": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
 }
}
```

### Update leaf node data
```bash
curl -v -H token:abc --header "Content-Type: application/json" --header "Accept: application/json" --data '{"index":1048575,"leaf_data":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE=","proof_type":"ProofV0"}' "http://localhost:50000/v1/leaves"
```
returns
```
{
 "node": {
  "index": 1048575,
  "hash": "4Nknab5e81ocyVPqxREoN9xKtLir1yJFOVc9q28WsCY=",
  "node_type": "NodeLeaf",
  "data": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE="
 },
 "proof": {
  "proof_type": "ProofV0",
  "proof": "IAAAAAAAAADg2Sdpvl7zWhzJU+rFESg33Eq0uKvXIkU5Vz2rbxawJiAAAAAAAAAABXpHvVH8xFgAguifSzz71A/ge0dL1aHWjQ2gU2CVwSkUAAAAAAAAACAAAAAAAAAAqQmS05drlx5BhgBhNsSt/FOXBdpZ338JRzXGW+InNBUgAAAAAAAAAAekCqvRl/li176xUOWhdJp948tZ88WBCfagCdiYsVsnIAAAAAAAAAB9jYOuHEhJ3pJBxefz9n7md8sk/I7HJeZP9QMfySVDCCAAAAAAAAAAUve4Q53fIbJgQ0PvZ2AmUE4TTulcG4AqmqohbSUBhx8gAAAAAAAAAPVIJDyje7OUqsNNWi//IdVTfmVBTbQPMDlkNPfaVSsEIAAAAAAAAAALQ7CmOje3TPpJupvzPir5lDmseKw7z7wNJX52Na2VGyAAAAAAAAAA+6EYkEJ7x0LpE0BTkRmuYSB4B9+9NYfUbZIX00sAXSEgAAAAAAAAAJgA3WidyadpQglp1BbRL9DGvY8qZybE0pxMC9alEy8VIAAAAAAAAAAQ2Cb8AHV2Gv7AIHkZ7Ga6hMJQRmZOfzGeZsC3salEACAAAAAAAAAA1aqN0u70OwWnZ2xzFb7TCMjzywtNC+fSmhePcHLFWwEgAAAAAAAAAKm79cBI2GrmMihD80lY2Nw1m609siA69brfB/Su8XAZIAAAAAAAAADtFhJ/nBmi6liu5m4zff9zM/ofd6LUf1rq+9RQSx8TGiAAAAAAAAAAO80QI/CxKSY/ksH9jg04lQjPxKb4z7LX31AnmdVj0AogAAAAAAAAANBtfUK/K/gh1YTtQ+czl3Dl3hpobJoh3OTnDp5GTsQcIAAAAAAAAAAhen8wWoTMm8IU5/TeCkJ/Ag/W1vrZreXcj8TJGqrdBSAAAAAAAAAAnQgSoW+PLL8PhH9I6n1jFc8KZZ1pNFqocBhjvKUAvQ8gAAAAAAAAAM3Wl5tMetUFELaiSthL7SpiJAPhGxaJigNK7ql00qsNIAAAAAAAAABrWeshh6LG40/T7q71F23S8epIjji7FB8oqVw0smYiACAAAAAAAAAA28pLDeVpAt/bAOxpS49M7zPDhb684vYVu7TqDGSykAMgAAAAAAAAAIpLUIwvaSaG6CIE0jCpzB5PbFY6N/OwB0KDQLxx5CEQ//8PAA=="
 }
}
```
## How to calculate index manually
```
let address = self.address.rules[0].u64_value().unwrap() as u32;
let index = (address as u32) + (1u32<<MERKLE_TREE_HEIGHT) - 1;
```
So if a address is 0x19281, then index = 0x19281 + (1u32<<20) - 1 = 1152511.

## How to calculate leaf data manually
Leaf data must be a uint8[32] array, the below command can be used to convert between base64 value and uint[32] array.

e.g 
### Get value
```
curl -v -H token:abc "http://rpc.zkcross.org:50000/v1/leaves?index=1152511"
```
```
 "node": {
  "index": 1152511,
  "hash": "htrWqb9MdIsgU2JDSmW0/zKu1l9OlHip1SlNTGD0txk=",
  "node_type": "NodeLeaf",
  "data": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADCAE="
 }
```

The return data is "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADCAE=", the actual data would be

```
$  base64 -d <<< AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADCAE= | xxd
00000000: 0000 0000 0000 0000 0000 0000 0000 0000  ................
00000010: 0000 0000 0000 0000 0000 0000 0003 0801  ................

[u8; 32] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 03, 08, 01]
```
### Update value

If you want to update the leaf data. e.g 
```
[u8; 32] = [0, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]
```
First, encode the data to base64:
```
$ printf "$(printf 0012000000000000000000000000000000000000000000000000000000000001 | fold -w 2 | xargs -n 1 printf '\\x%s')" | base64
ABIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE=
```

And then set:
```
curl -v -H token:abc --json '{"index":1152511,"leaf_data":"ABIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE=","proof_type":"ProofV0"}' "http://rpc.zkcross.org:50000/v1/leaves"
```
### Get new value

Now you can read back to confirm:
```
curl -v -H token:abc "http://rpc.zkcross.org:50000/v1/leaves?index=1152511"

 "node": {
  "index": 1152511,
  "hash": "zIEbxu4rl6oqoraI5w0yGAatkERPpiX/wWj5p3/M9yw=",
  "node_type": "NodeLeaf",
  "data": "ABIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE="
 }

$ base64 -d <<< ABIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE= | xxd
00000000: 0012 0000 0000 0000 0000 0000 0000 0000  ................
00000010: 0000 0000 0000 0000 0000 0000 0000 0001  ................
```

# Components

## Envoy
Envoy is a service proxy known for its extreme flexibility. Two notable features that we need for envoy are
[gRPC-JSON transcoder](https://www.envoyproxy.io/docs/envoy/latest/configuration/http/http_filters/grpc_json_transcoder_filter) and
[External Authorization](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/security/ext_authz_filter.html)

### gRPC-JSON transcoder
With gRPC-JSON transcoder, we implemented a single backend server that exposes the same functionality to both javascript client and other microservices.
This is quite useful as it is easier for javascript clients to call APIs in the RESTful way and microservices tend to communicate
with each other using gRPC. Envoy can transparently transcode json requests from javascript clients into gRPC requests.

### External Authorization
In order to gate keep API accesses from unauthorized parties, we use the external authorization of envoy to check whether some access is
authenticated. Each access to the backend gRPC server is first forwarded to the auth program. Auth program checks whether the request context
and determine whether to allow this request to hit at the gRPC server. If the request is legal, then `auth` may append additional HTTP headers
to gRPC server (e.g. contract ID used to track which contract is calling this API).

## Auth
The only functionality currently implemented in `auth` is to check the fixed header (`token: abc`) is presented
if it is there the allow this request and then append a fixed HTTP header `x-auth-contract-id: FX6glXnwnPljB/ayPW/WHDz/EjB21Ewn4um+3wITXoc=`
to the downstream request.

In the future, we may lookup token and client information from MongoDB, determine if the request is valid and pass the client information to gRPC server.

## Tonic gRPC server
We implemented part of the service `KvPair` in [./proto/kvpair.proto](./proto/kvpair.proto). Users may use the services provided by this server
with RESTFUL API as noted above or directly issue RPC with gRPC. An example usage is available at [./src/kvpair.rs](./src/kvpair.rs).

### kvpair
This kvpair service implements the Merkle tree trait. Instead of storing Merkle tree data locally, we send the data to the gRPC server and the server
stores the data. Set the environment variable `KVPAIR_GRPC_SERVER_URL`, and then create a `MongoMerkle` with `MongoMerkle::construct` to use this crate.
One thing to note is that we the gRPC server is currently not protected by authentication. We should not expose this service publicly.

## MongoDB
All the nodes in the Merkle tree are stored in the same collection with `MerkleRecord` as their data format.

One thing needs to take special care is that, the current root Merkle record is stored in document with a special
[ObjectId](https://www.mongodb.com/docs/manual/reference/bson-types/#std-label-objectid).

Whenever the client make a API access that mutate current Merkle tree root, we need to update in a the MongoDB transaction.
Otherwise, there may be some data corruption. We may need to implement some component like Sequencer to
serialize all the global data mutations.
