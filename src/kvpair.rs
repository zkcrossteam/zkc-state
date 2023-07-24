use crate::merkle::get_node_type;
use crate::proto::kv_pair_client::KvPairClient;

use crate::proto::node::NodeData;
use crate::proto::{
    GetLeafRequest, GetLeafResponse, GetNonLeafRequest, GetNonLeafResponse, GetRootRequest,
    GetRootResponse, Node, NodeChildren, NodeType, ProofType, SetLeafRequest, SetLeafResponse,
    SetNonLeafRequest, SetNonLeafResponse, SetRootRequest, SetRootResponse,
};

use crate::Error;

use super::merkle::{MerkleError, MerkleErrorCode, MerkleNode, MerkleTree};
use super::poseidon::gen_hasher;
use ff::PrimeField;
use futures::executor;
use halo2_proofs::pairing::bn256::Fr;

use mongodb::bson::doc;
use mongodb::bson::{spec::BinarySubtype, Bson};
use serde::{
    de::{Error as SerdeError, Unexpected},
    Deserialize, Deserializer, Serialize, Serializer,
};

use tonic::transport::Channel;
use tonic::{Request, Status};

pub const MERKLE_TREE_HEIGHT: usize = 20;

// In default_hash vec, it is from leaf to root.
// For example, height of merkle tree is 20.
// DEFAULT_HASH_VEC[0] leaf's default hash. DEFAULT_HASH_VEC[20] is root default hash. It has 21 layers including the leaf layer and root layer.
lazy_static::lazy_static! {
    pub static ref DEFAULT_HASH_VEC: [Hash; MERKLE_TREE_HEIGHT + 1] = {
        let mut leaf_hash = MongoMerkle::empty_leaf(0).hash();
        let mut default_hash = vec![leaf_hash];
        for _ in 0..MERKLE_TREE_HEIGHT {
            leaf_hash = Hash::hash_children(&leaf_hash, &leaf_hash);
            default_hash.push(leaf_hash);
        }
        default_hash.try_into().unwrap()
    };
}

#[derive(Copy, Debug, Clone, Eq, PartialEq, Default, Serialize, Deserialize)]
pub struct ContractId(
    #[serde(serialize_with = "self::serialize_bytes_as_binary")]
    #[serde(deserialize_with = "self::deserialize_u256_as_binary")]
    pub [u8; 32],
);

// TODO: Maybe use something like protovalidate to automatically validate fields.
impl TryFrom<&[u8]> for ContractId {
    type Error = Error;

    fn try_from(a: &[u8]) -> Result<ContractId, Self::Error> {
        a.try_into()
            .map_err(|_e| {
                Error::InvalidArgument("Contract Id malformed (must be [u8; 32])".to_string())
            })
            .map(ContractId)
    }
}

// TODO: Maybe use something like protovalidate to automatically validate fields.
impl TryFrom<&str> for ContractId {
    type Error = Error;

    fn try_from(a: &str) -> Result<ContractId, Self::Error> {
        use base64::{engine::general_purpose, Engine as _};
        general_purpose::STANDARD
            .decode(a)
            .map_err(|e| Error::InvalidArgument(format!("Base64 decoding failed: {e}")))
            .and_then(|v| Self::try_from(v.as_slice()))
    }
}

impl From<ContractId> for Vec<u8> {
    fn from(id: ContractId) -> Self {
        id.0.into()
    }
}

impl From<[u8; 32]> for ContractId {
    fn from(id: [u8; 32]) -> Self {
        Self(id)
    }
}

#[derive(Copy, Debug, Clone, Eq, PartialEq, Default, Serialize, Deserialize)]
pub struct Hash(
    #[serde(serialize_with = "self::serialize_bytes_as_binary")]
    #[serde(deserialize_with = "self::deserialize_u256_as_binary")]
    pub [u8; 32],
);

// TODO: Maybe use something like protovalidate to automatically validate fields.
impl TryFrom<&[u8]> for Hash {
    type Error = Error;

    fn try_from(a: &[u8]) -> Result<Hash, Self::Error> {
        a.try_into()
            .map_err(|_e| Error::InvalidArgument("Hash malformed (must be [u8; 32])".to_string()))
            .map(Hash)
    }
}

impl From<Hash> for Bson {
    fn from(hash: Hash) -> Self {
        hash_to_bson(&hash)
    }
}

impl From<Hash> for Vec<u8> {
    fn from(hash: Hash) -> Self {
        hash.0.into()
    }
}

impl From<[u8; 32]> for Hash {
    fn from(hash: [u8; 32]) -> Self {
        Self(hash)
    }
}

