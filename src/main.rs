use mongodb::Client;
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use tonic_web::GrpcWebLayer;

use proto::kv_pair_server::{KvPair, KvPairServer};
use proto::*;

pub mod proto {
    pub(crate) const FILE_DESCRIPTOR_SET: &[u8] =
        tonic::include_file_descriptor_set!("kvpair_descriptor");
    tonic::include_proto!("kvpair");
}

#[derive(Copy, Debug, Clone, Eq, PartialEq)]
pub struct ContractId([u8; 32]);

// TODO: Maybe use something like protovalidate to automatically validate fields.
impl TryFrom<&[u8]> for ContractId {
    type Error = Status;

    fn try_from(a: &[u8]) -> Result<ContractId, Self::Error> {
        a.try_into()
            .map_err(|_e| {
                Status::invalid_argument(format!("Contract Id malformed (must be [u8; 32])"))
            })
            .map(|id| ContractId(id))
    }
}

impl From<ContractId> for Vec<u8> {
    fn from(id: ContractId) -> Self {
        id.0.into()
    }
}

mod kvpair;
mod merkle;
mod poseidon;

pub struct MongoKvPair {
    client: Client,
}

#[tonic::async_trait]
impl KvPair for MongoKvPair {
    async fn get_root(
        &self,
        request: Request<GetRootRequest>,
    ) -> std::result::Result<Response<GetRootResponse>, Status> {
        dbg!(&request);
        let request = request.into_inner();
        let contract_id: ContractId = request.contract_id.as_slice().try_into()?;
        Ok(Response::new(GetRootResponse {
            contract_id: contract_id.into(),
            root: [0u8; 32].into(),
        }))
    }

    async fn set_root(
        &self,
        request: Request<SetRootRequest>,
    ) -> std::result::Result<Response<SetRootResponse>, Status> {
        dbg!(request);
        unimplemented!()
    }

    async fn get_leaf(
        &self,
        request: Request<GetLeafRequest>,
    ) -> std::result::Result<Response<GetLeafResponse>, Status> {
        dbg!(request);
        unimplemented!()
    }

    async fn set_leaf(
        &self,
        request: Request<SetLeafRequest>,
    ) -> std::result::Result<Response<GetLeafResponse>, Status> {
        dbg!(request);
        unimplemented!()
    }

    async fn get_non_leaf(
        &self,
        request: Request<GetNonLeafRequest>,
    ) -> std::result::Result<Response<GetNonLeafResponse>, Status> {
        dbg!(request);
        unimplemented!()
    }

    async fn set_non_leaf(
        &self,
        request: Request<SetNonLeafRequest>,
    ) -> std::result::Result<Response<SetNonLeafResponse>, Status> {
        dbg!(request);
        unimplemented!()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "127.0.0.1:50051".parse().unwrap();

    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build()
        .unwrap();

    let mongodb_uri: String =
        std::env::var("MONGODB_URI").unwrap_or("mongodb://localhost:27017".to_string());
    let client = Client::with_uri_str(&mongodb_uri).await?;
    let server = MongoKvPair { client };
    let server = KvPairServer::new(server);

    println!("Server listening on {}", addr);

    Server::builder()
        // GrpcWeb is over http1 so we must enable it.
        .accept_http1(true)
        .layer(GrpcWebLayer::new())
        .add_service(reflection_service)
        .add_service(tonic_web::enable(server))
        .serve(addr)
        .await?;

    Ok(())
}
