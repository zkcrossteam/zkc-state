use std::borrow::Borrow;

use crate::kvpair::{LeafData, MERKLE_TREE_HEIGHT};
use crate::merkle::{get_offset, get_path, get_sibling_index, leaf_check, MerkleNode, MerkleProof};
use crate::Error;

use super::kvpair::{hash_to_bson, ContractId, Hash, MerkleRecord};
use mongodb::bson::{doc, to_bson, Document};
use mongodb::error::{TRANSIENT_TRANSACTION_ERROR, UNKNOWN_TRANSACTION_COMMIT_RESULT};
use mongodb::options::{
    Acknowledgment, FindOneOptions, InsertOneOptions, ReadConcern, ReplaceOptions,
    TransactionOptions, UpdateModifications, UpdateOptions, WriteConcern,
};
use mongodb::results::{InsertOneResult, UpdateResult};
use mongodb::{Client, ClientSession, Collection};
use tonic::{Request, Response, Status};

use super::proto::kv_pair_server::KvPair;
use super::proto::Proof;
use super::proto::ProofType;
use super::proto::*;

pub struct MongoKvPair {
    client: Client,
}

#[derive(Debug)]
pub struct MongoCollection<T> {
    collection: Collection<T>,
    session: Option<ClientSession>,
}

impl<T> MongoCollection<T> {
    fn get_database_name() -> String {
        "zkwasmkvpair".to_string()
    }

    fn get_collection_name(contract_id: &ContractId) -> String {
        format!("MERKLEDATA_{}", hex::encode(contract_id.0))
    }

    pub async fn new(
        client: Client,
        contract_id: &ContractId,
        with_session: bool,
    ) -> Result<Self, mongodb::error::Error> {
        let session = if with_session {
            let mut session = client.start_session(None).await?;
            let options = TransactionOptions::builder()
                .read_concern(ReadConcern::majority())
                .write_concern(WriteConcern::builder().w(Acknowledgment::Majority).build())
                .build();
            session.start_transaction(options).await?;
            Some(session)
        } else {
            None
        };
        let database = client.clone().database(Self::get_database_name().as_str());
        let collection_name = Self::get_collection_name(contract_id);
        let collection = database.collection::<T>(collection_name.as_str());
        Ok(Self {
            collection,
            session,
        })
    }

    pub async fn commit(&mut self) -> Result<(), mongodb::error::Error> {
        if let Some(mut session) = self.session.take() {
            // A "TransientTransactionError" label indicates that the entire transaction can be retried
            // with a reasonable expectation that it will succeed.
            // An "UnknownTransactionCommitResult" label indicates that it is unknown whether the
            // commit has satisfied the write concern associated with the transaction. If an error
            // with this label is returned, it is safe to retry the commit until the write concern is
            // satisfied or an error without the label is returned.
            loop {
                let result = session.commit_transaction().await;
                if let Err(ref error) = result {
                    if error.contains_label(UNKNOWN_TRANSACTION_COMMIT_RESULT)
                        || error.contains_label(TRANSIENT_TRANSACTION_ERROR)
                    {
                        continue;
                    }
                }
                result?
            }
        }
        Ok(())
    }

    pub async fn drop(&self) -> Result<(), mongodb::error::Error> {
        let options = mongodb::options::DropCollectionOptions::builder().build();
        self.collection.drop(options).await?;
        Ok(())
    }
}

impl MongoCollection<MerkleRecord> {
    // Special ObjectId to track current root.
    pub fn get_current_root_object_id() -> mongodb::bson::oid::ObjectId {
        mongodb::bson::oid::ObjectId::from_bytes([0; 12])
    }

    pub async fn find_one(
        &mut self,
        filter: impl Into<Option<Document>>,
        options: impl Into<Option<FindOneOptions>>,
    ) -> Result<Option<MerkleRecord>, mongodb::error::Error> {
        let result = match self.session.as_mut() {
            Some(session) => {
                self.collection
                    .find_one_with_session(filter, options, session)
                    .await?
            }
            _ => self.collection.find_one(filter, options).await?,
        };
        Ok(result)
    }

