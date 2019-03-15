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

	let memory_page_cost = 256 * 1024; // 256k gas for 1 page (64k) of memory

	// let config = pwasm_utils::rules::Set::default()
	//	.with_forbidden_floats() // Reject floating point opreations.
	//	.with_grow_cost(memory_page_cost);

	let config = pwasm_utils::rules::Set::default()
		.with_forbidden_floats();

	// Loading module
	let module = parity_wasm::deserialize_file(&args[1]).expect("Module deserialization to succeed");

	let result = utils::inject_gas_counter(
		module, &config
	).expect("Failed to inject gas. Some forbidden opcodes?");

	parity_wasm::serialize_to_file(&args[2], result).expect("Module serialization to succeed")
}