impl Hash {
    pub fn hash_children(left: &Self, right: &Self) -> Self {
        let a = left.0;
        let b = right.0;
        let mut hasher = gen_hasher();
        let a = Fr::from_repr(a).unwrap();
        let b = Fr::from_repr(b).unwrap();
        hasher.update(&[a, b]);
        hasher.squeeze().to_repr().into()
    }

    pub fn hash_data(data: &LeafData) -> Self {
        let data: [u8; 32] = data.0;
        let batchdata = data
            .chunks(16)
            .map(|x| {
                let mut v = x.clone().to_vec();
                v.extend_from_slice(&[0u8; 16]);
                let f = v.try_into().unwrap();
                Fr::from_repr(f).unwrap()
            })
            .collect::<Vec<Fr>>();
        let values: [Fr; 2] = batchdata.try_into().unwrap();
        let mut hasher = gen_hasher();
        hasher.update(&values);
        hasher.squeeze().to_repr().into()
    }

    /// depth start from 0 up to Self::height(). Example 20 height MongoMerkle, root depth=0, leaf depth=20
    pub fn get_default_hash(depth: usize) -> Result<Hash, MerkleError> {
        if depth <= MERKLE_TREE_HEIGHT {
            Ok(DEFAULT_HASH_VEC[MERKLE_TREE_HEIGHT - depth])
        } else {
            Err(MerkleError::new(
                [0; 32].into(),
                depth as u32,
                MerkleErrorCode::InvalidDepth,
            ))
        }
    }

    pub fn validate_children(hash: &Self, left: &Self, right: &Self) -> Result<(), Error> {
        let new_hash = Hash::hash_children(left, right);
        if *hash != new_hash {
            return Err(Error::InvalidArgument(format!(
                "Hash not matching: {:?} and {:?} hashed to {:?}, not {:?}",
                &left, &right, &new_hash, &hash
            )));
        }
        Ok(())
    }
    pub fn validate_data(hash: &Hash, data: &LeafData) -> Result<(), Error> {
        let new_hash = Self::hash_data(data);
        if *hash != new_hash {
            return Err(Error::InvalidArgument(format!(
                "Hash not matching: {:?} hashed to {:?}, not {:?}",
                &data, &new_hash, &hash
            )));
        }
        Ok(())
    }
}

#[derive(Copy, Debug, Clone, Eq, PartialEq, Default, Serialize, Deserialize)]
pub struct LeafData(
    #[serde(serialize_with = "self::serialize_bytes_as_binary")]
    #[serde(deserialize_with = "self::deserialize_u256_as_binary")]
    pub [u8; 32],
);

// TODO: Maybe use something like protovalidate to automatically validate fields.
impl TryFrom<&[u8]> for LeafData {
    type Error = Error;

    fn try_from(a: &[u8]) -> Result<LeafData, Self::Error> {
        a.try_into()
            .map_err(|_e| {
                Error::InvalidArgument("LeafData malformed (must be [u8; 32])".to_string())
            })
            .map(LeafData)
    }
}

impl From<LeafData> for Vec<u8> {
    fn from(value: LeafData) -> Self {
        value.0.into()
    }
}

impl From<[u8; 32]> for LeafData {
    fn from(value: [u8; 32]) -> Self {
        Self(value)
    }
}

fn deserialize_u256_as_binary<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
where
    D: Deserializer<'de>,
{
    match Bson::deserialize(deserializer) {
        Ok(Bson::Binary(bytes)) => Ok(bytes.bytes.try_into().unwrap()),
        Ok(..) => Err(SerdeError::invalid_value(Unexpected::Enum, &"Bson::Binary")),
        Err(e) => Err(e),
    }
}

pub fn serialize_bytes_as_binary<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let binary = Bson::Binary(mongodb::bson::Binary {
        subtype: BinarySubtype::Generic,
        bytes: bytes.into(),
    });
    binary.serialize(serializer)
}

pub fn hash_to_bson(x: &Hash) -> Bson {
    Bson::Binary(mongodb::bson::Binary {
        subtype: BinarySubtype::Generic,
        bytes: (*x).into(),
    })
}

#[derive(Debug)]
pub struct MongoMerkle {
    root_hash: Hash,
    contract_id: ContractId,
    client: KvPairClient<Channel>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, Default, Eq, PartialEq)]
pub struct MerkleRecord {
    pub index: u32,
    pub hash: Hash,
    pub left: Hash,
    pub right: Hash,
    pub data: LeafData,
}

impl TryFrom<Node> for MerkleRecord {
    type Error = Error;

