extern crate ewasm_api;
extern crate parity_wasm;
extern crate pwasm_utils;

fn inject_metering(code: &[u8]) -> Result<Vec<u8>, parity_wasm::elements::Error> {
    let module = parity_wasm::deserialize_buffer(&code)?;

    // TODO: extract values from the GasCostTable

    let memory_page_cost = 256 * 1024; // 256k gas for 1 page (64k) of memory

    let config = pwasm_utils::rules::Set::default()
        .with_forbidden_floats() // Reject floating point opreations.
        .with_grow_cost(memory_page_cost);

    let result = match pwasm_utils::inject_gas_counter(module, &config) {
        Ok(output) => output,
        Err(_) => {
            return Err(parity_wasm::elements::Error::Other(
                "Metering injection failed.",
            ));
        }
    };

    parity_wasm::serialize(result)
}

#[no_mangle]
pub extern "C" fn main() {
    let code = ewasm_api::calldata_acquire();

    // TODO: make this a configuration feature
    let total_cost = 32 * code.len();
    ewasm_api::consume_gas(total_cost as u64);

    match inject_metering(&code) {
        Ok(output) => ewasm_api::finish_data(&output),
        // TODO: return reason in revert
        Err(_) => ewasm_api::revert(),
    }
}
