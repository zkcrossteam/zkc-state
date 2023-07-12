use super::kvpair::{bytes_to_bson, ContractId, Hash, MerkleRecord};
use mongodb::bson::doc;
use mongodb::Client;
use tonic::{Request, Response, Status};

use super::proto::kv_pair_server::KvPair;
use super::proto::*;

pub struct MongoKvPair {
    client: Client,
}

impl MongoKvPair {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
    fn get_collection_name(&self, contract_id: &ContractId) -> String {
        format!("MERKLEDATA_{}", hex::encode(contract_id.0))
    }

    fn get_database_name(&self) -> String {
        "zkwasmkvpair".to_string()
    }

    pub async fn get_collection<T>(
        &self,
        contract_id: &ContractId,
    ) -> Result<mongodb::Collection<T>, mongodb::error::Error> {
        let database = self
            .client
            .clone()
            .database(self.get_database_name().as_str());
        let collection_name = self.get_collection_name(contract_id);
        Ok(database.collection::<T>(collection_name.as_str()))
    }

    pub async fn drop_collection<T>(
        &self,
        contract_id: &ContractId,
    ) -> Result<(), mongodb::error::Error> {
        let collection = self.get_collection::<T>(contract_id).await?;
        let options = mongodb::options::DropCollectionOptions::builder().build();
        collection.drop(options).await
    }

    pub async fn get_merkle_record(
        &self,
        contract_id: &ContractId,
        index: u32,
        hash: &Hash,
    ) -> Result<Option<MerkleRecord>, mongodb::error::Error> {
        let collection = self.get_collection::<MerkleRecord>(contract_id).await?;
        let mut filter = doc! {};
        filter.insert("index", index);
        filter.insert("hash", bytes_to_bson(&hash.0));
        collection.find_one(filter, None).await
    }

    /* We always insert new record as there might be uncommitted update to the merkle tree */
    pub async fn update_merkle_record(
        &self,
        contract_id: &ContractId,
        record: &MerkleRecord,
    ) -> Result<MerkleRecord, mongodb::error::Error> {
        let collection = self.get_collection::<MerkleRecord>(contract_id).await?;
        let mut filter = doc! {};
        filter.insert("index", record.index);
        filter.insert("hash", bytes_to_bson(&record.hash));
        let exists = collection.find_one(filter, None).await?;
        exists.map_or(
            {
                collection.insert_one(record, None).await?;
                Ok(*record)
            },
            |record| {
                //println!("find existing node, preventing duplicate");
                Ok(record)
            },
        )
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
        let index = request.index;
        let hash: Hash = request.hash.as_slice().try_into()?;
        let record = self
            .get_merkle_record(&contract_id, index, &hash)
            .await
            .map_err(|e| Status::internal(format!("Query mongodb failed: {}", e)))
            .and_then(|r| {
                r.ok_or(Status::not_found(format!(
                    "Leaf with index {} and hash 0x{} not found",
                    index,
                    hex::encode(&hash.0)
                )))
            })?;
        let node = record.try_into()?;
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
        dbg!(request);
        unimplemented!()
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