    fn try_from(n: Node) -> Result<Self, Self::Error> {
        if n.node_type == NodeType::NodeLeaf as i32 {
            match n.node_data {
                Some(NodeData::Data(data)) => {
                    let data: LeafData = data.as_slice().try_into()?;
                    let record = MerkleRecord::new_leaf(n.index, data);
                    assert_eq!(record.hash.0.to_vec(), n.hash);
                    Ok(record)
                }
                _ => {
                    dbg!(&n);
                    panic!("Invalid node data");
                }
            }
        } else if n.node_type == NodeType::NodeNonLeaf as i32 {
            match n.node_data {
                Some(NodeData::Children(children)) => {
                    let left: Hash = children.left_child_hash.as_slice().try_into()?;
                    let right: Hash = children.right_child_hash.as_slice().try_into()?;
                    let record = MerkleRecord::new_non_leaf(n.index, left, right);
                    assert_eq!(record.hash.0.to_vec(), n.hash);
                    Ok(record)
                }
                _ => {
                    dbg!(&n);
                    panic!("Invalid node data");
                }
            }
        } else {
            Err(Error::InvalidArgument("Invalid node type".to_string()))
        }
    }
}

impl TryFrom<MerkleRecord> for Node {
    type Error = Error;

    fn try_from(record: MerkleRecord) -> Result<Self, Self::Error> {
        let index = record.index();
        let hash = record.hash().into();
        let node_type = get_node_type(index, MERKLE_TREE_HEIGHT);
        if node_type != NodeType::NodeLeaf && node_type != NodeType::NodeNonLeaf {
            return Err(Error::InconsistentData(
                "Invalid node (must be leaf or nonleaf node)".to_string(),
            ));
        }
        let node_data = if node_type == NodeType::NodeLeaf {
            NodeData::Data(record.data.0.to_vec())
        } else {
            let left_child_hash = record
                .left()
                .ok_or(Error::InconsistentData(
                    "Nonleaf node has no children".to_string(),
                ))?
                .into();
            let right_child_hash = record
                .right()
                .ok_or(Error::InconsistentData(
                    "Nonleaf node has no children".to_string(),
                ))?
                .into();
            NodeData::Children(NodeChildren {
                left_child_hash,
                right_child_hash,
            })
        };
        Ok(Node {
            index,
            hash,
            node_type: node_type.into(),
            node_data: Some(node_data),
        })
    }
}

impl MerkleNode<Hash> for MerkleRecord {
    fn index(&self) -> u32 {
        self.index
    }
    fn hash(&self) -> Hash {
        self.hash
    }
    fn set(&mut self, data: &[u8]) {
        let data: [u8; 32] = data.clone().try_into().unwrap();
        self.data = data.into();
        self.hash = Hash::hash_data(&self.data);
    }
    fn right(&self) -> Option<Hash> {
        Some(self.right)
    }
    fn left(&self) -> Option<Hash> {
        Some(self.left)
    }
}

impl MerkleRecord {
    pub fn new(index: u32) -> Self {
        MerkleRecord {
            index,
            hash: [0; 32].into(),
            data: [0; 32].into(),
            left: [0; 32].into(),
            right: [0; 32].into(),
        }
    }

    pub fn new_leaf(index: u32, data: impl Into<LeafData>) -> Self {
        let mut record = MerkleRecord::new(index);
        record.data = data.into();
        record.hash = Hash::hash_data(&record.data);
        record
    }

    pub fn new_non_leaf(index: u32, left: impl Into<Hash>, right: impl Into<Hash>) -> Self {
        let mut record = MerkleRecord::new(index);
        record.left = left.into();
        record.right = right.into();
        record.hash = Hash::hash_children(&record.left, &record.right);
        record
    }

    pub fn new_root(left: impl Into<Hash>, right: impl Into<Hash>) -> Self {
        let mut record = MerkleRecord::new(0);
        record.left = left.into();
        record.right = right.into();
        record.hash = Hash::hash_children(&record.left, &record.right);
        record
    }

    pub fn data_as_u64(&self) -> [u64; 4] {
        [
            u64::from_le_bytes(self.data.0[0..8].try_into().unwrap()),
            u64::from_le_bytes(self.data.0[8..16].try_into().unwrap()),
            u64::from_le_bytes(self.data.0[16..24].try_into().unwrap()),
            u64::from_le_bytes(self.data.0[24..32].try_into().unwrap()),
        ]
    }

    pub fn get_default_record(index: u32) -> Result<Self, MerkleError> {
        let height = (index + 1).ilog2() as usize;
        let default = Hash::get_default_hash(height)?;
        let child_hash = if height == MERKLE_TREE_HEIGHT {
            [0; 32].into()
        } else {
            Hash::get_default_hash(height + 1)?
        };
        Ok(MerkleRecord {
            index,
            hash: default,
            data: [0; 32].into(),
            left: child_hash,
            right: child_hash,
        })
    }
}

impl MongoMerkle {
    pub async fn get_client() -> KvPairClient<Channel> {
        KvPairClient::connect("http://localhost:50051")
            .await
            .expect("Connect gRPC server")
    }

