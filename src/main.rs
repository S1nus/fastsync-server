use std::io::Read;
//use tokio::main;
use std::{
    sync::Arc,
    fs,
};
use celestia_types::{
    ExtendedDataSquare, ExtendedHeader,
};
use celestia_rpc::{
    HeaderClient,
    ShareClient,
    Client,
};
use celestia_tendermint_proto::Protobuf;
use celestia_tendermint_proto::v0_37::{
    types::BlockId as RawBlockId,
    version::Consensus as RawConsensusVersion,
};
use rocksdb::{
    TransactionDB,
    MultiThreaded,
    Options,
};
use dirs::home_dir;
use bincode;

mod stripped_header;
use stripped_header::StrippedHeader;

use nmt_rs::{
    simple_merkle::{
        db::MemDb,
        tree::MerkleTree,
    },
    TmSha2Hasher,
};

#[tokio::main]
async fn main() {

    let token = std::env::var("CELESTIA_NODE_AUTH_TOKEN").expect("Token not provided");
    let client = Arc::new(Client::new("ws://localhost:26658", Some(&token))
        .await
        .expect("Failed creating rpc client"));

    let home = home_dir().expect("Could not find home directory");
    let subdir = home.join(".tia_sync_serv");
    if !subdir.exists() {
        fs::create_dir(&subdir).expect("Could not create subdirectory");
        println!("Subdirectory created: {:?}", subdir);
    } else {
        println!("Subdirectory already exists: {:?}", subdir);
    }

    let db_path = subdir.join("db");
    let db: Arc<TransactionDB::<MultiThreaded>> = Arc::new(TransactionDB::open_default(&db_path).expect("could not open database"));
    let _ = TransactionDB::<MultiThreaded>::destroy(&Options::default(), &db_path);
    let highest_saved_head = db.get(b"highest_saved_head")
        .expect("could not get highest_saved_head");    

    if highest_saved_head.is_none() {
        populate_init(client.clone(), db.clone()).await;
    }

    let catchup_worker = tokio::spawn(catchup_worker(client.clone(), db.clone()));
    let sync_worker = tokio::spawn(sync_worker(client.clone(), db.clone()));
    tokio::try_join!(catchup_worker, sync_worker).expect("could not join workers");
    //sync_worker.await.unwrap();


    // 1. Get network head
    // 2. Get highest locally-saved height
    // 3. Calculate starting height = network head - git remote add origin https://github.com/S1nus/fastsync-server.git80640 (about 2 weeks of blocks)
}

async fn populate_init(client: Arc<Client>, db: Arc<TransactionDB<MultiThreaded>>) {
    let head = client.header_network_head()
        .await
        .expect("could not get latest header");
    let starting_height = head.height().value() - 80640;
    let starting_header = client.header_get_by_height(starting_height)
        .await
        .expect("could not get starting header");
    let starting_eds = client.share_get_eds(&starting_header)
        .await
        .expect("could not get starting EDS");
    let txn = db.transaction();
    txn.put(b"lowest_saved_head", starting_height.to_le_bytes())
        .expect("could not put lowest_saved_head into db");
    txn.put(b"highest_saved_head", head.height().value().to_le_bytes())
        .expect("could not put highest_saved_head into db");
    txn.put(&starting_height.to_le_bytes(), eds_to_bytes(&starting_eds))
        .expect("could not put starting EDS into db");
    txn.commit()
        .expect("could not commit transaction");
}

async fn sync_worker(client: Arc<Client>, db: Arc<TransactionDB<MultiThreaded>>) {
    let sub = &mut client.header_subscribe()
        .await
        .expect("couldn't subscribe");
    loop {
        if let Some(Ok(header)) = sub.next().await {
            println!("Received header with height: {}", header.height());
            println!("stripping header...");
            let stripped_header = strip_header(&header);
            println!("getting the EDS...");
            let eds = client.share_get_eds(&header)
                .await
                .expect("couldn't get eds");
            let lowest_saved = db.get(b"lowest_saved_head")
                .expect("could not get lowest_saved_head")
                .unwrap();
            let txn = db.transaction();
            txn.delete([b"eds".to_vec(), lowest_saved.clone()].concat())
                .expect("could not cleanup eds");
            txn.delete([b"stripped_header".to_vec(), lowest_saved.clone()].concat())
                .expect("could not cleanup stripped header");
            // Increment lowest_saved_head
            txn.put(b"lowest_saved_head", (header.height().value()+1).to_le_bytes())
                .expect("could not update lowest_saved_head");
            // save EDS
            txn.put(get_eds_key(header.height().value()), eds_to_bytes(&eds))
                .expect("could not put EDS into db");
            // save stripped header
            txn.put(get_stripped_header_key(header.height().value()), bincode::serialize(&stripped_header)
                .expect("could not serialize stripped header")
            )
                .expect("could not put stripped header into db");
            // update highest_saved_head
            txn.put(b"highest_saved_head", header.height().value().to_le_bytes())
                .expect("could not put highest_saved_head into db");
            txn.commit()
                .expect("could not commit transaction");
        }
        else {
            println!("No more headers to receive, or problem encountered. Exiting...");
            return;
        }
    }
}

