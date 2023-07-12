use thiserror::Error;
use tonic::Status;

use crate::merkle::MerkleError;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Mongodb error: {0}")]
    Mongodb(#[from] mongodb::error::Error),
    #[error("Merkel tree error: {0:?}")]
    Merkle(#[from] MerkleError),
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
    #[error("Inconsistent data: {0}")]
    InconsistentData(String),
    #[error("Precondition not satisfied: {0}")]
    Precondition(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<Error> for Status {
    fn from(error: Error) -> Self {
        use Error::*;
        let s = format!("{error}");
        match error {
            Mongodb(_) | Merkle(_) | InconsistentData(_) | Precondition(_) => Status::internal(s),
            InvalidArgument(_) => Status::invalid_argument(s),
        }
    }
}


