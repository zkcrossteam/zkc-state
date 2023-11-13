use std::borrow::Borrow;

use crate::kvpair::MERKLE_TREE_HEIGHT;
use crate::merkle::{get_offset, get_path, get_sibling_index, leaf_check, MerkleNode, MerkleProof};
use crate::Error;

use super::kvpair::{hash_to_bson, u64_to_bson, ContractId, DataHashRecord, Hash, MerkleRecord};
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

#[derive(Copy, Clone, Debug)]
pub struct MongoKvPairTestConfig {
    pub contract_id: ContractId,
}

#[derive(Clone, Debug)]
pub struct MongoKvPair {
    client: Client,
    test_config: Option<MongoKvPairTestConfig>,
}

#[derive(Debug)]
pub struct MongoCollection<T, R> {
    merkle_collection: Collection<T>,
    datahash_collection: Collection<R>,
    session: Option<ClientSession>,
}

impl<T, R> MongoCollection<T, R> {
    fn get_database_name() -> String {
        "zkwasm-mongo-merkle".to_string()
    }

    fn get_merkle_collection_name(contract_id: &ContractId) -> String {
        format!("MERKLEDATA_{}", hex::encode(contract_id.0))
    }

    fn get_data_collection_name(contract_id: &ContractId) -> String {
        format!("DATAHASH_{}", hex::encode(contract_id.0))
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
        let merkle_collection_name = Self::get_merkle_collection_name(contract_id);
        let merkle_collection = database.collection::<T>(merkle_collection_name.as_str());
        let datahash_collection_name = Self::get_data_collection_name(contract_id);
        let datahash_collection = database.collection::<R>(datahash_collection_name.as_str());
        dbg!(merkle_collection_name, datahash_collection_name);
        Ok(Self {
            merkle_collection,
            datahash_collection,
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
        self.merkle_collection.drop(options.clone()).await?;
        self.datahash_collection.drop(options).await?;
        Ok(())
    }
}

impl MongoCollection<MerkleRecord, DataHashRecord> {
    // Special ObjectId to track current root.
    pub fn get_current_root_object_id() -> mongodb::bson::oid::ObjectId {
        mongodb::bson::oid::ObjectId::from_bytes([0; 12])
    }

    pub async fn find_one_merkle_record(
        &mut self,
        filter: impl Into<Option<Document>>,
        options: impl Into<Option<FindOneOptions>>,
    ) -> Result<Option<MerkleRecord>, mongodb::error::Error> {
        let result = match self.session.as_mut() {
            Some(session) => {
                self.merkle_collection
                    .find_one_with_session(filter, options, session)
                    .await?
            }
            _ => self.merkle_collection.find_one(filter, options).await?,
        };
        Ok(result)
    }

    pub async fn insert_one_merkle_record(
        &mut self,
        doc: impl Borrow<MerkleRecord>,
        options: impl Into<Option<InsertOneOptions>>,
    ) -> Result<InsertOneResult, mongodb::error::Error> {
        let result = match self.session.as_mut() {
            Some(session) => {
                self.merkle_collection
                    .insert_one_with_session(doc, options, session)
                    .await?
            }
            _ => self.merkle_collection.insert_one(doc, options).await?,
        };
        Ok(result)
    }

    pub async fn replace_one_merkle_record(
        &mut self,
        query: Document,
        replacement: impl Borrow<MerkleRecord>,
        options: impl Into<Option<ReplaceOptions>>,
    ) -> Result<UpdateResult, mongodb::error::Error> {
        let result = match self.session.as_mut() {
            Some(session) => {
                self.merkle_collection
                    .replace_one_with_session(query, replacement, options, session)
                    .await?
            }
            _ => {
                self.merkle_collection
                    .replace_one(query, replacement, options)
                    .await?
            }
        };
        Ok(result)
    }

    pub async fn update_one_merkle_record(
        &mut self,
        query: Document,
        update: impl Into<UpdateModifications>,
        options: impl Into<Option<UpdateOptions>>,
    ) -> Result<UpdateResult, mongodb::error::Error> {
        let result = match self.session.as_mut() {
            Some(session) => {
                self.merkle_collection
                    .update_one_with_session(query, update, options, session)
                    .await?
            }
            _ => {
                self.merkle_collection
                    .update_one(query, update, options)
                    .await?
            }
        };
        Ok(result)
    }

    pub async fn get_merkle_record(
        &mut self,
        index: u64,
        hash: &Hash,
    ) -> Result<Option<MerkleRecord>, Error> {
        let mut filter = doc! {};
        filter.insert("index", u64_to_bson(index));
        filter.insert("hash", hash_to_bson(hash));
        let record = self.find_one_merkle_record(filter, None).await?;
        if record.is_some() {
            return Ok(record);
        }
        let default_record = MerkleRecord::get_default_record(index)?;
        if default_record.hash == *hash {
            Ok(Some(default_record))
        } else {
            Ok(None)
        }
    }

    pub async fn must_get_merkle_record(
        &mut self,
        index: u64,
        hash: &Hash,
    ) -> Result<MerkleRecord, Error> {
        let record = self.get_merkle_record(index, hash).await?;
        record.ok_or(Error::Precondition("Merkle record not found".to_string()))
    }

    pub async fn get_root_merkle_record(&mut self) -> Result<Option<MerkleRecord>, Error> {
        let filter = doc! {"_id": Self::get_current_root_object_id()};
        let record = self.find_one_merkle_record(filter, None).await?;
        dbg!(&record);
        if record.is_some() {
            return Ok(record);
        }
        Ok(MerkleRecord::get_default_record(0).ok())
    }

    pub async fn must_get_root_merkle_record(&mut self) -> Result<MerkleRecord, Error> {
        let record = self.get_root_merkle_record().await?;
        assert!(record.is_some(), "BUG!!! Root record not found.");
        Ok(record.unwrap())
    }

    pub async fn insert_merkle_record(
        &mut self,
        record: &MerkleRecord,
    ) -> Result<MerkleRecord, Error> {
        let mut filter = doc! {};
        filter.insert("index", u64_to_bson(record.index));
        filter.insert("hash", hash_to_bson(&record.hash));
        let exists = self.find_one_merkle_record(filter, None).await?;
        exists.map_or(
            {
                let result = self.insert_one_merkle_record(record, None).await?;
                dbg!(record, &result);
                Ok(*record)
            },
            |record| {
                //println!("find existing node, preventing duplicate");
                Ok(record)
            },
        )
    }

    pub async fn insert_non_leaf_node(
        &mut self,
        index: u64,
        left: Hash,
        right: Hash,
    ) -> Result<MerkleRecord, Error> {
        let record = MerkleRecord::new_non_leaf(index, left, right);
        self.insert_merkle_record(&record).await
    }

    pub async fn update_root_merkle_record(
        &mut self,
        record: &MerkleRecord,
    ) -> Result<MerkleRecord, Error> {
        let filter = doc! {"_id": Self::get_current_root_object_id()};
        let update = doc! {
            "$set": {
                "index": u64_to_bson(0),
                "hash": to_bson(&record.hash).unwrap(),
                "left": to_bson(&record.left).unwrap(),
                "right": to_bson(&record.right).unwrap(),
            },
        };
        let options = UpdateOptions::builder().upsert(true).build();
        let result = self
            .update_one_merkle_record(filter, update, options)
            .await?;
        dbg!(&result);
        Ok(*record)
    }

    pub async fn get_leaf_and_proof(
        &mut self,
        index: u64,
    ) -> Result<(MerkleRecord, MerkleProof<Hash, MERKLE_TREE_HEIGHT>), Error> {
        leaf_check(index, MERKLE_TREE_HEIGHT)?;
        let paths = get_path(index, MERKLE_TREE_HEIGHT)?;
        // We push the search from the top
        let mut acc = 0;
        let mut acc_node = self.must_get_root_merkle_record().await?;
        let root_hash = acc_node.hash;
        let mut assist = Vec::with_capacity(MERKLE_TREE_HEIGHT);
        for child in paths {
            let is_left_child = (acc + 1) * 2 == child + 1;
            let is_right_child = (acc + 1) * 2 == child;
            assert!(is_left_child || is_right_child);
            let (hash, sibling_hash) = if is_left_child {
                (acc_node.left().unwrap(), acc_node.right().unwrap())
            } else {
                (acc_node.right().unwrap(), acc_node.left().unwrap())
            };
            let sibling = get_sibling_index(child);
            let sibling_node = self.must_get_merkle_record(sibling, &sibling_hash).await?;
            acc = child;
            acc_node = self.must_get_merkle_record(acc, &hash).await?;
            assist.push(sibling_node.hash());
        }
        let hash = acc_node.hash();
        Ok((
            acc_node,
            MerkleProof {
                source: hash,
                root: root_hash,
                assist,
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
        proof.source = hash;
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
            let record = MerkleRecord::new_non_leaf(index, left, right);
            assert_eq!(record.hash, hash);
            self.insert_merkle_record(&record).await?;
            if index == 0 {
                self.update_root_merkle_record(&record).await?;
            }
        }
        Ok(proof)
    }

    pub async fn find_one_datahash_record(
        &mut self,
        filter: impl Into<Option<Document>>,
        options: impl Into<Option<FindOneOptions>>,
    ) -> Result<Option<DataHashRecord>, mongodb::error::Error> {
        let result = match self.session.as_mut() {
            Some(session) => {
                self.datahash_collection
                    .find_one_with_session(filter, options, session)
                    .await?
            }
            _ => self.datahash_collection.find_one(filter, options).await?,
        };
        Ok(result)
    }

    pub async fn insert_one_datahash_record(
        &mut self,
        doc: impl Borrow<DataHashRecord>,
        options: impl Into<Option<InsertOneOptions>>,
    ) -> Result<InsertOneResult, mongodb::error::Error> {
        let result = match self.session.as_mut() {
            Some(session) => {
                self.datahash_collection
                    .insert_one_with_session(doc, options, session)
                    .await?
            }
            _ => self.datahash_collection.insert_one(doc, options).await?,
        };
        Ok(result)
    }

    pub async fn insert_datahash_record(
        &mut self,
        record: &DataHashRecord,
    ) -> Result<DataHashRecord, Error> {
        let mut filter = doc! {};
        filter.insert("hash", hash_to_bson(&record.hash));
        let exists = self.find_one_datahash_record(filter, None).await?;
        dbg!(&exists);
        exists.map_or(
            {
                let result = self.insert_one_datahash_record(record, None).await?;
                dbg!(&record, &result);
                Ok(record.clone())
            },
            |record| {
                //println!("find existing node, preventing duplicate");
                Ok(record.clone())
            },
        )
    }

    pub async fn get_datahash_record(
        &mut self,
        hash: &Hash,
    ) -> Result<Option<DataHashRecord>, Error> {
        dbg!(hash);
        if *hash == DataHashRecord::default().hash {
            return Ok(Some(DataHashRecord::default()));
        }
        let mut filter = doc! {};
        filter.insert("hash", hash_to_bson(hash));
        let record = self.find_one_datahash_record(filter, None).await?;
        Ok(record)
    }

    pub async fn must_get_datahash_record(&mut self, hash: &Hash) -> Result<DataHashRecord, Error> {
        let record = self.get_datahash_record(hash).await?;
        record.ok_or(Error::Precondition("Datahash record not found".to_string()))
    }
}

impl MongoKvPair {
    pub async fn new() -> Self {
        let mongodb_uri: String =
            std::env::var("MONGODB_URI").unwrap_or("mongodb://localhost:27017".to_string());
        let client = Client::with_uri_str(&mongodb_uri).await.unwrap();
        // Eagerly connect to mongodb server to fail faster.
        let _ = client
            .list_database_names(
                doc! {
                    "name": MongoCollection::<(), ()>::get_database_name(),
                },
                None,
            )
            .await
            .expect("List databases");
        MongoKvPair::new_with_client(client)
    }

    pub async fn new_with_test_config(test_config: Option<MongoKvPairTestConfig>) -> Self {
        let mut client = Self::new().await;
        client.test_config = test_config;
        client
    }

    fn new_with_client(client: Client) -> Self {
        Self {
            client,
            test_config: None,
        }
    }

    pub async fn new_collection<T, R>(
        &self,
        contract_id: &ContractId,
        with_session: bool,
    ) -> Result<MongoCollection<T, R>, Error> {
        Ok(MongoCollection::new(self.client.clone(), contract_id, with_session).await?)
    }

    pub async fn drop_test_collection(&self) -> Result<(), Error> {
        if let Some(test_config) = &self.test_config {
            let collection = self
                .new_collection::<MerkleRecord, DataHashRecord>(&test_config.contract_id, false)
                .await?;
            collection.drop().await?;
        }
        Ok(())
    }

    // Validate the contract id passed from http request or gRPC request parameter.
    // TODO: This function does nothing yet.
    fn validate_contract_id<T>(
        &self,
        _request: &Request<T>,
        _contract_id: &ContractId,
    ) -> Result<(), Status> {
        Ok(())
    }

    fn get_contract_id_from_request_context<T>(
        &self,
        request: &Request<T>,
    ) -> Result<ContractId, Status> {
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
        self.validate_contract_id(request, &contract_id)?;
        Ok(contract_id)
    }

    fn get_contract_id_from_request_parameters<T>(
        &self,
        request: &Request<T>,
        contract_id: &[u8],
    ) -> Result<ContractId, Status> {
        let contract_id: ContractId = contract_id.try_into()?;
        self.validate_contract_id(request, &contract_id)?;
        Ok(contract_id)
    }

    // Ideally the contract id should be obtained from the request context (e.g. lookup the
    // contract id coresponding to the token in the http header or use the contract id passed from http header directly).
    // But we have to take care of a few things.
    // 1. When we are testing the functionality of this program, we hard code a contract id in the
    //    test config. If that is the case, we use this contract id directly.
    // 2. Since the construct meothod of MerkleTree trait expects a contract_id, we need a way for
    //    the client to specify the contract id directly. In this case, we use the contract id from
    //    the gRPC request. We may need to validate the legality of this contract id. But we
    //    currently do nothing.
    // 3. Currently, if contract_id is not passed from any of these methods (test config, gRPC
    //    request parameter and http header), we just use the default contract id. This is only
    //    used to facliliate development. We MUST remove this when we are ready.
    fn get_contract_id<T>(
        &self,
        request: &Request<T>,
        contract_id: &Option<Vec<u8>>,
    ) -> Result<ContractId, Status> {
        if let Some(test_config) = &self.test_config {
            return Ok(test_config.contract_id);
        }

        if let Some(contract_id) = contract_id {
            return self.get_contract_id_from_request_parameters(request, contract_id);
        }

        Ok(self
            .get_contract_id_from_request_context(request)
            .unwrap_or_default())
    }
}

#[tonic::async_trait]
impl KvPair for MongoKvPair {
    async fn get_root(
        &self,
        request: Request<GetRootRequest>,
    ) -> std::result::Result<Response<GetRootResponse>, Status> {
        dbg!(&request);
        let contract_id = self.get_contract_id(&request, &request.get_ref().contract_id)?;
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
        dbg!(&request);
        let contract_id = self.get_contract_id(&request, &request.get_ref().contract_id)?;
        let request = request.into_inner();
        let mut collection = self.new_collection(&contract_id, false).await?;
        let hash: Hash = request.hash.as_slice().try_into()?;
        let record = collection.must_get_merkle_record(0, &hash).await?;
        dbg!(&record);
        collection.update_root_merkle_record(&record).await?;
        Ok(Response::new(SetRootResponse {
            root: record.hash.into(),
        }))
    }

    async fn get_leaf(
        &self,
        request: Request<GetLeafRequest>,
    ) -> std::result::Result<Response<GetLeafResponse>, Status> {
        dbg!(&request);
        let contract_id = self.get_contract_id(&request, &request.get_ref().contract_id)?;
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
        dbg!(&record, &proof);
        let datahash_record = collection.must_get_datahash_record(&record.hash()).await?;
        let node = (record, datahash_record).try_into()?;
        dbg!(&node);
        collection.commit().await.map_err(Error::from)?;
        Ok(Response::new(GetLeafResponse {
            node: Some(node),
            proof,
        }))
    }

    async fn set_leaf(
        &self,
        request: Request<SetLeafRequest>,
    ) -> std::result::Result<Response<SetLeafResponse>, Status> {
        dbg!(&request);
        let contract_id = self.get_contract_id(&request, &request.get_ref().contract_id)?;
        let request = request.into_inner();
        // TODO: Should use session here
        let mut collection = self.new_collection(&contract_id, false).await?;
        let index = request.index;

        let (data, hash, should_insert_into_db): (Vec<u8>, Hash, bool) =
            match (request.data, request.hash) {
                (Some(data), Some(hash)) => (data, hash.try_into()?, true),
                (Some(data), None) => (
                    data.clone(),
                    crate::poseidon::hash(&data)?.try_into().unwrap(),
                    true,
                ),
                (None, Some(hash)) => {
                    let hash = hash
                        .try_into()
                        .map_err(|_e| Status::invalid_argument("Invalid hash"))?;
                    let record = collection
                        .get_datahash_record(&hash)
                        .await?
                        .ok_or(Status::invalid_argument("No data associated to this hash"))?;
                    (record.data, hash, false)
                }
                (None, None) => {
                    return Err(Status::invalid_argument(
                        "Both data and data hash are not provided",
                    ))
                }
            };

        let datahash_record = DataHashRecord {
            hash,
            data: data.clone(),
        };
        if should_insert_into_db {
            collection.insert_datahash_record(&datahash_record).await?;
        }

        let merkle_record = MerkleRecord::new_leaf(index, hash);

        let proof = collection.set_leaf_and_get_proof(&merkle_record).await?;
        let proof = if request.proof_type == ProofType::ProofV0 as i32 {
            Some(Proof {
                proof_type: request.proof_type,
                proof: bincode::serialize(&proof).unwrap(),
            })
        } else {
            None
        };
        dbg!(&merkle_record);
        let node = (merkle_record, datahash_record).try_into()?;
        collection.commit().await.map_err(Error::from)?;
        dbg!(&node);
        Ok(Response::new(SetLeafResponse {
            node: Some(node),
            proof,
        }))
    }

    async fn get_non_leaf(
        &self,
        request: Request<GetNonLeafRequest>,
    ) -> std::result::Result<Response<GetNonLeafResponse>, Status> {
        dbg!(&request);
        let contract_id = self.get_contract_id(&request, &request.get_ref().contract_id)?;
        let request = request.into_inner();
        let mut collection = self.new_collection(&contract_id, false).await?;
        let index = request.index;
        let hash: Hash = request.hash.as_slice().try_into()?;
        let record = collection.must_get_merkle_record(index, &hash).await?;
        dbg!(&record);
        let node = record.try_into()?;
        dbg!(&node);
        Ok(Response::new(GetNonLeafResponse { node: Some(node) }))
    }

    async fn set_non_leaf(
        &self,
        request: Request<SetNonLeafRequest>,
    ) -> std::result::Result<Response<SetNonLeafResponse>, Status> {
        dbg!(&request);
        let contract_id = self.get_contract_id(&request, &request.get_ref().contract_id)?;
        let request = request.into_inner();
        // TODO: Should use session here
        let mut collection = self.new_collection(&contract_id, false).await?;
        let index = request.index;
        let left: Hash = request.left_child_hash.as_slice().try_into()?;
        let right: Hash = request.right_child_hash.as_slice().try_into()?;
        if let Some(hash) = request.hash {
            Hash::validate_children(&hash.as_slice().try_into()?, &left, &right)?;
        }
        let record = collection.insert_non_leaf_node(index, left, right).await?;
        dbg!(&record);
        let node = record.try_into()?;
        dbg!(&node);
        Ok(Response::new(SetNonLeafResponse { node: Some(node) }))
    }

    async fn poseidon_hash(
        &self,
        request: Request<PoseidonHashRequest>,
    ) -> std::result::Result<Response<PoseidonHashResponse>, Status> {
        dbg!(&request);
        let _contract_id = self.get_contract_id(&request, &request.get_ref().contract_id)?;
        let request = request.into_inner();
        // TODO: Should use session here
        let data_to_hash = request.data;
        let hash = crate::poseidon::hash(&data_to_hash)?;
        Ok(Response::new(PoseidonHashResponse { hash: hash.into() }))
    }
}
