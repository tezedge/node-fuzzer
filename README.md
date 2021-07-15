
# Build instructions
```
cargo build --release --bin new-node
```

# How to use
It is recommended to run the nodes inside a docker container limiting the amount of memory (for example passing `-m="8g"`).
The fuzzer sets the max number of connections to 1 to avoid connecting to anyone that is not the target, it is recommended to set the same limit for the target node (`--peer-thresh-high=1`). Ideally on should run it in an isolated network.

1. Run the target node, for example:
```
./run.sh release --network=mainnet --disable-bootstrap-lookup --synchronization-thresh 0 --peer-thresh-low=0 --peer-thresh-high=1
```
2. Once the target node is listening for connections run the fuzzer:
```
./target/release/new-node -c "configuration file"
```
3. Configuration file is a JSON file that gets deserialized in the following structure:
```
#[derive(Clone, Serialize, Deserialize)]
struct Credentials {
    peer_id: String,
    public_key: String,
    private_key: String,
    pow: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Profile {
    id: Credentials,
    peer: String,
    prng_seed: u64,
    push_messages: Vec<String>, // send messages to target regardless of requests
    push_throughput: u64,       // number of messages send in burst
    blocks_limit: usize,          // max block-header vector elements
    reply_to_get_current_branch: bool, // reply to GetCurrentBranch requests
    reply_to_get_block_headers: bool, // reply to GetBlockHeaders requests
    reply_to_get_operations_for_blocks: bool, // reply to GetOperationsForBlocks requests
}
```
4. If **no configuration file** is provided the following default configuration will be used:
```
{
    "id": {
        "peer_id": "idrdoT9g6YwELhUQyshCcHwAzBS9zA",
        "public_key": "94498d9416140fbc458495333daac1b4c87e419f5726717a54f9b6c67476ae1c",
        "private_key": "ac7acf3afed7637be10f8fc76a2eb6b3359c78adb1d813b41cbab3fae954f4b1",
        "pow": "bbc2300149249e1ccc84a54362236c3cbbc2cc2ffbd3b6ea"
    },
    "peer": "127.0.0.1:9732",
    "prng_seed": 1234567890,
    "push_messages": [
        "Advertise",
        "SwapRequest",
        "SwapAck",
        "GetCurrentBranch",
        "CurrentBranch",
        "Deactivate",
        "GetCurrentHead",
        "CurrentHead",
        "GetBlockHeaders",
        "BlockHeader",
        "GetOperations",
        "Operation",
        "GetProtocols",
        "Protocol",
        "GetOperationsForBlocks",
        "OperationsForBlocks"    
    ],
    "push_throughput": 65536,
    "blocks_limit": 500,
    "reply_to_get_current_branch": true,
    "reply_to_get_block_headers": true,
    "reply_to_get_operations_for_blocks": true
}
```
This default configuration will randomly push all the messages listed in `push_messages` in bursts of 65536 messages.
It will also generate random replies for remote peer's requests: `GetCurrentBranch`, `GetBlockHeaders`, `GetOperationsForBlocks`.

5. Some example configuration files are provided:
- Same configuration as default but a zero sized block map, so new randomly generated block headers are not stored: `./target/release/new-node -c default_no_blockmap.conf`.
- Configuration to send only `Advertise` messages: `./target/release/new-node -c advertise_flood.conf`.
- Configuration to send only `CurrentBranch` messages: `./target/release/new-node -c current_branch_flood.conf`.






