# FastSync Server
The backend for a sampling light client for Celestia. Syncs headers, strips them, and saves their EDS.
Light nodes will be able to request the last 2 weeks of stripped headers + 16 samples and merkle proofs for each block, as a single gzip'd payload.
Light nodes will then be able to download, extract, and verify without any back-and-forth on the p2p network.
This is made possible by [recursive-sync](https://github.com/S1nus/celestia-recursive-sync)

## Todo list:
- [X] RocksDB
  - datastore for headers and EDS
- [X] Catchup worker
  - catches up from the last two weeks to the head
- [X] Sync worker
  - follows the head of the chain
- [ ] Serve compressed sync payloads
