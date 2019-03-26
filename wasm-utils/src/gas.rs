use std::vec::Vec;

use parity_wasm::{elements, builder};
use crate::rules;

pub fn update_call_index(instructions: &mut elements::Instructions, inserted_import_index: u32, inserted_funcs: u32) {
	use parity_wasm::elements::Instruction::*;
	for instruction in instructions.elements_mut().iter_mut() {
		if let &mut Call(ref mut call_index) = instruction {
			if *call_index >= inserted_import_index { *call_index += inserted_funcs}
		}
	}
}

pub fn find_finish_import(module: elements::Module) -> (elements::Module, Option<u32>) {
	let mut finish_ix = None;

	for section in module.sections() {
		match *section {
			elements::Section::Import(ref import_section) => {
				for (i, e) in import_section.entries().iter().enumerate() {
					//println!("    {}.{}  external has index {:?}", e.module(), e.field(), i);
					if e.module() == "ethereum" && e.field() == "finish" {
						println!("ethereum.finish import has index: {}", i);
						finish_ix = Some(i as u32);
					}
				};
			},
			_ => {},
		}
	}

	(module, finish_ix)

}


pub fn find_usegas_import(module: elements::Module) -> (elements::Module, Option<u32>) {
	let mut usegas_ix = None;

	for section in module.sections() {
		match *section {
			elements::Section::Import(ref import_section) => {
				for (i, e) in import_section.entries().iter().enumerate() {
					if e.module() == "ethereum" && e.field() == "useGas" {
						println!("ethereum.useGas import has index: {}", i);
						usegas_ix = Some(i as u32);
					}
				};
			},
			_ => {},
		}
	}

	(module, usegas_ix)
}


#[derive(Debug)]
struct BlockEntry {
	/// Index of the first instruction (aka `Opcode`) in the block.
	start_pos: usize,
	/// Sum of costs of all instructions until end of the block.
	cost: u32,
}

struct Counter {
	/// All blocks in the order of theirs start position.
	blocks: Vec<BlockEntry>,

	// Stack of blocks. Each element is an index to a `self.blocks` vector.
	stack: Vec<usize>,
}

impl Counter {
	fn new() -> Counter {
		Counter {
			stack: Vec::new(),
			blocks: Vec::new(),
		}
	}

	/// Begin a new block.
	fn begin(&mut self, cursor: usize, cost: u32) {
		let block_idx = self.blocks.len();
		self.blocks.push(BlockEntry {
			start_pos: cursor,
			cost: cost,
		});
		self.stack.push(block_idx);
	}

	/// Finalize the current block.
	///
	/// Finalized blocks have final cost which will not change later.
	fn finalize(&mut self) -> Result<(), ()> {
		self.stack.pop().ok_or_else(|| ())?;
		Ok(())
	}

	/// Increment the cost of the current block by the specified value.
	fn increment(&mut self, val: u32) -> Result<(), ()> {
		let stack_top = self.stack.last_mut().ok_or_else(|| ())?;
		let top_block = self.blocks.get_mut(*stack_top).ok_or_else(|| ())?;

		top_block.cost = top_block.cost.checked_add(val).ok_or_else(|| ())?;

		Ok(())
	}
}

fn add_inline_gas_func(module: elements::Module) -> elements::Module {
	use parity_wasm::elements::Instruction::*;

	static DEFAULT_START_GAS: i32 = 2000000000; // 4 billion is too big for signed integer

	let global_gas_index = module.globals_space() as u32;
	//println!("total globals before injecting gas global: {:?}", global_gas_index);

	let inline_gas_func_index = module.functions_space() as u32;

	let b = builder::from_module(module)
				// the export is a workaround to add debug name "useGas"
				.with_export(elements::ExportEntry::new("inlineUseGas".to_string(), elements::Internal::Function(inline_gas_func_index)));

	let mut b2 = b.global().mutable()
		.value_type().i32().init_expr(elements::Instruction::I32Const(DEFAULT_START_GAS))
			.build()
		.export()
			.field("gas_global")
			.internal().global(global_gas_index)
			.build();

	b2.push_function(
			builder::function()
				.signature().param().i32().build()
				.body()
					.with_locals(vec![elements::Local::new(1, elements::ValueType::I32)])
					.with_instructions(elements::Instructions::new(vec![
						GetGlobal(global_gas_index),
						GetLocal(0), // local 0 is the input param
						I32Sub,
						TeeLocal(1), // save deducted gas to local, because stack item will be consumed by i32.lte_s
						I32Const(0 as i32),
						I32LtS,
						If(elements::BlockType::NoResult),
							Unreachable,
						End,
						GetLocal(1), // put deducted gas back on stack
						SetGlobal(global_gas_index), // save to global
						End,
					]))
					.build()
				.build()
		);

	b2.build()

}

