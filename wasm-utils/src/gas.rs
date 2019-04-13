use std::vec::Vec;

use parity_wasm::{elements, builder};
use crate::rules;

static DEFAULT_START_GAS: i64 = 1000000000000; // 1 trillion

pub fn update_call_index(instructions: &mut elements::Instructions, inserted_import_index: u32, inserted_funcs: u32) {
	use parity_wasm::elements::Instruction::*;
	for instruction in instructions.elements_mut().iter_mut() {
		if let &mut Call(ref mut call_index) = instruction {
			if *call_index >= inserted_import_index { *call_index += inserted_funcs}
		}
	}
}



/// A block of code represented by it's start position and cost.
///
/// The block typically starts with instructions such as `loop`, `block`, `if`, etc.
///
/// An example of block:
///
/// ```ignore
/// loop
///   i64.const 1
///   get_local 0
///   i64.sub
///   tee_local 0
///   br_if 0
/// end
/// ```
///
/// The start of the block is `i64.const 1`.
///
#[derive(Debug)]
struct BlockEntry {
	/// Index of the first instruction (aka `Opcode`) in the block.
	start_pos: usize,
	/// Sum of costs of all instructions until end of the block.
	cost: u32,
	flow_up: bool,
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
	fn begin(&mut self, cursor: usize, cost: u32, flow_up: bool) {
		let block_idx = self.blocks.len();
		self.blocks.push(BlockEntry {
			start_pos: cursor,
			cost: cost,
			flow_up: flow_up,
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

	fn increment_control_flow(&mut self, val: u32) -> Result<(), ()> {
		/*
		;; if the current block and parent block has 0 cost, then we're seeing a sequence of nested blocks

		(block $B1
		  (block $B2
		    (block $B3
		      ...
		     )))

		;; instead of calling to useGas once after each block, we can sum up the 1 gas per `block` instructions
		;; and charge for all three at the top of block $B1

		;; instead of this:
		(block $B1
		  (call useGas (i32.const 1))
		  (block $B2
		    (call useGas (i32.const 1))
		    (block $B3
		      (call useGas (i32.const 1))
		      ...
		      )))

		;; do this:
		(block $B1
		  (call useGas (i32.const 3))
		  (block $B2
		    (block $B3
		      ...
		      )))
		*/

		// find closest ancestor block (starting from top of stack and going down) with blocked flow and add 1

		for (_i, stack_i) in self.stack.iter().rev().enumerate() {
			let block_i = self.blocks.get_mut(*stack_i).ok_or_else(|| ())?;
			if !block_i.flow_up || *stack_i == 0 {
				block_i.cost = block_i.cost.checked_add(val).ok_or_else(|| ())?;
				//println!("found ancestor with blocked flow or no parent. incrementing to new cost: {:?} and returning...", block_i.cost);
				break;
			}
		}
		Ok(())

	}

}

fn add_inline_gas_func(module: elements::Module) -> elements::Module {
	use parity_wasm::elements::Instruction::*;

	let global_gas_index = module.globals_space() as u32;
	//println!("total globals before injecting gas global: {:?}", global_gas_index);

	let inline_gas_func_index = module.functions_space() as u32;

	let b = builder::from_module(module)
				// the export is a workaround to add debug name "inlineUseGas"
				.with_export(elements::ExportEntry::new("inlineUseGas".to_string(), elements::Internal::Function(inline_gas_func_index)));

// don't export mutable global. wasmi can't deal
let mut b2 = b.global().mutable()
	.value_type().i64().init_expr(elements::Instruction::I64Const(DEFAULT_START_GAS))
		.build();

/*
	let mut b2 = b.global().mutable()
		.value_type().i64().init_expr(elements::Instruction::I64Const(DEFAULT_START_GAS))
			.build()
		.export()
			.field("gas_global")
			.internal().global(global_gas_index)
			.build();
*/

	b2.push_function(
			builder::function()
				.signature().param().i64().build()
				.body()
					.with_locals(vec![elements::Local::new(1, elements::ValueType::I64)])
					.with_instructions(elements::Instructions::new(vec![
						GetGlobal(global_gas_index),
						GetLocal(0), // local 0 is the input param
						I64Sub,
						TeeLocal(1), // save deducted gas to local, because stack item will be consumed by i64.lte_s
						I64Const(0 as i64),
						I64LtS,
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
	counter.begin(0, 0, false);

	for cursor in 0..instructions.elements().len() {
		let instruction = &instructions.elements()[cursor];
		match *instruction {
			Block(_) => {
				// Increment previous block with the cost of the current opcode.
				let instruction_cost = rules.process(instruction)?;
				//counter.increment(instruction_cost)?;

				// Begin new block. The cost of the following opcodes until `End` or `Else` will
				// be included into this block.

				// add cost, which may flow up to ancestor block
				counter.increment_control_flow(instruction_cost)?;

				counter.begin(cursor + 1, 0, true);

			},
			If(_) => {
				// Increment previous block with the cost of the current opcode.
				let instruction_cost = rules.process(instruction)?;
				//counter.increment(instruction_cost)?;
				counter.increment_control_flow(instruction_cost)?;

				// begin If with cost 1, to force new costs added to top of block
				counter.begin(cursor + 1, 0, false);
			},
			BrIf(_) => {
				// Increment previous block with the cost of the current opcode.
				let instruction_cost = rules.process(instruction)?;
				//counter.increment(instruction_cost)?;
				counter.increment_control_flow(instruction_cost)?;

				// on a br_if, we finalize the previous block because those instructions will always be executed.
				// intructions after the if will be executed conditionally, so we start a new block so that gasUsed can be called after the if.
				counter.finalize()?;

				// begin If with cost 1, to force new costs added to top of block
				counter.begin(cursor + 1, 0, false);
			},
			Loop(_) => {
				let instruction_cost = rules.process(instruction)?;
				//counter.increment(instruction_cost)?;
				//counter.increment_control_flow(instruction_cost)?;

				counter.begin(cursor + 1, 0, false);
				// charge for the loop after the loop instruction
				// need to do this because the loop could be executed many times (the br_if that jumps to the loop is a separate instruction and gas charge)
				counter.increment_control_flow(instruction_cost)?;
			},
			Br(_) => {
				// anything after a break is dead code.
				// for now, we treat dead code blocks like any other (the metering will not be executed)
				// TODO: handle properly and don't inject metering inside dead code blocks
				let instruction_cost = rules.process(instruction)?;
				counter.increment_control_flow(instruction_cost)?;
				counter.finalize()?;
				counter.begin(cursor + 1, 0, false);
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
				counter.begin(cursor + 1, 0, false);
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
				counter.begin(cursor + 1, 1, false);
			},
			Unreachable => {
				// charge nothing, do nothing
			},
			_ => {
				// An ordinal non control flow instruction. Just increment the cost of the current block.
				let instruction_cost = rules.process(instruction)?;
				//counter.increment(instruction_cost)?;
				counter.increment_control_flow(instruction_cost)?;
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

	let module2 = module3;

	let module1;
	let inserted_imports;

	module1 = module2;
	inserted_imports = 0;

	let mut module = module1;

	//let inserted_import_index = usegas_import_ix.unwrap();
	let inserted_import_index = 0;

	/*
	// calculate actual function index of the imported definition
	//    (substract all imports that are NOT functions)

	let gas_func = module.import_count(elements::ImportCountType::Function) as u32 - 1;
	*/


	// for inline gas function
	let inline_gas_func_index = module.functions_space() as u32;
	// need to inject inline gas function after the metering statements,
	// or the gas function itself with be metered and recursively call itself

	// the inserted_funcs doesnt matter.
	// only inserted_imports matters. imports and functions share the same indexes.
	// so a new import will bump all function indexes up by 1.

	let mut inserted_funcs = 1; // for the inline gas function

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
					if rules.grow_cost() > 0 {
						if inject_grow_counter(func_body.code_mut(), total_func) > 0 {
							need_grow_counter = true;
						}
					}
				}
			},
			// if we aren't adding any new imports, we dont need these.
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
