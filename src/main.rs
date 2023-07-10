use mongodb::Client;
use tonic::transport::Server;
use tonic_web::GrpcWebLayer;

use zkc_state_manager::proto::{kv_pair_server::KvPairServer, FILE_DESCRIPTOR_SET};
use zkc_state_manager::service::MongoKvPair;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "0.0.0.0:50051".parse().unwrap();

    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .build()
        .unwrap();

    let mongodb_uri: String =
        std::env::var("MONGODB_URI").unwrap_or("mongodb://localhost:27017".to_string());
    let client = Client::with_uri_str(&mongodb_uri).await?;
    let server = MongoKvPair::new(client);
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


