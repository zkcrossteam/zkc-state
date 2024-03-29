use crate::merkle::get_node_type;
use crate::poseidon::{gen_merkle_hasher, gen_merkle_leaf_hasher};
use crate::proto::kv_pair_client::KvPairClient;

use crate::proto::node::NodeData;
use crate::proto::{
    GetLeafRequest, GetLeafResponse, GetNonLeafRequest, GetNonLeafResponse, GetRootRequest,
    GetRootResponse, Node, NodeChildren, NodeType, ProofType, SetLeafRequest, SetLeafResponse,
    SetNonLeafRequest, SetNonLeafResponse, SetRootRequest, SetRootResponse,
};

use crate::Error;

use super::merkle::{MerkleError, MerkleErrorCode, MerkleNode, MerkleTree};
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

pub const MERKLE_TREE_HEIGHT: usize = 32;

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
    #[serde(deserialize_with = "self::deserialize_u256_from_binary")]
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

/// Note that the hash here must represents a valid field element.
/// TODO: Maybe we should wrap Fr instead of [u8; 32] here.
#[derive(Copy, Debug, Clone, Eq, PartialEq, Default, Serialize, Deserialize)]
pub struct Hash(
    #[serde(serialize_with = "self::serialize_bytes_as_binary")]
    #[serde(deserialize_with = "self::deserialize_u256_from_binary")]
    pub [u8; 32],
);

// TODO: Maybe use something like protovalidate to automatically validate fields.
impl TryFrom<&[u8]> for Hash {
    type Error = Error;

    fn try_from(a: &[u8]) -> Result<Hash, Self::Error> {
        let a: [u8; 32] = a.try_into().map_err(|_e| {
            Error::InvalidArgument("Hash malformed (must be [u8; 32])".to_string())
        })?;
        a.try_into()
    }
}

// TODO: Maybe use something like protovalidate to automatically validate fields.
impl TryFrom<Vec<u8>> for Hash {
    type Error = Error;

    fn try_from(a: Vec<u8>) -> Result<Hash, Self::Error> {
        a.as_slice().try_into()
    }
}

// Check also [u8; 32] is a valid field element.
impl TryFrom<[u8; 32]> for Hash {
    type Error = Error;

    fn try_from(hash: [u8; 32]) -> Result<Hash, Self::Error> {
        Ok(Self(hash))
    }
}

impl From<Fr> for Hash {
    fn from(f: Fr) -> Self {
        Self(f.to_repr())
    }
}