    pub fn height() -> usize {
        MERKLE_TREE_HEIGHT
    }
    fn empty_leaf(index: u32) -> MerkleRecord {
        let mut leaf = MerkleRecord::new(index);
        leaf.set([0; 32].as_ref());
        leaf
    }

    pub async fn get_root(&mut self) -> Result<GetRootResponse, Status> {
        let response = self
            .client
            .get_root(Request::new(GetRootRequest {
                contract_id: Some(self.contract_id.into()),
            }))
            .await?;
        dbg!(&response);

        Ok(response.into_inner())
    }

    pub async fn set_root(
        &mut self,
        hash: Option<Hash>,
        left: Hash,
        right: Hash,
    ) -> Result<SetRootResponse, Status> {
        let response = self
            .client
            .set_root(Request::new(SetRootRequest {
                hash: hash.map(|x| x.into()),
                left_child_hash: left.into(),
                right_child_hash: right.into(),
                contract_id: Some(self.contract_id.into()),
            }))
            .await?;
        dbg!(&response);

        Ok(response.into_inner())
    }

    pub async fn get_leaf(
        &mut self,
        index: u32,
        hash: Option<Hash>,
        proof_type: ProofType,
    ) -> Result<GetLeafResponse, Status> {
        let response = self
            .client
            .get_leaf(Request::new(GetLeafRequest {
                index,
                hash: hash.map(|h| h.into()),
                proof_type: proof_type.into(),
                contract_id: Some(self.contract_id.into()),
            }))
            .await?;
        dbg!(&response);

        Ok(response.into_inner())
    }

    pub async fn set_leaf(
        &mut self,
        index: u32,
        leaf_data: LeafData,
        proof_type: ProofType,
    ) -> Result<SetLeafResponse, Status> {
        let leaf_data: Vec<u8> = leaf_data.0.into();
        let proof_type = proof_type.into();
        let response = self
            .client
            .set_leaf(Request::new(SetLeafRequest {
                index,
                leaf_data,
                proof_type,
                contract_id: Some(self.contract_id.into()),
            }))
            .await?;
        dbg!(&response);

        Ok(response.into_inner())
    }

    pub async fn get_non_leaf(
        &mut self,
        index: u32,
        hash: Hash,
    ) -> Result<GetNonLeafResponse, Status> {
        let response = self
            .client
            .get_non_leaf(Request::new(GetNonLeafRequest {
                index,
                hash: hash.into(),
                contract_id: Some(self.contract_id.into()),
            }))
            .await?;
        dbg!(&response);

        Ok(response.into_inner())
    }

    pub async fn set_non_leaf(
        &mut self,
        index: u32,
        hash: Option<Hash>,
        left: Hash,
        right: Hash,
    ) -> Result<SetNonLeafResponse, Status> {
        let response = self
            .client
            .set_non_leaf(Request::new(SetNonLeafRequest {
                index,
                hash: hash.map(|x| x.into()),
                left_child_hash: left.into(),
                right_child_hash: right.into(),
                contract_id: Some(self.contract_id.into()),
            }))
            .await?;
        dbg!(&response);

        Ok(response.into_inner())
    }
}

impl MerkleTree<Hash, 20> for MongoMerkle {
    type Id = ContractId;
    type Root = Hash;
    type Node = MerkleRecord;

    fn construct(addr: Self::Id, root: Self::Root) -> Self {
        let client = executor::block_on(Self::get_client());

        MongoMerkle {
            root_hash: root,
            client,
            contract_id: addr,
        }
    }

    fn get_root_hash(&self) -> Hash {
        self.root_hash
    }

    fn update_root_hash(&mut self, hash: &Hash) {
        self.root_hash = *hash;
    }

    fn hash(a: &Hash, b: &Hash) -> Hash {
        Hash::hash_children(a, b)
    }

    fn set_parent(
        &mut self,
        index: u32,
        hash: &Hash,
        left: &Hash,
        right: &Hash,
    ) -> Result<(), MerkleError> {
        self.boundary_check(index)?;
        println!("set_node_with_hash {} {:?}", index, hash);
        executor::block_on(self.set_non_leaf(index, Some(*hash), *left, *right)).map_err(|e| {
            dbg!(e);
            MerkleError::new(*hash, index, MerkleErrorCode::InvalidDepth)
        })?;
        Ok(())
    }

