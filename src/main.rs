use futures::{channel::oneshot, FutureExt};
use tokio::signal;
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

    let server = MongoKvPair::new().await;
    let server = KvPairServer::new(server);

    println!("Server listening on {}", addr);
    let (send, recv) = oneshot::channel();
    tokio::spawn(async move {
        match signal::ctrl_c().await {
            Ok(()) => {}
            Err(err) => {
                eprintln!("Unable to listen for shutdown signal: {}", err);
            }
        };
        println!("Shutting down");
        send.send(()).expect("Send shutdown signal");
    });

    Server::builder()
        // GrpcWeb is over http1 so we must enable it.
        .accept_http1(true)
        .layer(GrpcWebLayer::new())
        .add_service(reflection_service)
        .add_service(tonic_web::enable(server))
        .serve_with_shutdown(addr, recv.map(drop))
        .await?;

    Ok(())
}
