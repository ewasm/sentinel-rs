extern crate ewasm_api;
extern crate parity_wasm;
extern crate pwasm_utils;
extern crate eci;

fn validate_contract(code: &[u8]) -> bool {
    let mut checker = eci::checker::EcicChecker::default(&code.to_vec());
    checker.fire();
    return match checker.status() {
        eci::checklist::CheckStatus::Unknown => true,
        eci::checklist::CheckStatus::Nonexistent => false,
        eci::checklist::CheckStatus::Malformed => false,
        eci::checklist::CheckStatus::Good => true,
    }
}

fn inject_metering(code: &[u8]) -> Result<Vec<u8>, parity_wasm::elements::Error> {
    if !validate_contract(code) {
        return Err(parity_wasm::elements::Error::Other("Contract doesn't meet ECI/EEI restrictions."));
    }

    let module = parity_wasm::deserialize_buffer(&code)?;

    let result = match pwasm_utils::inject_gas_counter(module, &Default::default()) {
        Ok(output) => output,
        Err(_) => return Err(parity_wasm::elements::Error::Other("Metering injection failed.")),
    };

    parity_wasm::serialize(result)
}

#[no_mangle]
pub extern "C" fn main() {
    let code = ewasm_api::calldata_acquire();

    let total_cost = 32 * code.len();
    ewasm_api::consume_gas(total_cost as u64);

    match inject_metering(&code) {
        Ok(output) => ewasm_api::finish_data(&output),
        // TODO: return reason in revert
        Err(_) => ewasm_api::revert(),
    }
}
