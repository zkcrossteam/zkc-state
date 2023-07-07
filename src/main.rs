use tonic::transport::Server;
use tonic_web::GrpcWebLayer;

use proto::data_store_server::{DataStore, DataStoreServer};

pub mod proto {
    pub(crate) const FILE_DESCRIPTOR_SET: &[u8] =
        tonic::include_file_descriptor_set!("datastore_descriptor");
    tonic::include_proto!("datastore");
}

#[derive(Default)]
pub struct MongoDBDataStore {}

#[tonic::async_trait]
impl DataStore for MongoDBDataStore {
    async fn retrieve_data(
        &self,
        request: tonic::Request<proto::RetrievingDataRequest>,
    ) -> std::result::Result<tonic::Response<proto::DataResponse>, tonic::Status> {
        dbg!(request);
        unimplemented!()
    }
    async fn batch_retrieve_data(
        &self,
        request: tonic::Request<proto::RetrievingDataRequest>,
    ) -> std::result::Result<tonic::Response<proto::BatchDataResponse>, tonic::Status> {
        dbg!(request);
        unimplemented!()
    }
    async fn save_data(
        &self,
        request: tonic::Request<proto::SavingDataRequest>,
    ) -> std::result::Result<tonic::Response<proto::DataResponse>, tonic::Status> {
        dbg!(request);
        unimplemented!()
    }
    async fn batch_save_data(
        &self,
        request: tonic::Request<proto::SavingDataRequest>,
    ) -> std::result::Result<tonic::Response<proto::BatchDataResponse>, tonic::Status> {
        dbg!(request);
        unimplemented!()
    }
    async fn get_all_cookies(
        &self,
        request: tonic::Request<proto::GettingCookiesRequest>,
    ) -> std::result::Result<tonic::Response<proto::CookiesResponse>, tonic::Status> {
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

    let greeter = MongoDBDataStore::default();
    let greeter = DataStoreServer::new(greeter);

    println!("Server listening on {}", addr);

    Server::builder()
        // GrpcWeb is over http1 so we must enable it.
        .accept_http1(true)
        .layer(GrpcWebLayer::new())
        .add_service(reflection_service)
        .add_service(tonic_web::enable(greeter))
        .serve(addr)
        .await?;

    Ok(())
}
