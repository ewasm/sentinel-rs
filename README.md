# sentinel.rs

Validator and metering injector for eWASM.

## Build

1. Download wasm-chisel and build it

```sh
$ git clone https://github.com/wasmx/wasm-chisel
$ cd wasm-chisel
$ cargo build --release
```

The CLI is placed into `target/release/chisel`. Make sure this `chisel` binary is available in the path.

2. Build sentinel

```sh
$ cargo build --release --target wasm32-unknown-unknown
```

The resulting binary is at `target/wasm32-unknown-unknown/release/sentinel_rs.wasm`.

3. Transform with chisel

```sh
$ chisel target/wasm32-unknown-unknown/release/sentinel_rs.wasm sentinel.wasm
```

Now you have the binary ready.

## Author(s)

Alex Beregszaszi

## License

Apache 2.0
