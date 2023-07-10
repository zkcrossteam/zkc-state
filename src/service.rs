use crate::kvpair::LeafData;
use crate::Error;

use super::kvpair::{bytes_to_bson, ContractId, Hash, MerkleRecord};
use mongodb::bson::doc;
use mongodb::{Client, ClientSession, Collection};
use tonic::{Request, Response, Status};

use super::proto::kv_pair_server::KvPair;
use super::proto::*;

pub struct MongoKvPair {
    client: Client,
}

pub struct MongoCollection<T> {
    client: Client,
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
    ) -> Result<Self, Error> {
        let session = if with_session {
            Some(client.start_session(None).await?)
        } else {
            None
        };
        let database = client.clone().database(Self::get_database_name().as_str());
        let collection_name = Self::get_collection_name(contract_id);
        let collection = database.collection::<T>(collection_name.as_str());
        Ok(Self {
            client,
            collection,
            session,
        })
    }

    pub async fn drop(&self) -> Result<(), Error> {
        let options = mongodb::options::DropCollectionOptions::builder().build();
        self.collection.drop(options).await?;
        Ok(())
    }
}

impl MongoCollection<MerkleRecord> {
    pub async fn get_merkle_record(
        &self,
        index: u32,
        hash: &Hash,
    ) -> Result<Option<MerkleRecord>, Error> {
        let mut filter = doc! {};
        filter.insert("index", index);
        filter.insert("hash", bytes_to_bson(&hash.0));
        let record = self.collection.find_one(filter, None).await?;
        Ok(record)
    }

    pub async fn must_get_merkle_record(
        &self,
        index: u32,
        hash: &Hash,
    ) -> Result<MerkleRecord, Error> {
        let record = self.get_merkle_record(index, hash).await?;
        record.ok_or(Error::Precondition("Merkle record not found".to_string()))
    }

    pub async fn get_root_merkle_record(&self) -> Result<Option<MerkleRecord>, Error> {
        let hash = Hash::default();
        let mut filter = doc! {};
        filter.insert("hash", bytes_to_bson(&hash.0));
        let record = self.collection.find_one(filter, None).await?;
        Ok(record)
    }

    pub async fn must_get_root_merkle_record(&self) -> Result<MerkleRecord, Error> {
        let record = self.get_root_merkle_record().await?;
        record.ok_or(Error::Precondition("Merkle record not found".to_string()))
    }

    /* We always insert new record as there might be uncommitted update to the merkle tree */
    pub async fn update_merkle_record(&self, record: &MerkleRecord) -> Result<MerkleRecord, Error> {
        let mut filter = doc! {};
        filter.insert("index", record.index);
        filter.insert("hash", bytes_to_bson(&record.hash));
        let exists = self.collection.find_one(filter, None).await?;
        exists.map_or(
            {
                self.collection.insert_one(record, None).await?;
                Ok(*record)
            },
            |record| {
                //println!("find existing node, preventing duplicate");
                Ok(record)
            },
        )
    }

    pub async fn update_root_merkle_record(
        &self,
        record: &MerkleRecord,
    ) -> Result<MerkleRecord, Error> {
        assert_eq!(record.hash, Hash::default().0);
        self.update_merkle_record(record).await
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
        let collection = self.new_collection(&contract_id, false).await?;
        let index = request.index;
        let hash: Hash = request.hash.as_slice().try_into()?;
        let record = collection.must_get_merkle_record(index, &hash).await?;
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
        dbg!(&request);
        let request = request.into_inner();
        let contract_id: ContractId = request.contract_id.as_slice().try_into()?;
        // TODO: Should use session here
        let collection = self.new_collection(&contract_id, false).await?;
        let index = request.index;
        let hash: Hash = request.hash.as_slice().try_into()?;
        let leaf_data: LeafData = request.leaf_data.as_slice().try_into()?;
        let record = collection.get_merkle_record(index, &hash).await?;
        let record = match record {
            Some(record) if record.data == leaf_data.0 => record,
            _ => todo!(),
        };
        let node = record.try_into()?;
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
