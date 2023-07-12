use zkc_state_manager::kvpair::ContractId;
use zkc_state_manager::kvpair::Hash;
use zkc_state_manager::kvpair::LeafData;
use zkc_state_manager::kvpair::MerkleRecord;
use zkc_state_manager::kvpair::DEFAULT_HASH_VEC;
use zkc_state_manager::kvpair::MERKLE_TREE_HEIGHT;
use zkc_state_manager::proto::kv_pair_client::KvPairClient;
use zkc_state_manager::proto::kv_pair_server::KvPairServer;
use zkc_state_manager::proto::node::NodeData;
use zkc_state_manager::proto::GetLeafRequest;
use zkc_state_manager::proto::GetRootRequest;
use zkc_state_manager::proto::NodeType;
use zkc_state_manager::proto::ProofType;
use zkc_state_manager::proto::SetLeafRequest;
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
        dbg!(&response);
        let response = response.into_inner();
        assert_eq!(
            Hash::try_from(response.root.as_slice()).unwrap(),
            DEFAULT_HASH_VEC[MERKLE_TREE_HEIGHT]
        );
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
async fn get_leaf() {
    async fn test(client: &mut KvPairClient<Channel>) {
        let index = 2_u32.pow(MERKLE_TREE_HEIGHT as u32) - 1;
        let response = client
            .get_leaf(Request::new(GetLeafRequest {
                index: 2_u32.pow(MERKLE_TREE_HEIGHT as u32) - 1,
                hash: None,
                proof_type: ProofType::ProofV0.into(),
            }))
            .await
            .unwrap();
        dbg!(&response);
        let response = response.into_inner();
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

    let (serve_future, mut client) = get_server_and_client_stub().await;
    let request_future = test(&mut client);
    // Wait for completion, when the client request future completes
    tokio::select! {
        _ = serve_future => panic!("server returned first"),
        _ = request_future => (),
    }
}

#[tokio::test]
async fn set_and_get_leaf() {
    async fn test(client: &mut KvPairClient<Channel>) {
        let index = 2_u32.pow(MERKLE_TREE_HEIGHT as u32) - 1;
        let leaf_data: Vec<u8> = [42_u8; 32].into();
        let response = client
            .set_leaf(Request::new(SetLeafRequest {
                index: 2_u32.pow(MERKLE_TREE_HEIGHT as u32) - 1,
                leaf_data: leaf_data.clone(),
                proof_type: ProofType::ProofEmpty.into(),
            }))
            .await
            .unwrap();
        dbg!(&response);
        let response = response.into_inner();
        assert!(response.node.is_some());
        let node = response.node.unwrap();
        assert_eq!(node.index, index);
        assert_eq!(node.node_type, NodeType::NodeLeaf as i32);
        match node.node_data {
            Some(NodeData::Data(data)) => {
                assert_eq!(
                    LeafData::try_from(data.as_slice()).unwrap(),
                    LeafData::try_from(leaf_data.as_slice()).unwrap()
                )
            }
            _ => panic!("Invalid node data"),
        }

        let response = client
            .get_leaf(Request::new(GetLeafRequest {
                index,
                hash: None,
                proof_type: ProofType::ProofEmpty.into(),
            }))
            .await
            .unwrap();
        dbg!(&response);
        let response = response.into_inner();
        assert!(response.node.is_some());
        assert_eq!(
            response.node.unwrap().node_data,
            Some(NodeData::Data(leaf_data))
        );
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
