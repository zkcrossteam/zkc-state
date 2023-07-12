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

```bash
curl -H token:abc "http://localhost:50000/v1/root"
```
returns
```
{
 "root": "SVNXWlYM9cwac67SR5Unp7sDYcpklUFlOwvvXZZ+IQs="
}
```


```bash
curl -H token:abc "http://localhost:50000/v1/leaves?index=1048575"
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
We implemented part of the service `KvPair` in [./proto/kvpair.proto](./proto/kvpair.proto).

## MongoDB
All the nodes in the Merkle tree are stored in the same collection with `MerkleRecord` as their data format.

One thing needs to take special care is that, the current root Merkle record is stored in document with a special
[ObjectId](https://www.mongodb.com/docs/manual/reference/bson-types/#std-label-objectid).

Whenever the client make a API access that mutate current Merkle tree root, we need to update in a the MongoDB transaction.
Otherwise, there may be some data corruption. We may need to implement some component like Sequencer to
serialize all the global data mutations.