    fn get_node_with_hash(&mut self, index: u32, hash: &Hash) -> Result<Self::Node, MerkleError> {
        let node_type = get_node_type(index, MERKLE_TREE_HEIGHT);
        let node = if node_type == NodeType::NodeLeaf {
            executor::block_on(self.get_leaf(index, Some(*hash), ProofType::ProofEmpty))
                .map(|x| x.node.unwrap())
        } else {
            executor::block_on(self.get_non_leaf(index, *hash)).map(|x| x.node.unwrap())
        }
        .and_then(|x| Ok(MerkleRecord::try_from(x)?))
        .map_err(|e| {
            dbg!(e);
            MerkleError::new(*hash, index, MerkleErrorCode::InvalidOther)
        })?;
        Ok(node)
    }

    fn set_leaf(&mut self, leaf: &MerkleRecord) -> Result<(), MerkleError> {
        self.boundary_check(leaf.index())?; //should be leaf check?
        executor::block_on(self.set_leaf(leaf.index, leaf.data, ProofType::ProofEmpty)).map_err(
            |e| {
                dbg!(e);
                MerkleError::new(leaf.hash, leaf.index, MerkleErrorCode::InvalidOther)
            },
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rand::{thread_rng, RngCore};

    use super::{MongoMerkle, DEFAULT_HASH_VEC};
    use crate::{
        kvpair::Hash,
        merkle::{MerkleNode, MerkleTree},
    };

    fn get_test_contract_id() -> [u8; 32] {
        let mut rng = thread_rng();
        let mut contract_id = [0u8; 32];
        rng.fill_bytes(&mut contract_id);
        let prefix = b"test";
        contract_id[0..prefix.len()].copy_from_slice(prefix);
        contract_id
    }

    #[test]
    /* Test for check parent node
     * 1. Clear m tree collection. Create default empty m tree. Check root.
     * 2. Update index=2_u32.pow(20) - 1 (first leaf) leave value.
     * 3. Update index=2_u32.pow(20) (second leaf) leave value.
     * 4. Get index=2_u32.pow(19) - 1 node with hash and confirm the left and right are previous set leaves.
     * 5. Load mt from DB and Get index=2_u32.pow(19) - 1 node with hash and confirm the left and right are previous set leaves.
     */
    fn test_mongo_merkle_parent_node() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _enter = rt.enter();

        // Init checking results
        let test_addr: [u8; 32] = get_test_contract_id();

        const DEFAULT_ROOT_HASH: [u8; 32] = [
            73, 83, 87, 90, 86, 12, 245, 204, 26, 115, 174, 210, 71, 149, 39, 167, 187, 3, 97, 202,
            100, 149, 65, 101, 59, 11, 239, 93, 150, 126, 33, 11,
        ];

        const DEFAULT_ROOT_HASH64: [u64; 4] = [
            14768724118053802825,
            12044759864135545626,
            7296277131441537979,
            802061392934800187,
        ];

        const INDEX1: u32 = 2_u32.pow(20) - 1;
        const LEAF1_DATA: [u8; 32] = [
            0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ];
        const ROOT_HASH_AFTER_LEAF1: [u8; 32] = [
            220, 212, 154, 109, 18, 67, 151, 222, 104, 230, 29, 103, 72, 127, 226, 98, 46, 127,
            161, 130, 32, 163, 238, 58, 18, 59, 206, 101, 225, 141, 44, 15,
        ];
        const ROOT64_HASH_AFTER_LEAF1: [u64; 4] = [
            16039362344330646748,
            7125397509397931624,
            4246510858682859310,
            1093404808759360274,
        ];

        const INDEX2: u32 = 2_u32.pow(20);
        const LEAF2_DATA: [u8; 32] = [
            0, 0, 17, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ];

        const ROOT_HASH_AFTER_LEAF2: [u8; 32] = [
            175, 143, 236, 107, 248, 137, 32, 236, 42, 18, 173, 218, 205, 20, 180, 200, 201, 160,
            246, 213, 197, 176, 39, 245, 64, 103, 6, 30, 133, 153, 10, 38,
        ];
        const ROOT64_HASH_AFTER_LEAF2: [u64; 4] = [
            17014751092261293999,
            14462207177763131946,
            17665282427128815817,
            2741172120221804352,
        ];

        const PARENT_INDEX: u32 = 2_u32.pow(19) - 1;

        // 1
        let mut mt =
            MongoMerkle::construct(test_addr.into(), DEFAULT_HASH_VEC[MongoMerkle::height()]);
        let root = mt.get_root_hash();
        let root64 = root
            .0
            .chunks(8)
            .map(|x| u64::from_le_bytes(x.to_vec().try_into().unwrap()))
            .collect::<Vec<u64>>();
        /* */
        assert_eq!(root.0, DEFAULT_ROOT_HASH);
        assert_eq!(root64, DEFAULT_ROOT_HASH64);

        // 2
        let (mut leaf1, _) = mt.get_leaf_with_proof(INDEX1).unwrap();
        leaf1.set(LEAF1_DATA.as_ref());
        mt.set_leaf_with_proof(&leaf1).unwrap();

        let root = mt.get_root_hash();
        let root64 = root
            .0
            .chunks(8)
            .map(|x| u64::from_le_bytes(x.to_vec().try_into().unwrap()))
            .collect::<Vec<u64>>();
        assert_eq!(root.0, ROOT_HASH_AFTER_LEAF1);
        assert_eq!(root64, ROOT64_HASH_AFTER_LEAF1);

        // 3
        let (mut leaf2, _) = mt.get_leaf_with_proof(INDEX2).unwrap();
        leaf2.set(LEAF2_DATA.as_ref());
        mt.set_leaf_with_proof(&leaf2).unwrap();

        let root = mt.get_root_hash();
        let root64 = root
            .0
            .chunks(8)
            .map(|x| u64::from_le_bytes(x.to_vec().try_into().unwrap()))
            .collect::<Vec<u64>>();
        assert_eq!(root.0, ROOT_HASH_AFTER_LEAF2);
        assert_eq!(root64, ROOT64_HASH_AFTER_LEAF2);

        // 4
        let parent_hash = Hash::hash_children(&leaf1.hash, &leaf2.hash);
        let parent_node = mt.get_node_with_hash(PARENT_INDEX, &parent_hash).unwrap();
        assert_eq!(leaf1.hash, parent_node.left().unwrap());
        assert_eq!(leaf2.hash, parent_node.right().unwrap());

        // 5
        let a: [u8; 32] = ROOT_HASH_AFTER_LEAF2;
        let mut mt_loaded: MongoMerkle = MongoMerkle::construct(test_addr.into(), a.into());
        assert_eq!(mt_loaded.get_root_hash().0, a);
        let (leaf1, _) = mt_loaded.get_leaf_with_proof(INDEX1).unwrap();
        assert_eq!(leaf1.index, INDEX1);
        assert_eq!(leaf1.data.0, LEAF1_DATA);
        let (leaf2, _) = mt_loaded.get_leaf_with_proof(INDEX2).unwrap();
        assert_eq!(leaf2.index, INDEX2);
        assert_eq!(leaf2.data.0, LEAF2_DATA);
        let parent_hash = Hash::hash_children(&leaf1.hash, &leaf2.hash);
        let parent_node = mt_loaded
            .get_node_with_hash(PARENT_INDEX, &parent_hash)
            .unwrap();
        assert_eq!(leaf1.hash, parent_node.left().unwrap());
        assert_eq!(leaf2.hash, parent_node.right().unwrap());
    }

    #[test]
    /* Basic tests for 20 height m tree
     * 1. Clear m tree collection. Create default empty m tree. Check root.
     * 2. Update index=2_u32.pow(20) - 1 (first leaf) leave value. Check root.
     * 3. Check index=2_u32.pow(20) - 1 leave value updated.
     * 4. Load m tree from DB, check root and leave value.
     */
    fn test_mongo_merkle_single_leaf_update() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _enter = rt.enter();

        // Init checking results
        let test_addr: [u8; 32] = get_test_contract_id();
        const DEFAULT_ROOT_HASH: [u8; 32] = [
            73, 83, 87, 90, 86, 12, 245, 204, 26, 115, 174, 210, 71, 149, 39, 167, 187, 3, 97, 202,
            100, 149, 65, 101, 59, 11, 239, 93, 150, 126, 33, 11,
        ];

        const DEFAULT_ROOT_HASH64: [u64; 4] = [
            14768724118053802825,
            12044759864135545626,
            7296277131441537979,
            802061392934800187,
        ];

        const INDEX1: u32 = 2_u32.pow(20) - 1;
        const LEAF1_DATA: [u8; 32] = [
            0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ];
        const ROOT_HASH_AFTER_LEAF1: [u8; 32] = [
            220, 212, 154, 109, 18, 67, 151, 222, 104, 230, 29, 103, 72, 127, 226, 98, 46, 127,
            161, 130, 32, 163, 238, 58, 18, 59, 206, 101, 225, 141, 44, 15,
        ];
        const ROOT64_HASH_AFTER_LEAF1: [u64; 4] = [
            16039362344330646748,
            7125397509397931624,
            4246510858682859310,
            1093404808759360274,
        ];

        // 1
        let mut mt =
            MongoMerkle::construct(test_addr.into(), DEFAULT_HASH_VEC[MongoMerkle::height()]);
        let root = mt.get_root_hash();
        let root64 = root
            .0
            .chunks(8)
            .map(|x| u64::from_le_bytes(x.to_vec().try_into().unwrap()))
            .collect::<Vec<u64>>();
        assert_eq!(root.0, DEFAULT_ROOT_HASH);
        assert_eq!(root64, DEFAULT_ROOT_HASH64);

        // 2
        let (mut leaf, _) = mt.get_leaf_with_proof(INDEX1).unwrap();
        leaf.set(LEAF1_DATA.as_ref());
        mt.set_leaf_with_proof(&leaf).unwrap();

        let root = mt.get_root_hash();
        let root64 = root
            .0
            .chunks(8)
            .map(|x| u64::from_le_bytes(x.to_vec().try_into().unwrap()))
            .collect::<Vec<u64>>();
        assert_eq!(root.0, ROOT_HASH_AFTER_LEAF1);
        assert_eq!(root64, ROOT64_HASH_AFTER_LEAF1);

        // 3
        let (leaf, _) = mt.get_leaf_with_proof(INDEX1).unwrap();
        assert_eq!(leaf.index, INDEX1);
        assert_eq!(leaf.data.0, LEAF1_DATA);

        // 4
        let a = ROOT_HASH_AFTER_LEAF1;
        let mut mt = MongoMerkle::construct(test_addr.into(), a.into());
        assert_eq!(mt.get_root_hash().0, a);
        let (leaf, _) = mt.get_leaf_with_proof(INDEX1).unwrap();
        assert_eq!(leaf.index, INDEX1);
        assert_eq!(leaf.data.0, LEAF1_DATA);
    }

