# Quick instructions
1. Run `./deploy.sh`.
This script will:
- Build the binaries with `cargo`
- Create a Docker network (*fuzznet* 172.18.0.0/16)
- Assign a subnet to the loopback device (172.28.0.0/16)
- Build the *fuzz_target* Docker container
- Block *fuzz_target* from connecting to the Tezos network.
2. Start the *fuzz_target* container: `./scripts/run_fuzz_target`
3. Start the RPC/P2P fuzzers: `./scripts/run_fuzzers`

# RPC fuzzer
## Example use
```
# cargo build --release --bin rpc_fuzzer
# ./target/release/rpc_fuzzer -t 50 -n 172.18.0.101:18732
```

## Parameters
- `-n ip:port`: Target node's address, default is *172.18.0.101:18732*.
- `-s seed`: PRNG seed number, default is *1234567890*.
- `-t workers`: Number of tokio workers, default is *20*.
- `-S shell_schema_path`: Path containing JSON files for the shell RPC schemas. Default path is *./rpc_shell_schemas*.
- `-P protocol_schema_path`: Path containing JSON files for the protocol RPC schemas. Default path is *./rpc_protocol_schemas*.

## Limitations
1. RPC fuzzer supports HTTP **only**, to use HTTPS the fuzzer could piped through some tool like STUNEL(https://www.stunnel.org/).
2. The fuzzer doesn't store fuzzcases, but it logs the generated URIs. Extra information like the requests' bodies will be missing.
3. Even when the fuzzer takes a PRNG seed, external information is used to generate requests (for example block hashes from node's responses).

Because of #2 and #3, it is not possible to have deterministic fuzzcase reproduction. However, this is not required if we run the fuzzer and/or the target inside a record/replay (virtnyl) engine. A simpler alternative is to use a traffic sniffer (wireshark) to save HTTP requests.

# P2P fuzzer
## Example use
```
# cargo build --release --bin p2p_fuzzer
# ./target/release/p2p_fuzzer -c p2p_fuzzer_default.conf -i identities.json 
```

## Parameters
- `-c configuration_file`: JSON file containing the fuzzer configuration.
- `-i identities_file`: JSON file containing pregenerated node identities. These can be generated with `identity_tool`.

## Configuration file
The configuration file allows to tune most aspects of the fuzzer behavior.

The general options are:
- `peer`: The IP/port of the target node.
- `prng_seed`: An interger that will be used to seed the fuzzer PRNG.
- `max_chunk_size`: An integer with the maximum chunk size in bytes (protocol allows up to 64kB, minus 2 bytes to encode the chunk size).
- `threads`: Number of `tokio` workers used by the fuzzer. A good default is to use the number of peer connections accepted by the target node.

The rest of the configuration allows to tune handshake and peer-mesage specific options. Both of them start with a `chunk_options` field to control fuzzer behaviour at the chunking layer.

### Chunk fuzzing options
The following are the sub-fields corresponding to a `chunk_options` field: 
- `flip_bits`: Float that indicates the probability of performing bit flipping over the final chunk.
- `split_chunks`: Float that indicates the probability of randomizing chunk sizes, otherwise optimum chunking will be performed.
- `split_bytes`: Float that indicates the probability of randomizing the size used in the final stream write, otherwise optimum size will be used. This option is moslty useful when used in conjuction with `write_bytes_sleep` and `write_bytes_sleep_ms`.
- `incomplete_send`: Float that indicates the probability of randomly interrupting the data transmission.
- `write_bytes_sleep`: When bytes are split (`split_bytes`) in multiple slices, this option contains a float that indicates the probability of sleeping (for `write_bytes_sleep_ms` milliseconds) during a slice write.
- `write_bytes_sleep_ms`: Integer that indicates the number of milliseconds to sleep.

### Handshake fuzzing options
The following are the sub-fields corresponding to the `handshake_fuzzing` field:
- `chunk_options`: Contains the chunk fuzzing options used during handshaking.
- `abort_before_connect`: Float that indicates the probability of randomly disconnecting before sending the P2P *connect* message.
- `connect_sleep`: Float that indicates the probability of randomly sleeping between the TCP connect and the P2P *connect* message.
- `connect_sleep_ms`: Integer that indicates the number of milliseconds to sleep between the TCP connect and the P2P *connect* message.
- `connect_msg_sleep`: Float that indicates the probability of randomly sleeping after the P2P *connect* message.
- `connect_msg_sleep_ms`: Integer that indicates the number of milliseconds to sleep after the P2P *connect* message.
- `fuzz_public_key`: Float that indicates the probability of randomizing the public key.
- `fuzz_pow`: Float that indicates the probability of randomizing the Proof of Work.
- `fuzz_network_version`: Float that indicates the probability of randomizing the network version.
- `abort_before_metadata`: Float that indicates the probability of randomly disconnecting before sending the P2P *metadata* message.
- `metadata_msg_sleep`: Float that indicates the probability of randomly sleeping after the P2P *metadata* message.
- `metadata_msg_sleep_ms`: Integer that indicates the number of milliseconds to sleep after the P2P *metadata* message. 
- `recv_metadata`: Float that indicates the probability of waiting for the *metadata* message from the peer.
- `fuzz_metadata`: Float that indicates the probability of randomizing the P2P *metadata* message.
- `abort_before_ack`: Float that indicates the probability of randomly disconnecting before sending the P2P *ack* message.
- `ack_msg_sleep`: Float that indicates the probability of randomly sleeping after the P2P *ack* message.
- `ack_msg_sleep_ms`: Integer that indicates the number of milliseconds to sleep after the P2P *ack* message.
- `fuzz_ack`: Float that indicates the probability of randomizing the P2P *ack* message. 
- `recv_ack`: Float that indicates the probability of waiting for the *ack* message from the peer.

### Peer Message fuzzing options
The following are the sub-fields corresponding to the `peer_message_fuzzing` field:
- `chunk_options`: Contains the chunk fuzzing options used during peer messages.
- `messages_per_chunk`: Integer that indicates how many *peer messages* should try to fit into the same chunk. We usually want to set this value to *1* since the P2P protocol doesn't allow multiple messages into a signle chunk.
- `push_messages`: A list of messages types that the fuzzer can randomly generate and send to the peer. Any combination of the possible values can be supplied: `Disconnect`, `Bootstrap`, `Advertise`, `SwapRequest`, `SwapAck`, `GetCurrentBranch`, `CurrentBranch`, `Deactivate`, `GetCurrentHead`, `CurrentHead`, `GetBlockHeaders`, `BlockHeader`, `GetOperations`, `Operation`, `GetProtocols`, `Protocol`, `GetOperationsForBlocks`, `OperationsForBlocks`.
- `max_push_throughput`: Integer that indicates the maximum number of messages that can be sent in burst to the peer (before waiting for a response).
- `reply_options`: Contains sub-fields to control the fuzzer behaviour when receving messages from the peer.

#### Reply options
- `bootstrap`: Float that indicates the probability of replying (with a randomized `Advertise` message) after receving a `Bootstrap` message.
- `get_current_head`: Float that indicates the probability of replying (with a randomized `CurrentHead` message) after receving a `GetCurrentHead` message.
- `get_current_branch`: Float that indicates the probability of replying (with a randomized `CurrentBranch` message) after receving a `GetCurrentBranch` message.
- `get_block_headers`: Float that indicates the probability of replying (with cached `BlockHeader` messages) after receving a `GetBlockHeaders` message. **WARNING**: the blocks list is currently disabled so this option doesn't have any effect.
- `get_operations_for_blocks`: Float that indicates the probability of replying (with multiple randomized `OperationsForBlocks` messages) after receving a `GetOperationsForBlocks` message.

## Identities file
The identities file is a JSON file with a collection of pre-generated peer identities used in the P2P *connection* message during the handshake process. The fuzzer will load the identities so fuzzer's workers can pick random identities for each connection.  

We have previously set a subnet (172.28.0.0/16) to the loopback device, so the fuzzer can also use random IP addresses in this subnet range. The combination of random IPs and random identities makes it possible to simulate multiple peers from a single fuzzer.

Identities can eb generated with the `identity_tool`. For example to generate 10000 identities we can run:
```
# cargo build --release --bin identity_tool
# ./target/release/identity_tool -n 10000 > identities.json
```
This can take a considerable amount of time depending of the processing resources of the machine runnig the tool.

A pre-generated `identities.json` file is alread included in the repository.

## Limitations
Similar limitations to the RPC fuzzer regarding fuzzcases and external feedback, that prevent deterministic bug reproduction. The solutions are similar too, but in case of using a sniffer extra work must be required for the crypto-layer since nonces might change. 

# Coverage
- LCOV reports are periodically generated in the *fuzz_target* and can be navigated at http://172.18.0.101:8000/
- Coverage statistics over time can be obtained in JSON format from http://172.18.0.101:8000/history.json
- From this JSON information a chart is generated at http://172.18.0.101:8000/charts/
