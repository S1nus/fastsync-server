use nmt_rs::{
    TmSha2Hasher,
    simple_merkle::proof::Proof
};
use serde::{Deserialize, Serialize};

/// This is a compressed header for usage in backwards sync
/// It's 98.7% smaller than an ExtendedHeader xD
#[derive(Serialize, Deserialize)]
pub struct StrippedHeader {
    pub hash: Vec<u8>,
    pub last_block_id: Vec<u8>,
    pub last_block_id_proof: Proof<TmSha2Hasher>,
    pub data_root: Vec<u8>,
    pub data_root_proof: Proof<TmSha2Hasher>,
}