    pub async fn insert_one(
        &mut self,
        doc: impl Borrow<MerkleRecord>,
        options: impl Into<Option<InsertOneOptions>>,
    ) -> Result<InsertOneResult, mongodb::error::Error> {
        let result = match self.session.as_mut() {
            Some(session) => {
                self.collection
                    .insert_one_with_session(doc, options, session)
                    .await?
            }
            _ => self.collection.insert_one(doc, options).await?,
        };
        Ok(result)
    }

    pub async fn replace_one(
        &mut self,
        query: Document,
        replacement: impl Borrow<MerkleRecord>,
        options: impl Into<Option<ReplaceOptions>>,
    ) -> Result<UpdateResult, mongodb::error::Error> {
        let result = match self.session.as_mut() {
            Some(session) => {
                self.collection
                    .replace_one_with_session(query, replacement, options, session)
                    .await?
            }
            _ => {
                self.collection
                    .replace_one(query, replacement, options)
                    .await?
            }
        };
        Ok(result)
    }

    pub async fn update_one(
        &mut self,
        query: Document,
        update: impl Into<UpdateModifications>,
        options: impl Into<Option<UpdateOptions>>,
    ) -> Result<UpdateResult, mongodb::error::Error> {
        let result = match self.session.as_mut() {
            Some(session) => {
                self.collection
                    .update_one_with_session(query, update, options, session)
                    .await?
            }
            _ => self.collection.update_one(query, update, options).await?,
        };
        Ok(result)
    }

    pub async fn get_merkle_record(
        &mut self,
        index: u32,
        hash: &Hash,
    ) -> Result<Option<MerkleRecord>, Error> {
        let mut filter = doc! {};
        filter.insert("index", index);
        filter.insert("hash", hash_to_bson(&hash));
        let record = self.find_one(filter, None).await?;
        if record.is_some() {
            return Ok(record);
        }
        Ok(MerkleRecord::get_default_record(index).ok())
    }

    pub async fn must_get_merkle_record(
        &mut self,
        index: u32,
        hash: &Hash,
    ) -> Result<MerkleRecord, Error> {
        let record = self.get_merkle_record(index, hash).await?;
        record.ok_or(Error::Precondition("Merkle record not found".to_string()))
    }

    pub async fn get_root_merkle_record(&mut self) -> Result<Option<MerkleRecord>, Error> {
        let filter = doc! {"_id": Self::get_current_root_object_id()};
        let record = self.find_one(filter, None).await?;
        dbg!(record);
        if record.is_some() {
            return Ok(record);
        }
        Ok(MerkleRecord::get_default_record(0).ok())
    }

    pub async fn must_get_root_merkle_record(&mut self) -> Result<MerkleRecord, Error> {
        let record = self.get_root_merkle_record().await?;
        record.ok_or(Error::Precondition("Merkle record not found".to_string()))
    }

    pub async fn insert_merkle_record(
        &mut self,
        record: &MerkleRecord,
    ) -> Result<MerkleRecord, Error> {
        let mut filter = doc! {};
        filter.insert("index", record.index);
        filter.insert("hash", hash_to_bson(&record.hash));
        let exists = self.find_one(filter, None).await?;
        exists.map_or(
            {
                let result = self.insert_one(record, None).await?;
                dbg!(&result);
                Ok(*record)
            },
            |record| {
                //println!("find existing node, preventing duplicate");
                Ok(record)
            },
        )
    }

    pub async fn insert_leaf_node(
        &mut self,
        index: u32,
        hash: &Hash,
        data: &LeafData,
    ) -> Result<MerkleRecord, Error> {
        let mut record = MerkleRecord::default();
        record.index = index;
        record.hash = *hash;
        record.data = *data;
        self.insert_merkle_record(&record).await
    }

    pub async fn insert_non_leaf_node(
        &mut self,
        index: u32,
        hash: Hash,
        left: Hash,
        right: Hash,
    ) -> Result<MerkleRecord, Error> {
        let mut record = MerkleRecord::default();
        record.index = index;
        record.hash = hash;
        record.left = left;
        record.right = right;
        self.insert_merkle_record(&record).await
    }