async fn catchup_worker(client: Arc<Client>, db: Arc<TransactionDB<MultiThreaded>>) {
    let mut network_head = client.header_network_head()
        .await
        .expect("could not get latest header")
        .height().value();
    let highest_synced = db.get(b"highest_saved_head")
        .expect("could not get highest_saved_head")
        .unwrap();
    let mut buf = [0; 8];
    buf.copy_from_slice(&highest_synced);
    let mut highest_synced_height = u64::from_le_bytes(buf);
    while highest_synced_height < network_head {
        let next_height = highest_synced_height + 1;
        println!("syncing {}", next_height);
        let next_header = client.header_get_by_height(next_height)
            .await
            .expect("could not get next header");
        let stripped_header = strip_header(&next_header);
        let eds = client.share_get_eds(&next_header)
            .await
            .expect("could not get EDS");
        let txn = db.transaction();
        txn.delete([b"eds".to_vec(), highest_synced.clone()].concat())
            .expect("could not cleanup eds");
        txn.delete([b"stripped_header".to_vec(), highest_synced.clone()].concat())
            .expect("could not cleanup stripped header");
        txn.put(get_eds_key(next_height), eds_to_bytes(&eds))
            .expect("could not put EDS into db");
        txn.put(get_stripped_header_key(next_height), bincode::serialize(&stripped_header)
            .expect("could not serialize stripped header")
        )
            .expect("could not put stripped header into db");
        txn.put(b"highest_saved_head", next_height.to_le_bytes())
            .expect("could not update highest_saved_head");
        txn.commit()
            .expect("could not commit transaction");
        highest_synced_height = next_height;
        network_head = client.header_network_head()
            .await
            .expect("could not get latest header")
            .height().value();
    }
}

fn get_stripped_header_key(height: u64) -> Vec<u8> {
    [b"stripped_header".to_vec(), height.to_le_bytes().to_vec()].concat()
}

fn get_eds_key(height: u64) -> Vec<u8> {
    [b"eds".to_vec(), height.to_le_bytes().to_vec()].concat()
}

fn eds_to_bytes(eds: &ExtendedDataSquare) -> Vec<u8> {
    bincode::serialize(&eds).expect("could not serialize EDS")
}

fn bytes_to_eds(bytes: &[u8]) -> ExtendedDataSquare {
    bincode::deserialize(bytes).expect("could not deserialize EDS")
}

fn strip_header(header: &ExtendedHeader) -> StrippedHeader {
    let hasher = TmSha2Hasher {};
    let mut tree: MerkleTree<MemDb<[u8; 32]>, TmSha2Hasher> = MerkleTree::with_hasher(hasher);
    let fields_bytes = vec![
            Protobuf::<RawConsensusVersion>::encode_vec(&header.header.version).unwrap(),
            header.header.chain_id.encode_vec().unwrap(),
            header.header.height.encode_vec().unwrap(),
            header.header.time.encode_vec().unwrap(),
            Protobuf::<RawBlockId>::encode_vec(&header.header.last_block_id.unwrap_or_default()).unwrap(),
            header.header.last_commit_hash.encode_vec().unwrap(),
            header.header.data_hash.encode_vec().unwrap(),
            header.header.validators_hash.encode_vec().unwrap(),
            header.header.next_validators_hash.encode_vec().unwrap(),
            header.header.consensus_hash.encode_vec().unwrap(),
            header.header.app_hash.encode_vec().unwrap(),
            header.header.last_results_hash.encode_vec().unwrap(),
            header.header.evidence_hash.encode_vec().unwrap(),
            header.header.proposer_address.encode_vec().unwrap(),
        ];
    fields_bytes.iter().for_each(|l| {
        tree.push_raw_leaf(l);
    });
    assert_eq!(tree.root(), header.header.hash().as_bytes());
    /* TODO: fix this, return a Result instead of assert and panic
    if tree.root() != header.header.hash().as_bytes() {
        Err()
    }*/
    let (last_block_id, last_block_id_proof) = tree.get_index_with_proof(4);
    let (data_root, data_root_proof) = tree.get_index_with_proof(6);

    StrippedHeader {
        hash: header.header.hash().as_bytes().to_vec(),
        last_block_id,
        last_block_id_proof,
        data_root,
        data_root_proof,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use serde_json;

    #[test]
    fn test_strip_header() {
        let header_file = std::fs::File::open("header3.json").unwrap();
        let h: Option<ExtendedHeader> = serde_json::from_reader(header_file).unwrap();
        let stripped: StrippedHeader = match h {
            Some(header) => {
                let header_bytes = bincode::serialize(&header).unwrap();
                println!("full header size: {}", header_bytes.len());
                strip_header(&header)
            },
            None => panic!("no header"),
        };
        let stripped_bytes = bincode::serialize(&stripped).unwrap();
        println!("stripped header size: {}", stripped_bytes.len());
    }
}