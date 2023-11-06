use zkc_state_manager::kvpair::Hash;
use zkc_state_manager::kvpair::LeafData;
use zkc_state_manager::kvpair::DEFAULT_HASH_VEC;
use zkc_state_manager::kvpair::MERKLE_TREE_HEIGHT;
use zkc_state_manager::proto::kv_pair_client::KvPairClient;
use zkc_state_manager::proto::kv_pair_server::KvPairServer;
use zkc_state_manager::proto::node::NodeData;
use zkc_state_manager::proto::GetLeafRequest;
use zkc_state_manager::proto::GetLeafResponse;
use zkc_state_manager::proto::GetRootRequest;
use zkc_state_manager::proto::GetRootResponse;
use zkc_state_manager::proto::NodeType;
use zkc_state_manager::proto::PoseidonHashRequest;
use zkc_state_manager::proto::PoseidonHashResponse;
use zkc_state_manager::proto::ProofType;
use zkc_state_manager::proto::SetLeafRequest;
use zkc_state_manager::proto::SetLeafResponse;
use zkc_state_manager::service::MongoKvPair;
use zkc_state_manager::service::MongoKvPairTestConfig;

use std::sync::Arc;

use futures::{channel::oneshot, FutureExt};
use rand::{thread_rng, RngCore};
use tempfile::NamedTempFile;
use tokio::net::{UnixListener, UnixStream};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Channel, Endpoint, Server, Uri};
use tonic::Request;
use tower::service_fn;

// Start a gRPC server in the background, returns the JoinHandle to the background task of this
// server, a RPC client for this server and a channel sender which can be used to cancel the
// executation of this gRPC server by sending a message `()` with this sender. This function
// automatically creates a random collection which is automatically dropped at the executation of
// the server task.
async fn start_server_get_client_and_cancellation_handler() -> (
    tokio::task::JoinHandle<()>,
    KvPairClient<Channel>,
    oneshot::Sender<()>,
) {
    let (tx, rx) = oneshot::channel::<()>();
    let socket = NamedTempFile::new().unwrap();
    let socket = Arc::new(socket.into_temp_path());
    std::fs::remove_file(&*socket).unwrap();

    let uds = UnixListener::bind(&*socket).unwrap();
    let stream = UnixListenerStream::new(uds);

    let mut rng = thread_rng();
    let mut contract_id = [0u8; 32];
    rng.fill_bytes(&mut contract_id);
    let test_config = MongoKvPairTestConfig {
        contract_id: contract_id.into(),
    };
    let server = MongoKvPair::new_with_test_config(Some(test_config)).await;
    let kvpair_server = KvPairServer::new(server.clone());

    let join_handler = tokio::spawn(async move {
        let result = Server::builder()
            .add_service(kvpair_server)
            .serve_with_incoming_shutdown(stream, rx.map(drop))
            .await;
        assert!(result.is_ok());
        if std::env::var("KEEP_TEST_COLLECTIONS").is_ok() {
            println!("Keeping test collections");
        } else {
            let result2 = server.drop_test_collection().await;
            assert!(result2.is_ok());
        }
    });

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

    (join_handler, client, tx)
}

async fn get_root(client: &mut KvPairClient<Channel>) -> GetRootResponse {
    let response = client
        .get_root(Request::new(GetRootRequest { contract_id: None }))
        .await
        .unwrap();
    dbg!(&response);

    response.into_inner()
}

async fn get_leaf(
    client: &mut KvPairClient<Channel>,
    index: u64,
    hash: Option<Hash>,
    proof_type: ProofType,
) -> GetLeafResponse {
    let response = client
        .get_leaf(Request::new(GetLeafRequest {
            index,
            hash: hash.map(|h| h.into()),
            proof_type: proof_type.into(),
            contract_id: None,
        }))
        .await
        .unwrap();
    dbg!(&response);

    response.into_inner()
}

async fn set_leaf(
    client: &mut KvPairClient<Channel>,
    index: u64,
    leaf_data: LeafData,
    proof_type: ProofType,
) -> SetLeafResponse {
    let leaf_data: Vec<u8> = leaf_data.0;
    let proof_type = proof_type.into();
    let response = client
        .set_leaf(Request::new(SetLeafRequest {
            index,
            data: Some(leaf_data),
            proof_type,
            contract_id: None,
            hash: None,
        }))
        .await
        .unwrap();
    dbg!(&response);

    response.into_inner()
}