    pub async fn update_root_merkle_record(
        &mut self,
        record: &MerkleRecord,
    ) -> Result<MerkleRecord, Error> {
        let filter = doc! {"_id": Self::get_current_root_object_id()};
        let update = doc! {
            "$set": {
                "hash": to_bson(&Hash::from(record.hash)).unwrap(),
                "left": to_bson(&Hash::from(record.left)).unwrap(),
                "right": to_bson(&Hash::from(record.right)).unwrap(),
                "data": to_bson(&LeafData::from(record.data)).unwrap(),
            },
            // We use this to track the number of root updates.
            "$inc": {
                "index": 1i64,
            },
        };
        let options = UpdateOptions::builder().upsert(true).build();
        let result = self.update_one(filter, update, options).await?;
        dbg!(&result);
        Ok(*record)
    }

    pub async fn get_leaf_and_proof(
        &mut self,
        index: u32,
    ) -> Result<(MerkleRecord, MerkleProof<Hash, MERKLE_TREE_HEIGHT>), Error> {
        leaf_check(index, MERKLE_TREE_HEIGHT)?;
        let paths = get_path(index, MERKLE_TREE_HEIGHT)?;
        // We push the search from the top
        let mut acc = 0;
        let mut acc_node = self.must_get_root_merkle_record().await?;
        let root_hash = acc_node.hash;
        let mut assist = Vec::with_capacity(MERKLE_TREE_HEIGHT);
        for child in paths {
            let (hash, sibling_hash) = if (acc + 1) * 2 == child + 1 {
                // left child
                (acc_node.left().unwrap(), acc_node.right().unwrap())
            } else {
                assert!((acc + 1) * 2 == child);
                (acc_node.right().unwrap(), acc_node.left().unwrap())
            };
            let sibling = get_sibling_index(child);
            let sibling_node = self
                .must_get_merkle_record(sibling, &sibling_hash.into())
                .await?;
            acc = child;
            acc_node = self.must_get_merkle_record(acc, &hash.into()).await?;
            assist.push(Hash::from(sibling_node.hash()));
        }
        let hash = acc_node.hash();
        Ok((
            acc_node,
            MerkleProof {
                source: hash.into(),
                root: root_hash.into(),
                assist: assist.try_into().unwrap(),
                index,
            },
        ))
    }

    pub async fn set_leaf_and_get_proof(
        &mut self,
        leaf: &MerkleRecord,
    ) -> Result<MerkleProof<Hash, MERKLE_TREE_HEIGHT>, Error> {
        let index = leaf.index();
        let mut hash = leaf.hash();
        let (_, mut proof) = self.get_leaf_and_proof(index).await?;
        proof.source = hash.clone().into();
        let mut p = get_offset(index);
        self.insert_merkle_record(leaf).await?;
        for i in 0..MERKLE_TREE_HEIGHT {
            let cur_hash = hash;
            let depth = MERKLE_TREE_HEIGHT - i - 1;
            let (left, right) = if p % 2 == 1 {
                (proof.assist[depth], cur_hash)
            } else {
                (cur_hash, proof.assist[depth])
            };
            hash = Hash::hash_children(&left, &right);
            p /= 2;
            let index = p + (1 << depth) - 1;
            let mut record = MerkleRecord::default();
            record.index = index;
            record.hash = hash;
            record.left = left;
            record.right = right;
            self.insert_merkle_record(&record).await?;
            if index == 0 {
                self.update_root_merkle_record(&record).await?;
            }
        }
        Ok(proof)
    }
}

impl MongoKvPair {
    pub async fn new() -> Self {
        let mongodb_uri: String =
            std::env::var("MONGODB_URI").unwrap_or("mongodb://localhost:27017".to_string());
        let client = Client::with_uri_str(&mongodb_uri).await.unwrap();
        MongoKvPair::new_with_client(client)
    }

    pub fn new_with_client(client: Client) -> Self {
        Self { client }
    }

    pub async fn new_collection<T>(
        &self,
        contract_id: &ContractId,
        with_session: bool,
    ) -> Result<MongoCollection<T>, Error> {
        Ok(MongoCollection::new(self.client.clone(), contract_id, with_session).await?)
    }
}

fn get_contract_id<T>(request: &Request<T>) -> Result<ContractId, Status> {
    let id = request
        .metadata()
        .get("x-auth-contract-id")
        .ok_or(Status::unauthenticated("Contract id not found"))?;
    let contract_id = id
        .to_str()
        .map_err(|e| Status::unauthenticated(format!("Invalid Contract id: {e}")))?
        .try_into()
        .map_err(|e| Status::unauthenticated(format!("Invalid Contract id: {e}")))?;
    dbg!(&contract_id);
    Ok(contract_id)
}

