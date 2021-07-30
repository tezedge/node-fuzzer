
# Build instructions
```
cargo build --release --bin badnode
```

# How to use
It is recommended to run both the target, and the fuzzer inside docker containers in an isolated network.

1. Create an internal network for the containers:
```
docker network create --d bridge --subnet=172.28.0.0/16 --ip-range=172.28.5.0/24 --gateway=172.28.5.254 --internal fuzznet
```

2. Run the target node in a container attached to that network. For example if the target is Octez:
```
docker run -m="8g" --network fuzznet -ti 4a30177671d1 tezos-node --no-bootstrap-peers
```
In this case we also limited the node memory with the `-m="8g"` option.

3. If the target is TezEdge pass the following options to `light-node`:
```
--network=mainnet --disable-bootstrap-lookup --synchronization-thresh 0 --peer-thresh-low=0
```

4. Many of the fuzzer options (especially handshake fuzzing) might cause the node to quickly blacklist the fuzzer. This can be avoided by passing `--disable-peer-blacklist` to the node.
**TODO**: figure out if there is a way to do this in Octez. 


5. Once the target node is listening for connections run the fuzzer (from inside a container attached to the same network):
```
./target/release/badnode -c "configuration file" -i "identities file"
```
If no `-c` or `-i` flags are specified the fuzzer uses a default configuration.

**NOTE**: the configuration file must include the correct IP address (docker internal network) of the container running the node.

# Identities
The identities file can be generated with `identity_tool`.
```
cargo build --release --bin identity_tool
./target/release/identity_tool -n "number of identities" > identities.json
```
There is already an `identities.json` file in this repository with 10K pre-generated peer identities.

The fuzzer will pick a random identity from the set on each connection. If no identities file is provided the fuzzer will use one default identity:
```
[{
    "peer_id":"idt3LoYeur2FXAvRXsvmW2Ac9EpEHb",
    "public_key":"f0c7f2fdf2e6effb32fb6d750051dc63512b5bd8d16ac7aa110f4f4fc840552e",
    "proof_of_work_stamp":"d5a8e2140d85615201c0e08b1bd22780b8d766af9be06123",
    "secret_key":"051a6c62253e37577de1031fee4e584487494d6da8186dba608a894ba2834a40"
}]
```

# Configuration file
The configuration file allows to tune most fuzzer behavior.

If no configuration file is provided the fuzzer uses the following default configuration:

```
{
    "peer": "172.28.5.0:9732",
    "prng_seed": 1234567890,
    "max_chunk_size": 65519,
    "threads": 1,
    "handshake_fuzzing": null,
    "peer_message_fuzzing": {
        "chunk_options": {
            "split_chunks": true,
            "split_bytes": false,
            "incomplete_send": false,
            "max_write_bytes_sleep": 0
        },
        "messages_per_chunk": 1,
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
        "block_header_limit": 0,
        "reply_options": null
    }
}
```

Configuration options are the following:
- peer: IP address and port of the target node
- prng_seed: `u64` number that is used to seed the fuzzer.
- max_chunk_size: `usize` with the maximum size the fuzzer is allowed to use when sending chunks.
- threads: number of parallel connections. To prevent locking each `async` "thread" initializes its own PRNG object with `prng_seed + thread#`.
- handshake_fuzzing: optional handshake fuzzing options. If this value is `null` the fuzzer will always perform valid handshakes.
- peer_message_fuzzing: optional peer message fuzzing options. If this value is `null` the fuzzer won't send peer messages.

## Chunk options
Both handshake and peer message options start with an `chunk_options` field. This field allows to configure the chunking behavior of messages:
- split_chunks: `bool` if enabled split messages into random sized chunks.
- split_bytes: `bool` if enabled split a chunk into random sized sets of bytes.
- incomplete_send: `bool` this option only makes sense if `split_bytes` is enabled. If enabled it will randomly disconnect from the peer in the middle of sending one of the bytes sets.
- max_write_bytes_sleep: `u64` this option only makes sense if `split_bytes` is enabled. When sending bytes it will randomly pick a milliseconds value in the range of 0 to `max_write_bytes_sleep` and sleep.
 

## Handshake fuzzing options
- chunk_options
- abort_before_connect: `bool` if enabled the fuzzer will randomly disconnect from the target before sending a connect message.
- max_connext_sleep: `u64` random sleep time in milliseconds between the TCP connection is established and sending the connect message.
- max_connect_msg_sleep: `u64` random sleep time in milliseconds after sending a connect message.
- fuzz_public_key: `bool` if enabled the public key of the connect message will be randomized.
- fuzz_pow: `bool` if enabled the Proof of Work of the connect message will be randomized.
- fuzz_network_version: `bool` if enabled the protocol version of the connect message will be randomized.
- abort_before_metadata: `bool` if enabled the fuzzer will randomly disconnect before sending a metadata message.
- max_metadata_msg_sleep: `u64` random sleep time in milliseconds after sending metadata message.
- always_recv_metadata: `bool` if enabled the fuzzer will always try to read a metadata message sent by the target.
- fuzz_metadata: `bool` if enabled the fuzzer will randomize the metadata message.
- abort_before_ack: `bool` if enabled the fuzzer will randomly disconnect before sending an ACK message.
- max_ack_msg_sleep: `u64`  random sleep time in milliseconds after sending an ACK message.
- ack_replies: a list of strings with possible ACK replies that can be sent by the fuzzer. Possible values are: `Ack`, `NackV0`, `Nack`.
- always_recv_ack: `bool` if enabled the fuzzer will always try to read an ACK message sent by the target.

## Peer message fuzzing options
- chunk_options
- messages_per_chunk: `u64` if this value is more than 1 the fuzzer will try to put up to `messages_per_chunk` messages in the same chunk.
- push_messages: a list of strings with possible messages that can be sent by the fuzzer. Possible values are: `Advertise`, `SwapRequest`, `SwapAck`, `GetCurrentBranch`, `CurrentBranch`, `Deactivate`, `GetCurrentHead`, `CurrentHead`, `GetBlockHeaders`, `BlockHeader`, `GetOperations`, `Operation`, `GetProtocols`, `Protocol`, `GetOperationsForBlocks` `OperationsForBlocks`.
- push_throughput: `u64` number of peer messages that the fuzzer will sent in bulk before receiving messages from the target.
- block_header_limit: `usize` the fuzzer can keep a `HashMap` of generated block headers that can be sent if the target requests a block header hash. This map can only hold up to `block_header_limit` elements.
- reply_options: the fuzzer can optionally take actions depending of messages received from the target. If this value is `null` the fuzzer will ignore incoming messages.

### Reply options
- get_current_branch: `bool` if enabled the fuzzer will generate random replies to incoming `GetCurrenBranch` requests.
- get_block_headers: `bool` if enabled the fuzzer will generate random replies to incoming `GetBlockHeaders` requests.
- get_operations_for_blocks: `bool` if enabled the fuzzer will generate random replies to incoming `GetOperationsForBlocks` requests.

### Example configurations
Some example configurations can be found in this repo:
- peermsg_no_blockmap.conf
- peermsg_no_blockmap_incomplete.conf
- nack_flood.conf
- handshake_fuzz.conf
- current_branch_flood.conf
- advertise_flood.conf