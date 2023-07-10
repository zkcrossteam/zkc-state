use tonic::transport::Server;
use tonic_web::GrpcWebLayer;

use proto::kv_pair_server::{KvPair, KvPairServer};

pub mod proto {
    pub(crate) const FILE_DESCRIPTOR_SET: &[u8] =
        tonic::include_file_descriptor_set!("kvpair_descriptor");
    tonic::include_proto!("kvpair");
}

mod kvpair;
mod merkle;
mod poseidon;

#[derive(Default)]
pub struct MongoKvPair {}

#[tonic::async_trait]
impl KvPair for MongoKvPair {
    async fn get_root(
        &self,
        request: tonic::Request<proto::GetRootRequest>,
    ) -> std::result::Result<tonic::Response<proto::GetRootResponse>, tonic::Status> {
        dbg!(request);
        unimplemented!()
    }

    async fn set_root(
        &self,
        request: tonic::Request<proto::SetRootRequest>,
    ) -> std::result::Result<tonic::Response<proto::SetRootResponse>, tonic::Status> {
        dbg!(request);
        unimplemented!()
    }

    async fn get_leaf(
        &self,
        request: tonic::Request<proto::GetLeafRequest>,
    ) -> std::result::Result<tonic::Response<proto::GetLeafResponse>, tonic::Status> {
        dbg!(request);
        unimplemented!()
    }

    async fn set_leaf(
        &self,
        request: tonic::Request<proto::SetLeafRequest>,
    ) -> std::result::Result<tonic::Response<proto::GetLeafResponse>, tonic::Status> {
        dbg!(request);
        unimplemented!()
    }

    async fn get_non_leaf(
        &self,
        request: tonic::Request<proto::GetNonLeafRequest>,
    ) -> std::result::Result<tonic::Response<proto::GetNonLeafResponse>, tonic::Status> {
        dbg!(request);
        unimplemented!()
    }

    async fn set_non_leaf(
        &self,
        request: tonic::Request<proto::SetNonLeafRequest>,
    ) -> std::result::Result<tonic::Response<proto::SetNonLeafResponse>, tonic::Status> {
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

    let server = MongoKvPair::default();
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
