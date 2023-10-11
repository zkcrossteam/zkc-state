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
use zkc_state_manager::proto::GetLeafResponse;
use zkc_state_manager::proto::GetRootRequest;
use zkc_state_manager::proto::GetRootResponse;
use zkc_state_manager::proto::NodeType;
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
        let result2 = server.drop_test_collection().await;
        assert!(result.is_ok());
        assert!(result2.is_ok());
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
    let leaf_data: Vec<u8> = leaf_data.0.into();
    let proof_type = proof_type.into();
    let response = client
        .set_leaf(Request::new(SetLeafRequest {
            index,
            leaf_data,
            proof_type,
            contract_id: None,
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
async fn test_set_and_get_leaf() {
    async fn test(client: &mut KvPairClient<Channel>) {
        let index = 2_u64.pow(MERKLE_TREE_HEIGHT.try_into().unwrap()) - 1;
        let leaf_data: LeafData = [42_u8; 32].into();
        let response = set_leaf(client, index, leaf_data, ProofType::ProofEmpty).await;
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

#[tokio::test]
async fn test_merkle_parent_node() {
    async fn test(client: &mut KvPairClient<Channel>) {
        /* Test for check parent node
         * 1. Clear m tree collection. Create default empty m tree. Check root.
         * 2. Update index=2_u64.pow(20) - 1 (first leaf) leave value.
         * 3. Update index=2_u64.pow(20) (second leaf) leave value.
         * 4. Get index=2_u64.pow(19) - 1 node with hash and confirm the left and right are previous set leaves.
         * 5. Load mt from DB and Get index=2_u64.pow(19) - 1 node with hash and confirm the left and right are previous set leaves.
         */
        // Init checking results
        const DEFAULT_ROOT_HASH: [u8; 32] = [
            73, 83, 87, 90, 86, 12, 245, 204, 26, 115, 174, 210, 71, 149, 39, 167, 187, 3, 97, 202,
            100, 149, 65, 101, 59, 11, 239, 93, 150, 126, 33, 11,
        ];

        const INDEX1: u64 = 2_u64.pow(20) - 1;
        const LEAF1_DATA: [u8; 32] = [
            0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ];
        const ROOT_HASH_AFTER_LEAF1: [u8; 32] = [
            220, 212, 154, 109, 18, 67, 151, 222, 104, 230, 29, 103, 72, 127, 226, 98, 46, 127,
            161, 130, 32, 163, 238, 58, 18, 59, 206, 101, 225, 141, 44, 15,
        ];

        const INDEX2: u64 = 2_u64.pow(20);
        const LEAF2_DATA: [u8; 32] = [
            0, 0, 17, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ];

        const ROOT_HASH_AFTER_LEAF2: [u8; 32] = [
            175, 143, 236, 107, 248, 137, 32, 236, 42, 18, 173, 218, 205, 20, 180, 200, 201, 160,
            246, 213, 197, 176, 39, 245, 64, 103, 6, 30, 133, 153, 10, 38,
        ];

        const PARENT_INDEX: u64 = 2_u64.pow(19) - 1;

        // 1
        assert_eq!(get_root(client).await.root.as_slice(), DEFAULT_ROOT_HASH);

        // 2
        let leaf1_response =
            set_leaf(client, INDEX1, LEAF1_DATA.into(), ProofType::ProofEmpty).await;
        let leaf1_hash = Hash::try_from(leaf1_response.node.unwrap().hash.as_slice()).unwrap();
        assert_eq!(
            get_root(client).await.root.as_slice(),
            ROOT_HASH_AFTER_LEAF1
        );

        // 3
        let leaf2_response =
            set_leaf(client, INDEX2, LEAF2_DATA.into(), ProofType::ProofEmpty).await;
        let leaf2_hash = Hash::try_from(leaf2_response.node.unwrap().hash.as_slice()).unwrap();
        assert_eq!(
            get_root(client).await.root.as_slice(),
            ROOT_HASH_AFTER_LEAF2
        );

        // 4
        let parent_hash = Hash::hash_children(&leaf1_hash, &leaf2_hash);
        let response = get_leaf(
            client,
            PARENT_INDEX,
            Some(parent_hash),
            ProofType::ProofEmpty,
        )
        .await;
        assert!(response.node.is_some());
        let node = response.node.unwrap();
        assert_eq!(node.index, PARENT_INDEX);
        assert_eq!(node.node_type, NodeType::NodeNonLeaf as i32);
        match node.node_data {
            Some(NodeData::Children(children)) => {
                assert_eq!(children.left_child_hash, leaf1_hash.0);
                assert_eq!(children.right_child_hash, leaf2_hash.0);
            }
            _ => panic!("Invalid node data"),
        }

        // 5
        let leaf1 = get_leaf(client, INDEX1, None, ProofType::ProofEmpty).await;
        assert!(leaf1.node.is_some());
        let node1 = leaf1.node.unwrap();
        assert_eq!(node1.index, INDEX1);
        assert_eq!(node1.node_type, NodeType::NodeLeaf as i32);
        match node1.node_data {
            Some(NodeData::Data(data)) => {
                assert_eq!(data.as_slice(), LEAF1_DATA)
            }
            _ => panic!("Invalid node data"),
        }

        let leaf2 = get_leaf(client, INDEX2, None, ProofType::ProofEmpty).await;
        assert!(leaf2.node.is_some());
        let node2 = leaf2.node.unwrap();
        assert_eq!(node2.index, INDEX2);
        assert_eq!(node2.node_type, NodeType::NodeLeaf as i32);
        match node2.node_data {
            Some(NodeData::Data(data)) => {
                assert_eq!(data.as_slice(), LEAF2_DATA)
            }
            _ => panic!("Invalid node data"),
        }
    }

    let (join_handler, mut client, tx) = start_server_get_client_and_cancellation_handler().await;
    test(&mut client).await;
    tx.send(()).unwrap();
    join_handler.await.unwrap()
}
