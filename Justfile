build:
    cargo build --release --target wasm32-unknown-unknown
    chisel run
