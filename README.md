# Rust Storage Task

This fork of **TezEdge** contains two implementations for key-value store
used in [MerkleStorage](storage/src/merkle_storage.rs):

  1. [In-memory]: which uses `BTreeMap` from rust's standard library. For
    recovering existing state it uses: [ContextActionStorage](storage/src/context_action_storage.rs) in order
    to reapply all actions that would mutate `MerkleStorage`.
  1. [Persistent]: which uses [sled](https://docs.rs/sled/0.34.6/sled/index.html).

For building the project use: `cargo build`
For testing the project use: `cargo test`
For benchmarking the project use: `cargo bench`
