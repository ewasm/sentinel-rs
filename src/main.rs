extern crate parity_wasm;
#[macro_use]
extern crate log;

use std::str;

use parity_wasm::elements::*;

use std::env;

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        println!("Usage: {} in.wasm out.wasm", args[0]);
        return;
    }

    let module = parity_wasm::deserialize_file(&args[1]).expect("Failed to load module");

    if let Some(section) = module.function_section() {
        for (i, entry) in section.entries().iter().enumerate() {
            debug!("function {:?}", i);
        }
    }

    if let Some(section) = module.code_section() {
        for (i, entry) in section.bodies().iter().enumerate() {
            for opcode in entry.code().elements() {
              debug!("opcode {:?}", opcode)
              // iterate opcodes..
            }
        }
    }

    parity_wasm::serialize_to_file(&args[2], module).expect("Failed to write module");
}