    #[test]
    /* Tests for 20 height m tree with updating multple leaves
     * 1. Clear m tree collection. Create default empty m tree. Check root (default one, A).
     * 2. Update index=2_u32.pow(20) - 1 (first leaf) leave value. Check root (1 leave updated, B). Check index=2_u32.pow(20) - 1 leave value updated.
     * 3. Update index=2_u32.pow(20) (second leaf) leave value. Check root (1 leave updated, C). Check index=2_u32.pow(20) leave value updated.
     * 4. Update index=2_u32.pow(21) - 2 (last leaf) leave value. Check root (1 leave updated, D). Check index=2_u32.pow(21) -2 leave value updated.
     * 5. Load m tree from DB with D root hash, check root and leaves' values.
     */
    fn test_mongo_merkle_multi_leaves_update() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _enter = rt.enter();

        // Init checking results
        let test_addr: [u8; 32] = get_test_contract_id();
        const DEFAULT_ROOT_HASH: [u8; 32] = [
            73, 83, 87, 90, 86, 12, 245, 204, 26, 115, 174, 210, 71, 149, 39, 167, 187, 3, 97, 202,
            100, 149, 65, 101, 59, 11, 239, 93, 150, 126, 33, 11,
        ];

        const DEFAULT_ROOT_HASH64: [u64; 4] = [
            14768724118053802825,
            12044759864135545626,
            7296277131441537979,
            802061392934800187,
        ];