impl From<Hash> for Fr {
    fn from(h: Hash) -> Fr {
        Fr::from_repr(h.0).unwrap()
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

impl Hash {
    pub fn hash_children(left: &Self, right: &Self) -> Self {
        let mut hasher = gen_merkle_hasher();
        let a = Fr::from(*left);
        let b = Fr::from(*right);
        hasher.update_exact(&[a, b]).into()
    }

    pub fn hash_data(data: &[u8]) -> Self {
        let data: [u8; 32] = data.clone().try_into().unwrap();
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
        let mut hasher = gen_merkle_leaf_hasher();
        // Upstream uses `update_exact` to obtain the hash result.
        // https://github.com/DelphinusLab/zkWasm-host-circuits/pull/75/files#diff-569acc27d1b9b0aa262ff90201af200d25432920c537df3c945fee07271ca2ed
        // Note that update_exact is not equvilent to update and suqeeze.
        // Only using update_exact can we obtain the new root in
        // https://github.com/DelphinusLab/zkWasm-rust/pull/14/files#diff-a1e31cd1b554d09f75df1ea4255aeaf3dff9f3093d378ae7f078368b5b2285b2
        let result = hasher.update_exact(&values);
        result.into()
    }

    pub const fn empty() -> Self {
        Self([0u8; 32])
    }

    /// depth start from 0 up to Self::height(). Example 20 height MongoMerkle, root depth=0, leaf depth=20
    pub fn get_default_hash_for_depth(depth: usize) -> Result<Hash, MerkleError> {
        if depth <= MERKLE_TREE_HEIGHT {
            Ok(DEFAULT_HASH_VEC[MERKLE_TREE_HEIGHT - depth])
        } else {
            Err(MerkleError::new(
                [0; 32].try_into().unwrap(),
                depth as u64,
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
        let new_hash = Self::hash_data(&data.0);
        if *hash != new_hash {
            return Err(Error::InvalidArgument(format!(
                "Hash not matching: {:?} hashed to {:?}, not {:?}",
                &data, &new_hash, &hash
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct LeafData(
    #[serde(serialize_with = "self::serialize_bytes_as_binary")]
    #[serde(deserialize_with = "self::deserialize_bytes_from_binary")]
    pub Vec<u8>,
);

impl Default for LeafData {
    fn default() -> Self {
        [0; 32].into()
    }
}

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
        value.0
    }
}

impl From<Vec<u8>> for LeafData {
    fn from(value: Vec<u8>) -> Self {
        LeafData(value)
    }
}

impl From<[u8; 32]> for LeafData {
    fn from(value: [u8; 32]) -> Self {
        Self(value.to_vec())
    }
}

pub fn deserialize_u64_as_binary<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    match Bson::deserialize(deserializer) {
        Ok(Bson::Binary(bytes)) => Ok({
            let c: [u8; 8] = bytes.bytes.try_into().unwrap();
            u64::from_le_bytes(c)
        }),
        Ok(..) => Err(SerdeError::invalid_value(Unexpected::Enum, &"Bson::Binary")),
        Err(e) => Err(e),
    }
}

pub fn serialize_u64_as_binary<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let binary = Bson::Binary(mongodb::bson::Binary {
        subtype: BinarySubtype::Generic,
        bytes: value.to_le_bytes().to_vec(),
    });
    binary.serialize(serializer)
}

pub fn deserialize_u256_from_binary<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
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

pub fn deserialize_bytes_from_binary<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    match Bson::deserialize(deserializer) {
        Ok(Bson::Binary(bytes)) => Ok(bytes.bytes.to_vec()),
        Ok(..) => Err(SerdeError::invalid_value(Unexpected::Enum, &"Bson::Binary")),
        Err(e) => Err(e),
    }
}

pub fn u256_to_bson(x: &[u8; 32]) -> Bson {
    Bson::Binary(mongodb::bson::Binary {
        subtype: BinarySubtype::Generic,
        bytes: (*x).into(),
    })
}

pub fn u64_to_bson(x: u64) -> Bson {
    Bson::Binary(mongodb::bson::Binary {
        subtype: BinarySubtype::Generic,
        bytes: x.to_le_bytes().to_vec(),
    })
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
    #[serde(serialize_with = "self::serialize_u64_as_binary")]
    #[serde(deserialize_with = "self::deserialize_u64_as_binary")]
    pub index: u64,
    pub hash: Hash,
    pub left: Hash,
    pub right: Hash,
    #[serde(serialize_with = "self::serialize_bytes_as_binary")]
    #[serde(deserialize_with = "self::deserialize_u256_from_binary")]
    pub data: [u8; 32],
}

impl TryFrom<Node> for MerkleRecord {
    type Error = Error;

    fn try_from(n: Node) -> Result<Self, Self::Error> {
        let hash: Hash = n.hash.as_slice().try_into()?;
        if n.node_type == NodeType::NodeLeaf as i32 {
            match n.node_data {
                Some(NodeData::Data(_)) => {
                    let record = MerkleRecord::new_leaf(n.index, hash);
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

impl TryFrom<(MerkleRecord, DataHashRecord)> for Node {
    type Error = Error;

    fn try_from(record: (MerkleRecord, DataHashRecord)) -> Result<Self, Self::Error> {
        let merkle_record = record.0;
        let datahash_record = record.1;
        let node = Self::try_from(merkle_record);
        if node.is_ok() {
            return node;
        }

        if merkle_record.hash != datahash_record.hash {
            return Err(Error::InvalidArgument("Hash mismatched".to_string()));
        }

        let node_type = get_node_type(merkle_record.index(), MERKLE_TREE_HEIGHT);
        if node_type != NodeType::NodeLeaf {
            return Err(Error::InvalidArgument("Unknown node type".to_string()));
        }
        let node_data = { NodeData::Data(datahash_record.data) };
        Ok(Node {
            index: merkle_record.index(),
            hash: merkle_record.hash().into(),
            node_type: node_type.into(),
            node_data: Some(node_data),
        })
    }
}

impl TryFrom<MerkleRecord> for Node {
    type Error = Error;

    fn try_from(merkle_record: MerkleRecord) -> Result<Self, Self::Error> {
        let index = merkle_record.index();
        let hash = merkle_record.hash().into();
        let node_type = get_node_type(index, MERKLE_TREE_HEIGHT);
        if node_type != NodeType::NodeNonLeaf {
            return Err(Error::InconsistentData("Unknown node type".to_string()));
        }
        let node_data = {
            let left_child_hash = merkle_record
                .left()
                .ok_or(Error::InconsistentData(
                    "Nonleaf node has no children".to_string(),
                ))?
                .into();
            let right_child_hash = merkle_record
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
    fn index(&self) -> u64 {
        self.index
    }
    fn hash(&self) -> Hash {
        self.hash
    }
    fn set(&mut self, data: &[u8]) {
        self.hash = Hash::hash_data(data);
    }
    fn right(&self) -> Option<Hash> {
        Some(self.right)
    }
    fn left(&self) -> Option<Hash> {
        Some(self.left)
    }
}

impl MerkleRecord {
    pub fn new(index: u64) -> Self {
        MerkleRecord {
            index,
            hash: [0; 32].try_into().unwrap(),
            left: [0; 32].try_into().unwrap(),
            right: [0; 32].try_into().unwrap(),
            data: [0; 32],
        }
    }

    pub fn new_leaf(index: u64, hash: impl Into<Hash>) -> Self {
        let mut record = MerkleRecord::new(index);
        record.hash = hash.into();
        record
    }

    pub fn new_non_leaf(index: u64, left: impl Into<Hash>, right: impl Into<Hash>) -> Self {
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

    pub fn get_default_record(index: u64) -> Result<Self, MerkleError> {
        let height = (index + 1).ilog2() as usize;
        let default = Hash::get_default_hash_for_depth(height)?;
        let child_hash = if height == MERKLE_TREE_HEIGHT {
            [0; 32].try_into().unwrap()
        } else {
            Hash::get_default_hash_for_depth(height + 1)?
        };
        Ok(MerkleRecord {
            index,
            hash: default,
            left: child_hash,
            right: child_hash,
            data: [0; 32],
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct DataHashRecord {
    pub hash: Hash,
    #[serde(serialize_with = "self::serialize_bytes_as_binary")]
    #[serde(deserialize_with = "self::deserialize_bytes_from_binary")]
    pub data: Vec<u8>,
}

impl DataHashRecord {
    pub fn new(hash: Hash, data: Vec<u8>) -> Self {
        Self { hash, data }
    }

    pub const fn empty() -> Self {
        Self {
            // Note that we use the hash of [0u8; 32] as default hash, while empty vector to represent empty data
            hash: Hash::empty(),
            data: vec![],
        }
    }
}

impl MongoMerkle {
    pub async fn get_client() -> KvPairClient<Channel> {
        let server =
            std::env::var("KVPAIR_GRPC_SERVER_URL").unwrap_or("http://localhost:50051".to_string());
        KvPairClient::connect(server)
            .await
            .expect("Connect gRPC server")
    }

    pub fn height() -> usize {
        MERKLE_TREE_HEIGHT
    }
    fn empty_leaf(index: u64) -> MerkleRecord {
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

    pub async fn set_root(&mut self, hash: Hash) -> Result<SetRootResponse, Status> {
        let response = self
            .client
            .set_root(Request::new(SetRootRequest {
                contract_id: Some(self.contract_id.into()),
                hash: hash.into(),
            }))
            .await?;
        dbg!(&response);

        Ok(response.into_inner())
    }

    pub async fn get_leaf(
        &mut self,
        index: u64,
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
        index: u64,
        leaf_data: LeafData,
        proof_type: ProofType,
    ) -> Result<SetLeafResponse, Status> {
        let proof_type = proof_type.into();
        let response = self
            .client
            .set_leaf(Request::new(SetLeafRequest {
                index,
                hash: None,
                data: Some(leaf_data.0),
                proof_type,
                contract_id: Some(self.contract_id.into()),
            }))
            .await?;
        dbg!(&response);

        Ok(response.into_inner())
    }

    pub async fn get_non_leaf(
        &mut self,
        index: u64,
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
        index: u64,
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

impl MerkleTree<Hash, MERKLE_TREE_HEIGHT> for MongoMerkle {
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
        index: u64,
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

    fn get_node_with_hash(&mut self, index: u64, hash: &Hash) -> Result<Self::Node, MerkleError> {
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
        executor::block_on(self.set_leaf(leaf.index, Default::default(), ProofType::ProofEmpty))
            .map_err(|e| {
                dbg!(e);
                MerkleError::new(leaf.hash, leaf.index, MerkleErrorCode::InvalidOther)
            })?;
        Ok(())
    }
}

impl Node {
    /// This corresponds to data in simple_get/simple_set of zkWasm-rust.
    /// Here we create a Node that has empty vector as its data, although
    /// technically an empty vector does not hash to the hash in the merkle record.
    pub fn new_simple_leaf(index: u64, hash: Hash) -> Self {
        Node {
            index,
            hash: hash.into(),
            node_type: NodeType::NodeLeaf.into(),
            node_data: Some(NodeData::Data(vec![])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn bytes_to_u64(bytes: &[u8; 32]) -> [u64; 4] {
        let r = bytes
            .to_vec()
            .chunks_exact(8)
            .map(|x| u64::from_le_bytes(x.try_into().unwrap()))
            .collect::<Vec<_>>();
        r.try_into().unwrap()
    }

    #[test]
    fn show_default_root() {
        for (i, h) in DEFAULT_HASH_VEC.iter().enumerate() {
            dbg!(i, hex::encode(h.0));
        }
    }

    #[test]
    fn test_new_merkle_root() {
        let root = &DEFAULT_HASH_VEC[32].0;
        assert_eq!(
            bytes_to_u64(root),
            // Root obtained from
            // https://github.com/DelphinusLab/zkWasm-rust/blob/757b1326959474e136e2253d9ced18456195a2d6/src/merkle.rs#L62-L68
            [
                14789582351289948625,
                10919489180071018470,
                10309858136294505219,
                2839580074036780766
            ]
        );
    }
}