// todo: also need revert_inline_calls

fn inject_finish_inline_calls(instructions: &mut elements::Instructions, finish_import_func: u32, finish_inline_func: u32) -> usize {
	use parity_wasm::elements::Instruction::*;
	let mut counter = 0;
	for instruction in instructions.elements_mut() {
		if let Call(ref call_index) = *instruction {
			if *call_index == finish_import_func {
				*instruction = Call(finish_inline_func);
			}
			counter += 1;
		}
	}
	counter
}


// instead of calling finish directly, first call useGas then call finish.
// this is an easy way to tell the host what the total gas used was (so host doesn't have to check the gas global)
fn add_inline_finish_func(module: elements::Module, global_gas_index: u32, imported_gas_func: u32, imported_finish_func: u32, inline_finish_func_ix: u32) -> elements::Module {
	use parity_wasm::elements::Instruction::*;

	static DEFAULT_START_GAS: i32 = 2000000000; // 4 billion is too big for signed integer

	let global_startgas_index = module.globals_space() as u32;
	//println!("total globals before injecting start gas global: {:?}", global_startgas_index);
	let global_gas_index = global_startgas_index - 1;

	//let inline_gas_func_index = module.functions_space() as u32;

	let b = builder::from_module(module)
				// the export is a workaround to add debug name "useGas"
				.with_export(elements::ExportEntry::new("inlineFinish".to_string(), elements::Internal::Function(inline_finish_func_ix)));

	let mut b2 = b.global().mutable()
		.value_type().i32().init_expr(elements::Instruction::I32Const(DEFAULT_START_GAS))
			.build()
		.export()
			.field("startgas_global")
			.internal().global(global_startgas_index)
			.build();

	b2.push_function(
			builder::function()
				.signature().params().i32().i32().build().build()
				.body()
				.with_instructions(elements::Instructions::new(vec![
					GetGlobal(global_startgas_index),
					GetGlobal(global_gas_index), // this is gas left.  should instead call with total gas used.
					I32Sub, // gas used
					Call(imported_gas_func),
					GetLocal(0),
					GetLocal(1),
					Call(imported_finish_func),
					// Unreachable, // not entirely sure about this
					End,
				]))
					.build()
				.build()
		);

	b2.build()

}

fn inject_grow_counter(instructions: &mut elements::Instructions, grow_counter_func: u32) -> usize {
	use parity_wasm::elements::Instruction::*;
	let mut counter = 0;
	for instruction in instructions.elements_mut() {
		if let GrowMemory(_) = *instruction {
			*instruction = Call(grow_counter_func);
			counter += 1;
		}
	}
	counter
}

fn add_grow_counter(module: elements::Module, rules: &rules::Set, gas_func: u32) -> elements::Module {
	use parity_wasm::elements::Instruction::*;

	let mut b = builder::from_module(module);
	b.push_function(
		builder::function()
			.signature().params().i64().build().with_return_type(Some(elements::ValueType::I64)).build()
			.body()
				.with_instructions(elements::Instructions::new(vec![
					GetLocal(0),
					GetLocal(0),
					I64Const(rules.grow_cost() as i64),
					I64Mul,
					// todo: there should be strong guarantee that it does not return anything on stack?
					Call(gas_func),
					GrowMemory(0),
					End,
				]))
				.build()
			.build()
	);

	b.build()
}

