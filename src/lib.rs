pub mod errors;
mod kvpair;
mod merkle;
mod poseidon;
pub mod service;

pub mod proto {
    pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("kvpair_descriptor");
    tonic::include_proto!("kvpair");
}

use errors::*;
