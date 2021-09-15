# Build instructions
```
cargo build --release --bin rpc_fuzzer
```

# How to use
The fuzzer requires the following parameters:
```
-n ip:port 
IP/port of the target node running the RPC endpoints, default value is 172.28.0.0:18732

-s n
PRNG seed number, default value is 1234567890

-t n
Number of tokio threads, default value is 4. To stress test the node higher values are recommended (>20).

-S path
Path containing JSON files with the shell RPC schemas. The fuzzer parses all files found in this directory.
Default path is ./rpc_shell_schemas

-P path
Path containing JSON files with the protocol RPC schemas. The fuzzer parses all files found in this directory.
Default path is ./rpc_protocol_schemas
```

Example:
```
 ./target/release/rpc_fuzzer -t 50 -n 172.28.0.0:18732
```

# Notes
- HTTP support only. To use HTTPS the fuzzer could piped to a tool like STUNEL(https://www.stunnel.org/)
- The fuzzer doesn't store fuzzcases, but it logs the generated URIs. However, some extra information like request's bodys will be missing.
- Even if the fuzzer takes a PRNG seed, information used to generate requests (for example block hashes) comes from node's responses.

Because of these two last points is not possible to have 100% reproducible cases with the current fuzzer implementation. However this is not needed if fuzzing is performed inside a deterministic execution engine. Another simple option is just to use a traffic sniffer (like wireshark) to dump the HTTP requests contents while fuzzing.