#[tonic::async_trait]
impl KvPair for MongoKvPair {
    async fn get_root(
        &self,
        request: Request<GetRootRequest>,
    ) -> std::result::Result<Response<GetRootResponse>, Status> {
        dbg!(&request);
        let contract_id = get_contract_id(&request).unwrap_or_default();
        let mut collection = self.new_collection(&contract_id, false).await?;
        let record = collection.must_get_root_merkle_record().await?;
        Ok(Response::new(GetRootResponse {
            root: record.hash().into(),
        }))
    }

    async fn set_root(
        &self,
        request: Request<SetRootRequest>,
    ) -> std::result::Result<Response<SetRootResponse>, Status> {
        dbg!(request);
        unimplemented!()
    }

    async fn get_leaf(
        &self,
        request: Request<GetLeafRequest>,
    ) -> std::result::Result<Response<GetLeafResponse>, Status> {
        dbg!(&request);
        let contract_id = get_contract_id(&request).unwrap_or_default();
        let request = request.into_inner();
        let mut collection = self.new_collection(&contract_id, false).await?;
        let index = request.index;
        let proof_v0 = ProofType::ProofV0 as i32;
        let (record, proof) = match (request.hash.as_ref(), request.proof_type) {
            // Get merkle records in a faster way
            (Some(hash), _) if request.proof_type != proof_v0 => {
                let hash: Hash = hash.as_slice().try_into()?;
                let record = collection.must_get_merkle_record(index, &hash).await?;
                (record, None)
            }
            (_, _) => {
                let (record, proof) = collection.get_leaf_and_proof(index).await?;
                if request.hash.is_some() {
                    let hash: Hash = request.hash.unwrap().as_slice().try_into()?;
                    if hash != proof.source {
                        return Err(
                            Error::InvalidArgument("Leaf not in current root".to_string()).into(),
                        );
                    }
                }
                let proof_bytes = if request.proof_type == proof_v0 {
                    Some(Proof {
                        proof_type: request.proof_type,
                        proof: bincode::serialize(&proof).unwrap(),
                    })
                } else {
                    None
                };
                (record, proof_bytes)
            }
        };
        let node = record.try_into()?;
        collection.commit().await.map_err(Error::from)?;
        dbg!(&record, &node, &proof);
        Ok(Response::new(GetLeafResponse {
            node: Some(node),
            proof,
        }))
    }

    async fn set_leaf(
        &self,
        request: Request<SetLeafRequest>,
    ) -> std::result::Result<Response<GetLeafResponse>, Status> {
        dbg!(&request);
        let contract_id = get_contract_id(&request).unwrap_or_default();
        let request = request.into_inner();
        // TODO: Should use session here
        let mut collection = self.new_collection(&contract_id, false).await?;
        let index = request.index;
        let hash: Hash = request.hash.as_slice().try_into()?;
        let leaf_data: LeafData = request.leaf_data.as_slice().try_into()?;
        let record = MerkleRecord::new_leaf(index, hash, leaf_data);
        let proof = collection.set_leaf_and_get_proof(&record).await?;
        let proof = if request.proof_type == ProofType::ProofV0 as i32 {
            Some(Proof {
                proof_type: request.proof_type,
                proof: bincode::serialize(&proof).unwrap(),
            })
        } else {
            None
        };
        let node = record.try_into()?;
        collection.commit().await.map_err(Error::from)?;
        dbg!(&record, &node);
        Ok(Response::new(GetLeafResponse {
            node: Some(node),
            proof,
        }))
    }

    async fn get_non_leaf(
        &self,
        request: Request<GetNonLeafRequest>,
    ) -> std::result::Result<Response<GetNonLeafResponse>, Status> {
        dbg!(request);
        unimplemented!()
    }

    async fn set_non_leaf(
        &self,
        request: Request<SetNonLeafRequest>,
    ) -> std::result::Result<Response<SetNonLeafResponse>, Status> {
        dbg!(request);
        unimplemented!()
    }
}