pub fn inject_counter(
	instructions: &mut elements::Instructions,
	rules: &rules::Set,
	inline_gas_func: u32,
) -> Result<(), ()> {
	use parity_wasm::elements::Instruction::*;

	let mut counter = Counter::new();

	// Begin an implicit function (i.e. `func...end`) block.
	counter.begin(0, 0);

	for cursor in 0..instructions.elements().len() {
		let instruction = &instructions.elements()[cursor];
		match *instruction {
			Block(_) => {
				// Increment previous block with the cost of the current opcode.
				let instruction_cost = rules.process(instruction)?;
				counter.increment(instruction_cost)?;

				// Begin new block.
				counter.begin(cursor + 1, 0);
			},
			If(_) => {
				// Increment previous block with the cost of the current opcode.
				let instruction_cost = rules.process(instruction)?;
				counter.increment(instruction_cost)?;

				counter.begin(cursor + 1, 0);
			},
			BrIf(_) => {
				// Increment previous block with the cost of the current opcode.
				let instruction_cost = rules.process(instruction)?;
				counter.increment(instruction_cost)?;

				// on a br_if, we finalize the previous block because those instructions will always be executed.
				// intructions after the if will be executed conditionally, so we start a new block so that gasUsed can be called after the if.
				counter.finalize()?;

				counter.begin(cursor + 1, 0);
			},
			Loop(_) => {
				let instruction_cost = rules.process(instruction)?;

				counter.begin(cursor + 1, 0);
				// charge for the loop after the loop instruction
				// need to do this because the loop could be executed many times (the br_if that jumps to the loop is a separate instruction and gas charge)
				counter.increment(instruction_cost)?;
			},
			Br(_) => {
				// anything after a break is dead code.
				// for now, we treat dead code blocks like any other (the metering will not be executed)
				// TODO: handle properly and don't inject metering inside dead code blocks
				let instruction_cost = rules.process(instruction)?;
				counter.increment(instruction_cost)?;
				counter.finalize()?;
				counter.begin(cursor + 1, 0);
			},
			// br_table is always followed by end (in the ecmul wasm code, at least)
			// BrTable(_,_) => { },
			// return is always followed by end (in the ecmul wasm code, at least)
			// Return => { },
			End => {
				// Just finalize current block.
				//counter.increment_control_flow(instruction_cost)?;
				// wasabi doesn't count end as an instruction, so neither will we (no gas charge)

				counter.finalize()?;
				counter.begin(cursor + 1, 0);
			},
			Else => {
				// `Else` opcode is being encountered. So the case we are looking at:
				//
				// if
				//   ...
				// else <-- cursor
				//   ...
				// end
				//
				// Finalize the current block ('then' part of the if statement),
				// and begin another one for the 'else' part.
				counter.finalize()?;
				counter.begin(cursor + 1, 1);
			},
			Unreachable => {
				// charge nothing, do nothing
			},
			_ => {
				// An ordinal non control flow instruction. Just increment the cost of the current block.
				let instruction_cost = rules.process(instruction)?;
				//counter.increment(instruction_cost)?;
				counter.increment(instruction_cost)?;
			}
		}
	}

	// Then insert metering calls.
	let mut cumulative_offset = 0;
	for block in counter.blocks {
		if block.cost > 0 {
			let effective_pos = block.start_pos + cumulative_offset;

			instructions.elements_mut().insert(effective_pos, I64Const(block.cost as i64));
			instructions.elements_mut().insert(effective_pos+1, Call(inline_gas_func));

			// Take into account these two inserted instructions.
			cumulative_offset += 2;
		}
	}

	Ok(())
}

