extern crate ewasm_api;
extern crate parity_wasm;
extern crate pwasm_utils;

#[no_mangle]
pub extern "C" fn main() {
    let code = ewasm_api::calldata_acquire();

    let total_cost = 32 * code.len();
    ewasm_api::consume_gas(total_cost as u64);

    let module = parity_wasm::deserialize_buffer(&code).expect("Failed to load module");

    let result = pwasm_utils::inject_gas_counter(module, &Default::default())
        .expect("Failed to inject gas. Some forbidden opcodes?");

    let output = parity_wasm::serialize(result).expect("Failed to serialize");

    ewasm_api::finish_data(&output);
}