        const INDEX1: u32 = 2_u32.pow(20) - 1;
        const LEAF1_DATA: [u8; 32] = [
            0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ];
        const ROOT_HASH_AFTER_LEAF1: [u8; 32] = [
            220, 212, 154, 109, 18, 67, 151, 222, 104, 230, 29, 103, 72, 127, 226, 98, 46, 127,
            161, 130, 32, 163, 238, 58, 18, 59, 206, 101, 225, 141, 44, 15,
        ];
        const ROOT64_HASH_AFTER_LEAF1: [u64; 4] = [
            16039362344330646748,
            7125397509397931624,
            4246510858682859310,
            1093404808759360274,
        ];

        const INDEX2: u32 = 2_u32.pow(20);
        const LEAF2_DATA: [u8; 32] = [
            0, 0, 17, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ];
        const ROOT_HASH_AFTER_LEAF2: [u8; 32] = [
            175, 143, 236, 107, 248, 137, 32, 236, 42, 18, 173, 218, 205, 20, 180, 200, 201, 160,
            246, 213, 197, 176, 39, 245, 64, 103, 6, 30, 133, 153, 10, 38,
        ];
        const ROOT64_HASH_AFTER_LEAF2: [u64; 4] = [
            17014751092261293999,
            14462207177763131946,
            17665282427128815817,
            2741172120221804352,
        ];

        const INDEX3: u32 = 2_u32.pow(21) - 2;
        const LEAF3_DATA: [u8; 32] = [
            18, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ];
        const ROOT_HASH_AFTER_LEAF3: [u8; 32] = [
            43, 187, 19, 165, 241, 143, 152, 84, 13, 90, 30, 178, 214, 218, 174, 172, 3, 62, 218,
            225, 36, 25, 216, 69, 165, 241, 144, 78, 194, 164, 240, 21,
        ];
        const ROOT64_HASH_AFTER_LEAF3: [u64; 4] = [
            6095780363665390379,
            12443123436117449229,
            5032800229785222659,
            1580944623655776677,
        ];