/// Injects gas counter.
///
/// Can only fail if encounters operation forbidden by gas rules,
/// in this case it returns error with the original module.
pub fn inject_gas_counter(module: elements::Module, rules: &rules::Set)
	-> Result<elements::Module, elements::Module>
{

	let mbuilder = builder::from_module(module);
	let module3 = mbuilder.build();

	//let module_copy = mbuilder.build();

	let (module2, usegas_import_ix_check) = find_usegas_import(module3);

	let module1;
	let inserted_imports;
	if usegas_import_ix_check == None {
		let mut mbuilder2 = builder::from_module(module2);
		// Injecting useGas import
		let import_sig = mbuilder2.push_signature(
			builder::signature()
				.param().i32()
				.build_sig()
			);

		mbuilder2.push_import(
			builder::import()
				.module("ethereum")
				.field("useGas")
				.external().func(import_sig)
				.build()
			);

		module1 = mbuilder2.build();
		inserted_imports = 1;
	} else {
		module1 = module2;
		inserted_imports = 0;
	}

	let (module1a, usegas_import_ix) = find_usegas_import(module1);
	let inserted_import_index = usegas_import_ix.unwrap();

	// calculate actual function index of the imported definition
	//    (substract all imports that are NOT functions)


	// for inline gas function
	let inline_gas_func_index = module1a.functions_space() as u32;
	// need to inject inline gas function after the metering statements,
	// or the gas function itself with be metered and recursively call itself

	// the inserted_funcs doesnt matter.
	// only inserted_imports matters. imports and functions share the same indexes.
	// so a new import will bump all function indexes up by 1.

	let mut inserted_funcs = 1; // for the inline gas function

	let (mut module, finish_import_ix) = find_finish_import(module1a);

	let do_inline_finish = match (usegas_import_ix, finish_import_ix) {
		(Some(_usegas_i), Some(_finish_i)) => true,
		_ => false,
	};

	//do_inline_finish = false;
	let mut inline_finish_func_ix = 0;
	if do_inline_finish {
		inline_finish_func_ix = inline_gas_func_index + 1;
		inserted_funcs = inserted_funcs + 1; // for the inline finish function
	}

	let total_func = (inline_gas_func_index - 1) + inserted_funcs;

	let mut need_grow_counter = false;
	let mut error = false;

	// Updating calling addresses (all calls to function index >= `gas_func` should be incremented)
	for section in module.sections_mut() {
		match section {
			&mut elements::Section::Code(ref mut code_section) => {
				for ref mut func_body in code_section.bodies_mut() {
					// if we aren't adding any new imports, we dont need to do update_call_index
					if inserted_imports > 0 {
						update_call_index(func_body.code_mut(), inserted_import_index, inserted_imports);
					}
					if let Err(_) = inject_counter(func_body.code_mut(), rules, inline_gas_func_index) {
						error = true;
						break;
					}
					if do_inline_finish {
						inject_finish_inline_calls(func_body.code_mut(), finish_import_ix.unwrap(), inline_finish_func_ix);
					}
					if rules.grow_cost() > 0 {
						if inject_grow_counter(func_body.code_mut(), total_func) > 0 {
							need_grow_counter = true;
						}
					}
				}
			},
			&mut elements::Section::Export(ref mut export_section) => {
				if inserted_imports > 0 {
					for ref mut export in export_section.entries_mut() {
						if let &mut elements::Internal::Function(ref mut func_index) = export.internal_mut() {
							if *func_index >= inserted_import_index { *func_index += inserted_imports}
						}
					}
				}
			},
			&mut elements::Section::Element(ref mut elements_section) => {
				if inserted_imports > 0 {
					for ref mut segment in elements_section.entries_mut() {
						// update all indirect call addresses initial values
						for func_index in segment.members_mut() {
							if *func_index >= inserted_import_index { *func_index += inserted_imports}
						}
					}
				}
			},
			_ => { }
		}
	}

	if error { return Err(module); }

	// metering calls have been injected, now inject inline gas func 
	// inline gas func has to go first
	module = add_inline_gas_func(module);

	if do_inline_finish {
		// inline finish func has to go second
		let global_gas_index = module.globals_space() as u32;
		module = add_inline_finish_func(module, global_gas_index, usegas_import_ix.unwrap(), finish_import_ix.unwrap(), inline_finish_func_ix);
	}

	if need_grow_counter { Ok(add_grow_counter(module, rules, inline_gas_func_index)) } else { Ok(module) }
}

#[cfg(test)]
mod tests {

	extern crate wabt;

	use parity_wasm::{serialize, builder, elements};
	use super::*;
	use rules;

	#[test]
	fn simple_grow() {
		use parity_wasm::elements::Instruction::*;

		let module = builder::module()
			.global()
				.value_type().i64()
				.build()
			.function()
				.signature().param().i64().build()
				.body()
					.with_instructions(elements::Instructions::new(
						vec![
							GetGlobal(0),
							GrowMemory(0),
							End
						]
					))
					.build()
				.build()
			.build();

		let injected_module = inject_gas_counter(module, &rules::Set::default().with_grow_cost(10000)).unwrap();

		assert_eq!(
			&vec![
				I64Const(3),
				Call(0),
				GetGlobal(0),
				Call(2),
				End
			][..],
			injected_module
				.code_section().expect("function section should exist").bodies()[0]
				.code().elements()
		);
		assert_eq!(
			&vec![
				GetLocal(0),
				GetLocal(0),
				I64Const(10000),
				I64Mul,
				Call(0),
				GrowMemory(0),
				End,
			][..],
			injected_module
				.code_section().expect("function section should exist").bodies()[1]
				.code().elements()
		);

		let binary = serialize(injected_module).expect("serialization failed");
		self::wabt::wasm2wat(&binary).unwrap();
	}

	#[test]
	fn grow_no_gas_no_track() {
		use parity_wasm::elements::Instruction::*;

		let module = builder::module()
			.global()
				.value_type().i64()
				.build()
			.function()
				.signature().param().i64().build()
				.body()
					.with_instructions(elements::Instructions::new(
						vec![
							GetGlobal(0),
							GrowMemory(0),
							End
						]
					))
					.build()
				.build()
			.build();

		let injected_module = inject_gas_counter(module, &rules::Set::default()).unwrap();

		assert_eq!(
			&vec![
				I64Const(3),
				Call(0),
				GetGlobal(0),
				GrowMemory(0),
				End
			][..],
			injected_module
				.code_section().expect("function section should exist").bodies()[0]
				.code().elements()
		);

		assert_eq!(injected_module.functions_space(), 2);

		let binary = serialize(injected_module).expect("serialization failed");
		self::wabt::wasm2wat(&binary).unwrap();
	}

