use zkc_state_manager::kvpair::ContractId;
use zkc_state_manager::kvpair::MerkleRecord;
use zkc_state_manager::kvpair::DEFAULT_HASH_VEC;
use zkc_state_manager::proto::kv_pair_client::KvPairClient;
use zkc_state_manager::proto::kv_pair_server::KvPairServer;
use zkc_state_manager::proto::GetRootRequest;
use zkc_state_manager::service::MongoKvPair;

use std::future::Future;
use std::sync::Arc;

use tempfile::NamedTempFile;
use tokio::net::{UnixListener, UnixStream};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Channel, Endpoint, Server, Uri};
use tonic::Request;
use tower::service_fn;

async fn get_server_and_client_stub() -> (impl Future<Output = ()>, KvPairClient<Channel>) {
    let socket = NamedTempFile::new().unwrap();
    let socket = Arc::new(socket.into_temp_path());
    std::fs::remove_file(&*socket).unwrap();

    let uds = UnixListener::bind(&*socket).unwrap();
    let stream = UnixListenerStream::new(uds);

    let server = MongoKvPair::new().await;
    let server = KvPairServer::new(server);

    let serve_future = async {
        let result = Server::builder()
            .add_service(server)
            .serve_with_incoming(stream)
            .await;
        // Server must be running fine...
        assert!(result.is_ok());
    };

    let socket = Arc::clone(&socket);
    // Connect to the server over a Unix socket
    // The URL will be ignored.
    let channel = Endpoint::try_from("http://any.url")
        .unwrap()
        .connect_with_connector(service_fn(move |_: Uri| {
            let socket = Arc::clone(&socket);
            async move { UnixStream::connect(&*socket).await }
        }))
        .await
        .unwrap();

    let client = KvPairClient::new(channel);

    (serve_future, client)
}

#[tokio::test]
async fn get_root() {
    async fn test(client: &mut KvPairClient<Channel>) {
        let response = client
            .get_root(Request::new(GetRootRequest {}))
            .await
            .unwrap();
        // Validate server response with assertions
        dbg!(response);
    }

    let (serve_future, mut client) = get_server_and_client_stub().await;

    let request_future = test(&mut client);

    // Wait for completion, when the client request future completes
    tokio::select! {
        _ = serve_future => panic!("server returned first"),
        _ = request_future => (),
    }
}

#[tokio::test]
async fn test_service_e2e() {
    let server = MongoKvPair::new().await;
    let contract_id: ContractId = ContractId::default();
    let mut collection = server
        .new_collection::<MerkleRecord>(&contract_id, false)
        .await
        .unwrap();
    collection.drop().await.unwrap();
    collection
        .update_root_merkle_record(&MerkleRecord::default())
        .await
        .unwrap();
    collection
        .update_root_merkle_record(&MerkleRecord::default())
        .await
        .unwrap();
    dbg!(DEFAULT_HASH_VEC.to_vec());
}
