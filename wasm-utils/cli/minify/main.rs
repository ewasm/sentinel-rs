extern crate parity_wasm;
extern crate pwasm_utils as utils;
extern crate pwasm_utils_cli as logger;

use std::env;

fn main() {
	logger::init_log();

	let args = env::args().collect::<Vec<_>>();
	if args.len() != 3 {
		println!("Usage: {} input_file.wasm output_file.wasm", args[0]);
		return;
	}

	// Loading module
	let module = parity_wasm::deserialize_file(&args[1]).expect("Module deserialization to succeed");

	let result = utils::minify_hack(module).expect("Failed to minify!");

	parity_wasm::serialize_to_file(&args[2], result).expect("Module serialization to succeed")
}
