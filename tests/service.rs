use zkc_state_manager::kvpair::ContractId;
use zkc_state_manager::kvpair::MerkleRecord;
use zkc_state_manager::service::MongoKvPair;

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
}
