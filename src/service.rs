use std::borrow::Borrow;

use crate::kvpair::LeafData;
use crate::Error;

use super::kvpair::{bytes_to_bson, ContractId, Hash, MerkleRecord};
use mongodb::bson::{doc, Document};
use mongodb::error::{TRANSIENT_TRANSACTION_ERROR, UNKNOWN_TRANSACTION_COMMIT_RESULT};
use mongodb::options::{
    Acknowledgment, FindOneOptions, InsertOneOptions, ReadConcern, ReplaceOptions,
    TransactionOptions, WriteConcern,
};
use mongodb::results::{InsertOneResult, UpdateResult};
use mongodb::{Client, ClientSession, Collection};
use tonic::{Request, Response, Status};

use super::proto::kv_pair_server::KvPair;
use super::proto::*;

pub struct MongoKvPair {
    client: Client,
}

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
        if let Some(session) = self.session.as_mut() {
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

    pub async fn get_merkle_record(
        &mut self,
        index: u32,
        hash: &Hash,
    ) -> Result<Option<MerkleRecord>, Error> {
        let mut filter = doc! {};
        filter.insert("index", index);
        filter.insert("hash", bytes_to_bson(&hash.0));
        let record = self.find_one(filter, None).await?;
        Ok(record)
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
        let hash = Hash::default();
        let mut filter = doc! {};
        filter.insert("hash", bytes_to_bson(&hash.0));
        let record = self.find_one(filter, None).await?;
        Ok(record)
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
        filter.insert("hash", bytes_to_bson(&record.hash));
        let exists = self.find_one(filter, None).await?;
        exists.map_or(
            {
                self.insert_one(record, None).await?;
                Ok(*record)
            },
            |record| {
                //println!("find existing node, preventing duplicate");
                Ok(record)
            },
        )
    }

    pub async fn update_root_merkle_record(
        &mut self,
        record: &MerkleRecord,
    ) -> Result<MerkleRecord, Error> {
        assert_eq!(record.hash, Hash::default().0);
        let mut filter = doc! {};
        filter.insert("hash", bytes_to_bson(&record.hash));
        let result = self.replace_one(filter, record, None).await?;
        dbg!(&result);
        Ok(*record)
    }
}

impl MongoKvPair {
    pub fn new(client: Client) -> Self {
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

#[tonic::async_trait]
impl KvPair for MongoKvPair {
    async fn get_root(
        &self,
        request: Request<GetRootRequest>,
    ) -> std::result::Result<Response<GetRootResponse>, Status> {
        dbg!(&request);
        let request = request.into_inner();
        let contract_id: ContractId = request.contract_id.as_slice().try_into()?;
        Ok(Response::new(GetRootResponse {
            contract_id: contract_id.into(),
            root: [0u8; 32].into(),
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
        let request = request.into_inner();
        let contract_id: ContractId = request.contract_id.as_slice().try_into()?;
        let mut collection = self.new_collection(&contract_id, false).await?;
        let index = request.index;
        let hash: Hash = request.hash.as_slice().try_into()?;
        let record = collection.must_get_merkle_record(index, &hash).await?;
        let node = record.try_into()?;
        collection.commit().await.map_err(Error::from)?;
        dbg!(&record, &node);
        Ok(Response::new(GetLeafResponse {
            node: Some(node),
            proof: None,
        }))
    }

    async fn set_leaf(
        &self,
        request: Request<SetLeafRequest>,
    ) -> std::result::Result<Response<GetLeafResponse>, Status> {
        dbg!(&request);
        let request = request.into_inner();
        let contract_id: ContractId = request.contract_id.as_slice().try_into()?;
        // TODO: Should use session here
        let mut collection = self.new_collection(&contract_id, false).await?;
        let index = request.index;
        let hash: Hash = request.hash.as_slice().try_into()?;
        let leaf_data: LeafData = request.leaf_data.as_slice().try_into()?;
        let record = collection.get_merkle_record(index, &hash).await?;
        let record = match record {
            Some(record) if record.data == leaf_data.0 => record,
            _ => todo!(),
        };
        let node = record.try_into()?;
        collection.commit().await.map_err(Error::from)?;
        dbg!(&record, &node);
        Ok(Response::new(GetLeafResponse {
            node: Some(node),
            proof: None,
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