async fn poseidon_hash(client: &mut KvPairClient<Channel>, data: Vec<u8>) -> PoseidonHashResponse {
    let response = client
        .poseidon_hash(Request::new(PoseidonHashRequest {
            contract_id: None,
            data,
        }))
        .await
        .unwrap();
    dbg!(&response);

    response.into_inner()
}

#[tokio::test]
async fn test_get_root() {
    async fn test(client: &mut KvPairClient<Channel>) {
        let response = get_root(client).await;
        assert_eq!(
            Hash::try_from(response.root.as_slice()).unwrap(),
            DEFAULT_HASH_VEC[MERKLE_TREE_HEIGHT]
        );
    }

    let (join_handler, mut client, tx) = start_server_get_client_and_cancellation_handler().await;
    test(&mut client).await;
    tx.send(()).unwrap();
    join_handler.await.unwrap()
}

#[tokio::test]
async fn test_get_leaf() {
    async fn test(client: &mut KvPairClient<Channel>) {
        let index = 2_u64.pow(MERKLE_TREE_HEIGHT.try_into().unwrap()) - 1;
        let response = get_leaf(client, index, None, ProofType::ProofV0).await;
        assert!(response.proof.is_some());
        assert!(response.node.is_some());
        let node = response.node.unwrap();
        assert_eq!(node.index, index);
        assert_eq!(node.node_type, NodeType::NodeLeaf as i32);
        match node.node_data {
            Some(NodeData::Data(data)) => {
                assert_eq!(
                    LeafData::try_from(data.as_slice()).unwrap(),
                    LeafData::default()
                )
            }
            _ => panic!("Invalid node data"),
        }
    }

    let (join_handler, mut client, tx) = start_server_get_client_and_cancellation_handler().await;
    test(&mut client).await;
    tx.send(()).unwrap();
    join_handler.await.unwrap()
}

#[tokio::test]
async fn test_set_leaf_hash_that_is_not_a_field_element() {
    async fn test(client: &mut KvPairClient<Channel>) {
        let response = client
            .set_leaf(Request::new(SetLeafRequest {
                index: 2_u64.pow(MERKLE_TREE_HEIGHT.try_into().unwrap()) - 1,
                data: Some([0xff; 32].to_vec()),
                hash: Some([0xff; 32].to_vec()),
                proof_type: ProofType::ProofEmpty.into(),
                contract_id: None,
            }))
            .await;
        dbg!(&response);
        match response {
            Err(_) => {}
            _ => panic!("Should have returned error on invalid hash"),
        }
    }
    let (join_handler, mut client, tx) = start_server_get_client_and_cancellation_handler().await;
    test(&mut client).await;
    tx.send(()).unwrap();
    join_handler.await.unwrap()
}

#[tokio::test]
async fn test_set_and_get_leaf() {
    async fn test(client: &mut KvPairClient<Channel>) {
        let index = 2_u64.pow(MERKLE_TREE_HEIGHT.try_into().unwrap()) - 1;
        let leaf_data: LeafData = [42_u8; 32].into();
        let response = set_leaf(client, index, leaf_data.clone(), ProofType::ProofEmpty).await;
        assert!(response.node.is_some());
        let node = response.node.unwrap();
        assert_eq!(node.index, index);
        assert_eq!(node.node_type, NodeType::NodeLeaf as i32);
        match node.node_data {
            Some(NodeData::Data(data)) => {
                assert_eq!(LeafData::try_from(data.as_slice()).unwrap(), leaf_data)
            }
            _ => panic!("Invalid node data"),
        }

        let response = get_leaf(client, index, None, ProofType::ProofEmpty).await;
        assert!(response.node.is_some());
        assert_eq!(
            response.node.unwrap().node_data,
            Some(NodeData::Data(leaf_data.into()))
        );
    }

    let (join_handler, mut client, tx) = start_server_get_client_and_cancellation_handler().await;
    test(&mut client).await;
    tx.send(()).unwrap();
    join_handler.await.unwrap()
}

#[tokio::test]
async fn test_poseidon_hash() {
    async fn test(client: &mut KvPairClient<Channel>) {
        let response = poseidon_hash(client, [1; 32].to_vec()).await;
        dbg!(Hash::try_from(response.hash.as_slice()).unwrap());
    }

    let (join_handler, mut client, tx) = start_server_get_client_and_cancellation_handler().await;
    test(&mut client).await;
    tx.send(()).unwrap();
    join_handler.await.unwrap()
}