	#[test]
	fn simple() {
		use parity_wasm::elements::Instruction::*;

		let module = builder::module()
			.global()
				.value_type().i64()
				.build()
			.function()
				.signature().param().i64().build()
				.body()
					.with_instructions(elements::Instructions::new(
						vec![
							GetGlobal(0),
							End
						]
					))
					.build()
				.build()
			.build();

		let injected_module = inject_gas_counter(module, &Default::default()).unwrap();

		assert_eq!(
			&vec![
				I64Const(2),
				Call(0),
				GetGlobal(0),
				End
			][..],
			injected_module
				.code_section().expect("function section should exist").bodies()[0]
				.code().elements()
		);
	}

	#[test]
	fn nested() {
		use parity_wasm::elements::Instruction::*;

		let module = builder::module()
			.global()
				.value_type().i64()
				.build()
			.function()
				.signature().param().i64().build()
				.body()
					.with_instructions(elements::Instructions::new(
						vec![
							GetGlobal(0),
							Block(elements::BlockType::NoResult),
								GetGlobal(0),
								GetGlobal(0),
								GetGlobal(0),
							End,
							GetGlobal(0),
							End
						]
					))
					.build()
				.build()
			.build();

		let injected_module = inject_gas_counter(module, &Default::default()).unwrap();

		assert_eq!(
			&vec![
				I64Const(4),
				Call(0),
				GetGlobal(0),
				Block(elements::BlockType::NoResult),
					I64Const(4),
					Call(0),
					GetGlobal(0),
					GetGlobal(0),
					GetGlobal(0),
				End,
				GetGlobal(0),
				End
			][..],
			injected_module
				.code_section().expect("function section should exist").bodies()[0]
				.code().elements()
		);
	}

	#[test]
	fn ifelse() {
		use parity_wasm::elements::Instruction::*;

		let module = builder::module()
			.global()
				.value_type().i64()
				.build()
			.function()
				.signature().param().i64().build()
				.body()
					.with_instructions(elements::Instructions::new(
						vec![
							GetGlobal(0),
							If(elements::BlockType::NoResult),
								GetGlobal(0),
								GetGlobal(0),
								GetGlobal(0),
							Else,
								GetGlobal(0),
								GetGlobal(0),
							End,
							GetGlobal(0),
							End
						]
					))
					.build()
				.build()
			.build();

		let injected_module = inject_gas_counter(module, &Default::default()).unwrap();

		assert_eq!(
			&vec![
				I64Const(4),
				Call(0),
				GetGlobal(0),
				If(elements::BlockType::NoResult),
					I64Const(4),
					Call(0),
					GetGlobal(0),
					GetGlobal(0),
					GetGlobal(0),
				Else,
					I64Const(3),
					Call(0),
					GetGlobal(0),
					GetGlobal(0),
				End,
				GetGlobal(0),
				End
			][..],
			injected_module
				.code_section().expect("function section should exist").bodies()[0]
				.code().elements()
		);
	}

	#[test]
	fn call_index() {
		use parity_wasm::elements::Instruction::*;

		let module = builder::module()
			.global()
				.value_type().i64()
				.build()
			.function()
				.signature().param().i64().build()
				.body().build()
				.build()
			.function()
				.signature().param().i64().build()
				.body()
					.with_instructions(elements::Instructions::new(
						vec![
							Call(0),
							If(elements::BlockType::NoResult),
								Call(0),
								Call(0),
								Call(0),
							Else,
								Call(0),
								Call(0),
							End,
							Call(0),
							End
						]
					))
					.build()
				.build()
			.build();

		let injected_module = inject_gas_counter(module, &Default::default()).unwrap();

		assert_eq!(
			&vec![
				I64Const(4),
				Call(0),
				Call(1),
				If(elements::BlockType::NoResult),
					I64Const(4),
					Call(0),
					Call(1),
					Call(1),
					Call(1),
				Else,
					I64Const(3),
					Call(0),
					Call(1),
					Call(1),
				End,
				Call(1),
				End
			][..],
			injected_module
				.code_section().expect("function section should exist").bodies()[1]
				.code().elements()
		);
	}


	#[test]
	fn forbidden() {
		use parity_wasm::elements::Instruction::*;

		let module = builder::module()
			.global()
				.value_type().i64()
				.build()
			.function()
				.signature().param().i64().build()
				.body()
					.with_instructions(elements::Instructions::new(
						vec![
							F32Const(555555),
							End
						]
					))
					.build()
				.build()
			.build();

		let rules = rules::Set::default().with_forbidden_floats();

		if let Err(_) = inject_gas_counter(module, &rules) { }
		else { panic!("Should be error because of the forbidden operation")}

	}

}
