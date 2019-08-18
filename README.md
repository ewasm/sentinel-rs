# sentinel.rs

Validator and metering injector for eWASM.

## Build

1. Install wasm-chisel

```sh
$ cargo install chisel
```

Make sure to install at least 0.5.0.

2. Build sentinel

```sh
$ cargo build --release --target wasm32-unknown-unknown
```

The resulting binary is at `target/wasm32-unknown-unknown/release/sentinel_rs.wasm`.

3. Transform with chisel

```sh
$ chisel run
```

Now you have the binary ready.

## Author(s)

Alex Beregszaszi

## License

Apache 2.0
