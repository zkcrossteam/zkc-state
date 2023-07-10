use thiserror::Error;
use tonic::Status;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Mongodb error: {0}")]
    Mongodb(mongodb::error::Error),
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
    #[error("Inconsistent data: {0}")]
    InconsistentData(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<Error> for Status {
    fn from(error: Error) -> Self {
        match error {
            Error::Mongodb(e) => Status::internal(format!("Db error: {}", e)),
            Error::InvalidArgument(s) => Status::invalid_argument(s),
            Error::InconsistentData(s) => Status::internal(s),
        }
    }
}