        // 1
        let mut mt =
            MongoMerkle::construct(test_addr.into(), DEFAULT_HASH_VEC[MongoMerkle::height()]);
        let root = mt.get_root_hash();
        let root64 = root
            .0
            .chunks(8)
            .map(|x| u64::from_le_bytes(x.to_vec().try_into().unwrap()))
            .collect::<Vec<u64>>();

        assert_eq!(root.0, DEFAULT_ROOT_HASH);
        assert_eq!(root64, DEFAULT_ROOT_HASH64);

        // 2
        let (mut leaf, _) = mt.get_leaf_with_proof(INDEX1).unwrap();
        leaf.set(LEAF1_DATA.as_ref());
        mt.set_leaf_with_proof(&leaf).unwrap();

        let root = mt.get_root_hash();
        let root64 = root
            .0
            .chunks(8)
            .map(|x| u64::from_le_bytes(x.to_vec().try_into().unwrap()))
            .collect::<Vec<u64>>();

        assert_eq!(root.0, ROOT_HASH_AFTER_LEAF1);
        assert_eq!(root64, ROOT64_HASH_AFTER_LEAF1);

        let (leaf, _) = mt.get_leaf_with_proof(INDEX1).unwrap();

        assert_eq!(leaf.index, INDEX1);
        assert_eq!(leaf.data.0, LEAF1_DATA);

        // 3
        let (mut leaf, _) = mt.get_leaf_with_proof(INDEX2).unwrap();
        leaf.set(LEAF2_DATA.as_ref());
        mt.set_leaf_with_proof(&leaf).unwrap();

        let root = mt.get_root_hash();
        let root64 = root
            .0
            .chunks(8)
            .map(|x| u64::from_le_bytes(x.to_vec().try_into().unwrap()))
            .collect::<Vec<u64>>();

        assert_eq!(root.0, ROOT_HASH_AFTER_LEAF2);
        assert_eq!(root64, ROOT64_HASH_AFTER_LEAF2);

        let (leaf, _) = mt.get_leaf_with_proof(INDEX2).unwrap();
        assert_eq!(leaf.index, INDEX2);
        assert_eq!(leaf.data.0, LEAF2_DATA);

        // 4
        let (mut leaf, _) = mt.get_leaf_with_proof(INDEX3).unwrap();
        leaf.set(LEAF3_DATA.as_ref());
        mt.set_leaf_with_proof(&leaf).unwrap();

        let root = mt.get_root_hash();
        let root64 = root
            .0
            .chunks(8)
            .map(|x| u64::from_le_bytes(x.to_vec().try_into().unwrap()))
            .collect::<Vec<u64>>();

        assert_eq!(root.0, ROOT_HASH_AFTER_LEAF3);
        assert_eq!(root64, ROOT64_HASH_AFTER_LEAF3);

        let (leaf, _) = mt.get_leaf_with_proof(INDEX3).unwrap();
        assert_eq!(leaf.index, INDEX3);
        assert_eq!(leaf.data.0, LEAF3_DATA);

        // 5
        let mut mt = MongoMerkle::construct(test_addr.into(), ROOT_HASH_AFTER_LEAF3.into());
        assert_eq!(mt.get_root_hash().0, ROOT_HASH_AFTER_LEAF3);
        let (leaf, _) = mt.get_leaf_with_proof(INDEX1).unwrap();
        assert_eq!(leaf.index, INDEX1);
        assert_eq!(leaf.data.0, LEAF1_DATA);
        let (leaf, _) = mt.get_leaf_with_proof(INDEX2).unwrap();
        assert_eq!(leaf.index, INDEX2);
        assert_eq!(leaf.data.0, LEAF2_DATA);
        let (leaf, _) = mt.get_leaf_with_proof(INDEX3).unwrap();
        assert_eq!(leaf.index, INDEX3);
        assert_eq!(leaf.data.0, LEAF3_DATA);
    }

    #[test]
    fn test_generate_kv_input() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _enter = rt.enter();

        let mut mt =
            MongoMerkle::construct([0; 32].into(), DEFAULT_HASH_VEC[MongoMerkle::height()]);
        let (mut leaf, _) = mt.get_leaf_with_proof(2_u32.pow(20) - 1).unwrap();
        leaf.set([1u8; 32].as_ref());
        mt.set_leaf_with_proof(&leaf).unwrap();
        let _root = mt.get_root_hash();

        // get {
        //     current_root: 4*64 --> bn254    // fr 2^256-C
        //     index: 1*64
        //     ret: 4:64     ---> 2 * bn254
        // }
        // set {
        //     current_root: 4*64
        //     index: 1*64
        //     data: 4:64
        // }

        // TODO
    }
}
