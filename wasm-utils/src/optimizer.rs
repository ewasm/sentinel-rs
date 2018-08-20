#[cfg(features = "std")]
use std::collections::{HashSet as Set};
#[cfg(not(features = "std"))]
use std::collections::{BTreeSet as Set};
use std::vec::Vec;

use parity_wasm::elements;

use symbols::{Symbol, expand_symbols, push_code_symbols, resolve_function};

#[derive(Debug)]
pub enum Error {
	/// Since optimizer starts with export entries, export
	///   section is supposed to exist.
	NoExportSection,
}

pub fn optimize(
	module: &mut elements::Module, // Module to optimize
	used_exports: Vec<&str>,       // List of only exports that will be usable after optimization
) -> Result<(), Error> {
	// WebAssembly exports optimizer
	// Motivation: emscripten compiler backend compiles in many unused exports
	//   which in turn compile in unused imports and leaves unused functions

	// Algo starts from the top, listing all items that should stay
	let mut stay = Set::new();
	for (index, entry) in module.export_section().ok_or(Error::NoExportSection)?.entries().iter().enumerate() {
		if used_exports.iter().find(|e| **e == entry.field()).is_some() {
			stay.insert(Symbol::Export(index));
		}
	}

	// If there is start function in module, it should stary
	module.start_section().map(|ss| stay.insert(resolve_function(&module, ss)));

	// All symbols used in data/element segments are also should be preserved
	let mut init_symbols = Vec::new();
	if let Some(data_section) = module.data_section() {
		for segment in data_section.entries() {
			push_code_symbols(&module, segment.offset().code(), &mut init_symbols);
		}
	}
	if let Some(elements_section) = module.elements_section() {
		for segment in elements_section.entries() {
			push_code_symbols(&module, segment.offset().code(), &mut init_symbols);
			for func_index in segment.members() {
				stay.insert(resolve_function(&module, *func_index));
			}
		}
	}
	for symbol in init_symbols.drain(..) { stay.insert(symbol); }

	// Call function which will traverse the list recursively, filling stay with all symbols
	// that are already used by those which already there
	expand_symbols(module, &mut stay);

	for symbol in stay.iter() {
		trace!("symbol to stay: {:?}", symbol);
	}

	// Keep track of referreable symbols to rewire calls/globals
	let mut eliminated_funcs = Vec::new();
	let mut eliminated_globals = Vec::new();
	let mut eliminated_types = Vec::new();

	// First, iterate through types
	let mut index = 0;
	let mut old_index = 0;

	{
		loop {
			if type_section(module).map(|section| section.types_mut().len()).unwrap_or(0) == index { break; }

			if stay.contains(&Symbol::Type(old_index)) {
				index += 1;
			} else {
				type_section(module)
					.expect("If type section does not exists, the loop will break at the beginning of first iteration")
					.types_mut().remove(index);
				eliminated_types.push(old_index);
				trace!("Eliminated type({})", old_index);
			}
			old_index += 1;
		}
	}

	// Second, iterate through imports
	let mut top_funcs = 0;
	let mut top_globals = 0;
	index = 0;
	old_index = 0;

	if let Some(imports) = import_section(module) {
		loop {
			let mut remove = false;
			match imports.entries()[index].external() {
				&elements::External::Function(_) => {
					if stay.contains(&Symbol::Import(old_index)) {
						index += 1;
					} else {
						remove = true;
						eliminated_funcs.push(top_funcs);
						trace!("Eliminated import({}) func({}, {})", old_index, top_funcs, imports.entries()[index].field());
					}
					top_funcs += 1;
				},
				&elements::External::Global(_) => {
					if stay.contains(&Symbol::Import(old_index)) {
						index += 1;
					} else {
						remove = true;
						eliminated_globals.push(top_globals);
						trace!("Eliminated import({}) global({}, {})", old_index, top_globals, imports.entries()[index].field());
					}
					top_globals += 1;
				},
				_ => {
					index += 1;
				}
			}
			if remove {
				imports.entries_mut().remove(index);
			}

			old_index += 1;

			if index == imports.entries().len() { break; }
		}
	}

	// Third, iterate through globals
	if let Some(globals) = global_section(module) {
		index = 0;
		old_index = 0;

		loop {
			if globals.entries_mut().len() == index { break; }
			if stay.contains(&Symbol::Global(old_index)) {
				index += 1;
			} else {
				globals.entries_mut().remove(index);
				eliminated_globals.push(top_globals + old_index);
				trace!("Eliminated global({})", top_globals + old_index);
			}
			old_index += 1;
		}
	}

	// Forth, delete orphaned functions
	if function_section(module).is_some() && code_section(module).is_some() {
		index = 0;
		old_index = 0;

		loop {
			if function_section(module).expect("Functons section to exist").entries_mut().len() == index { break; }
			if stay.contains(&Symbol::Function(old_index)) {
				index += 1;
			} else {
				function_section(module).expect("Functons section to exist").entries_mut().remove(index);
				code_section(module).expect("Code section to exist").bodies_mut().remove(index);

				eliminated_funcs.push(top_funcs + old_index);
				trace!("Eliminated function({})", top_funcs + old_index);
			}
			old_index += 1;
		}
	}

	// Fifth, eliminate unused exports
	{
		let exports = export_section(module).ok_or(Error::NoExportSection)?;

		index = 0;
		old_index = 0;

		loop {
			if exports.entries_mut().len() == index { break; }
			if stay.contains(&Symbol::Export(old_index)) {
				index += 1;
			} else {
				trace!("Eliminated export({}, {})", old_index, exports.entries_mut()[index].field());
				exports.entries_mut().remove(index);
			}
			old_index += 1;
		}
	}

	if eliminated_globals.len() > 0 || eliminated_funcs.len() > 0 || eliminated_types.len() > 0 {
		// Finaly, rewire all calls, globals references and types to the new indices
		//   (only if there is anything to do)
		eliminated_globals.sort();
		eliminated_funcs.sort();
		eliminated_types.sort();

		for section in module.sections_mut() {
			match section {
				&mut elements::Section::Start(ref mut func_index) if eliminated_funcs.len() > 0 => {
					let totalle = eliminated_funcs.iter().take_while(|i| (**i as u32) < *func_index).count();
					*func_index -= totalle as u32;
				},
				&mut elements::Section::Function(ref mut function_section) if eliminated_types.len() > 0 => {
					for ref mut func_signature in function_section.entries_mut() {
						let totalle = eliminated_types.iter().take_while(|i| (**i as u32) < func_signature.type_ref()).count();
						*func_signature.type_ref_mut() -= totalle as u32;
					}
				},
				&mut elements::Section::Import(ref mut import_section) if eliminated_types.len() > 0 => {
					for ref mut import_entry in import_section.entries_mut() {
						if let &mut elements::External::Function(ref mut type_ref) = import_entry.external_mut() {
							let totalle = eliminated_types.iter().take_while(|i| (**i as u32) < *type_ref).count();
							*type_ref -= totalle as u32;
						}
					}
				},
				&mut elements::Section::Code(ref mut code_section) if eliminated_globals.len() > 0 || eliminated_funcs.len() > 0 => {
					for ref mut func_body in code_section.bodies_mut() {
						if eliminated_funcs.len() > 0 {
							update_call_index(func_body.code_mut(), &eliminated_funcs);
						}
						if eliminated_globals.len() > 0 {
							update_global_index(func_body.code_mut().elements_mut(), &eliminated_globals)
						}
						if eliminated_types.len() > 0 {
							update_type_index(func_body.code_mut(), &eliminated_types)
						}
					}
				},
				&mut elements::Section::Export(ref mut export_section) => {
					for ref mut export in export_section.entries_mut() {
						match export.internal_mut() {
							&mut elements::Internal::Function(ref mut func_index) => {
								let totalle = eliminated_funcs.iter().take_while(|i| (**i as u32) < *func_index).count();
								*func_index -= totalle as u32;
							},
							&mut elements::Internal::Global(ref mut global_index) => {
								let totalle = eliminated_globals.iter().take_while(|i| (**i as u32) < *global_index).count();
								*global_index -= totalle as u32;
							},
							_ => {}
						}
					}
				},
				&mut elements::Section::Global(ref mut global_section) => {
					for ref mut global_entry in global_section.entries_mut() {
						update_global_index(global_entry.init_expr_mut().code_mut(), &eliminated_globals)
					}
				},
				&mut elements::Section::Data(ref mut data_section) => {
					for ref mut segment in data_section.entries_mut() {
						update_global_index(segment.offset_mut().code_mut(), &eliminated_globals)
					}
				},
				&mut elements::Section::Element(ref mut elements_section) => {
					for ref mut segment in elements_section.entries_mut() {
						update_global_index(segment.offset_mut().code_mut(), &eliminated_globals);
						// update all indirect call addresses initial values
						for func_index in segment.members_mut() {
							let totalle = eliminated_funcs.iter().take_while(|i| (**i as u32) < *func_index).count();
							*func_index -= totalle as u32;
						}
					}
				},
				_ => { }
			}
		}
	}

	Ok(())
}


pub fn update_call_index(instructions: &mut elements::Instructions, eliminated_indices: &[usize]) {
	use parity_wasm::elements::Instruction::*;
	for instruction in instructions.elements_mut().iter_mut() {
		if let &mut Call(ref mut call_index) = instruction {
			let totalle = eliminated_indices.iter().take_while(|i| (**i as u32) < *call_index).count();
			trace!("rewired call {} -> call {}", *call_index, *call_index - totalle as u32);
			*call_index -= totalle as u32;
		}
	}
}

/// Updates global references considering the _ordered_ list of eliminated indices
pub fn update_global_index(instructions: &mut Vec<elements::Instruction>, eliminated_indices: &[usize]) {
	use parity_wasm::elements::Instruction::*;
	for instruction in instructions.iter_mut() {
		match instruction {
			&mut GetGlobal(ref mut index) | &mut SetGlobal(ref mut index) => {
				let totalle = eliminated_indices.iter().take_while(|i| (**i as u32) < *index).count();
				trace!("rewired global {} -> global {}", *index, *index - totalle as u32);
				*index -= totalle as u32;
			},
			_ => { },
		}
	}
}

/// Updates global references considering the _ordered_ list of eliminated indices
pub fn update_type_index(instructions: &mut elements::Instructions, eliminated_indices: &[usize]) {
	use parity_wasm::elements::Instruction::*;
	for instruction in instructions.elements_mut().iter_mut() {
		if let &mut CallIndirect(ref mut call_index, _) = instruction {
			let totalle = eliminated_indices.iter().take_while(|i| (**i as u32) < *call_index).count();
			trace!("rewired call_indrect {} -> call_indirect {}", *call_index, *call_index - totalle as u32);
			*call_index -= totalle as u32;
		}
	}
}

pub fn import_section<'a>(module: &'a mut elements::Module) -> Option<&'a mut elements::ImportSection> {
   for section in module.sections_mut() {
		if let &mut elements::Section::Import(ref mut sect) = section {
			return Some(sect);
		}
	}
	None
}

pub fn global_section<'a>(module: &'a mut elements::Module) -> Option<&'a mut elements::GlobalSection> {
   for section in module.sections_mut() {
		if let &mut elements::Section::Global(ref mut sect) = section {
			return Some(sect);
		}
	}
	None
}

pub fn function_section<'a>(module: &'a mut elements::Module) -> Option<&'a mut elements::FunctionSection> {
   for section in module.sections_mut() {
		if let &mut elements::Section::Function(ref mut sect) = section {
			return Some(sect);
		}
	}
	None
}

pub fn code_section<'a>(module: &'a mut elements::Module) -> Option<&'a mut elements::CodeSection> {
   for section in module.sections_mut() {
		if let &mut elements::Section::Code(ref mut sect) = section {
			return Some(sect);
		}
	}
	None
}

pub fn export_section<'a>(module: &'a mut elements::Module) -> Option<&'a mut elements::ExportSection> {
   for section in module.sections_mut() {
		if let &mut elements::Section::Export(ref mut sect) = section {
			return Some(sect);
		}
	}
	None
}

pub fn type_section<'a>(module: &'a mut elements::Module) -> Option<&'a mut elements::TypeSection> {
   for section in module.sections_mut() {
		if let &mut elements::Section::Type(ref mut sect) = section {
			return Some(sect);
		}
	}
	None
}

#[cfg(test)]
mod tests {

	use parity_wasm::{builder, elements};
	use super::*;

	/// @spec 0
	/// Optimizer presumes that export section exists and contains
	/// all symbols passed as a second parameter. Since empty module
	/// obviously contains no export section, optimizer should return
	/// error on it.
	#[test]
	fn empty() {
		let mut module = builder::module().build();
		let result = optimize(&mut module, vec!["_call"]);

		assert!(result.is_err());
	}

	/// @spec 1
	/// Imagine the unoptimized module has two own functions, `_call` and `_random`
	/// and exports both of them in the export section. During optimization, the `_random`
	/// function should vanish completely, given we pass `_call` as the only function to stay
	/// in the module.
	#[test]
	fn minimal() {
		let mut module = builder::module()
			.function()
				.signature().param().i32().build()
				.build()
			.function()
				.signature()
					.param().i32()
					.param().i32()
					.build()
				.build()
			.export()
				.field("_call")
				.internal().func(0).build()
			.export()
				.field("_random")
				.internal().func(1).build()
			.build();
		assert_eq!(module.export_section().expect("export section to be generated").entries().len(), 2);

		optimize(&mut module, vec!["_call"]).expect("optimizer to succeed");

		assert_eq!(
			1,
			module.export_section().expect("export section to be generated").entries().len(),
			"There should only 1 (one) export entry in the optimized module"
		);

		assert_eq!(
			1,
			module.function_section().expect("functions section to be generated").entries().len(),
			"There should 2 (two) functions in the optimized module"
		);
	}

	/// @spec 2
	/// Imagine there is one exported function in unoptimized module, `_call`, that we specify as the one
	/// to stay during the optimization. The code of this function uses global during the execution.
	/// This sayed global should survive the optimization.
	#[test]
	fn globals() {
		let mut module = builder::module()
			.global()
				.value_type().i32()
				.build()
			.function()
				.signature().param().i32().build()
				.body()
					.with_instructions(elements::Instructions::new(
						vec![
							elements::Instruction::GetGlobal(0),
							elements::Instruction::End
						]
					))
					.build()
				.build()
			.export()
				.field("_call")
				.internal().func(0).build()
			.build();

		optimize(&mut module, vec!["_call"]).expect("optimizer to succeed");

		assert_eq!(
			1,
			module.global_section().expect("global section to be generated").entries().len(),
			"There should 1 (one) global entry in the optimized module, since _call function uses it"
		);
	}

	/// @spec 2
	/// Imagine there is one exported function in unoptimized module, `_call`, that we specify as the one
	/// to stay during the optimization. The code of this function uses one global during the execution,
	/// but we have a bunch of other unused globals in the code. Last globals should not survive the optimization,
	/// while the former should.
	#[test]
	fn globals_2() {
		let mut module = builder::module()
			.global()
				.value_type().i32()
				.build()
			.global()
				.value_type().i64()
				.build()
			.global()
				.value_type().f32()
				.build()
			.function()
				.signature().param().i32().build()
				.body()
					.with_instructions(elements::Instructions::new(
						vec![
							elements::Instruction::GetGlobal(1),
							elements::Instruction::End
						]
					))
					.build()
				.build()
			.export()
				.field("_call")
				.internal().func(0).build()
			.build();

		optimize(&mut module, vec!["_call"]).expect("optimizer to succeed");

		assert_eq!(
			1,
			module.global_section().expect("global section to be generated").entries().len(),
			"There should 1 (one) global entry in the optimized module, since _call function uses only one"
		);
	}

	/// @spec 3
	/// Imagine the unoptimized module has two own functions, `_call` and `_random`
	/// and exports both of them in the export section. Function `_call` also calls `_random`
	/// in its function body. The optimization should kick `_random` function from the export section
	/// but preserve it's body.
	#[test]
	fn call_ref() {
		let mut module = builder::module()
			.function()
				.signature().param().i32().build()
				.body()
					.with_instructions(elements::Instructions::new(
						vec![
							elements::Instruction::Call(1),
							elements::Instruction::End
						]
					))
					.build()
				.build()
			.function()
				.signature()
					.param().i32()
					.param().i32()
					.build()
				.build()
			.export()
				.field("_call")
				.internal().func(0).build()
			.export()
				.field("_random")
				.internal().func(1).build()
			.build();
		assert_eq!(module.export_section().expect("export section to be generated").entries().len(), 2);

		optimize(&mut module, vec!["_call"]).expect("optimizer to succeed");

		assert_eq!(
			1,
			module.export_section().expect("export section to be generated").entries().len(),
			"There should only 1 (one) export entry in the optimized module"
		);

		assert_eq!(
			2,
			module.function_section().expect("functions section to be generated").entries().len(),
			"There should 2 (two) functions in the optimized module"
		);
	}

	/// @spec 4
	/// Imagine the unoptimized module has an indirect call to function of type 1
	/// The type should persist so that indirect call would work
	#[test]
	fn call_indirect() {
		let mut module = builder::module()
			.function()
				.signature().param().i32().param().i32().build()
				.build()
			.function()
				.signature().param().i32().param().i32().param().i32().build()
				.build()
			.function()
				.signature().param().i32().build()
				.body()
					.with_instructions(elements::Instructions::new(
						vec![
							elements::Instruction::CallIndirect(1, 0),
							elements::Instruction::End
						]
					))
					.build()
				.build()
			.export()
				.field("_call")
				.internal().func(2).build()
			.build();

		optimize(&mut module, vec!["_call"]).expect("optimizer to succeed");

		assert_eq!(
			2,
			module.type_section().expect("type section to be generated").types().len(),
			"There should 2 (two) types left in the module, 1 for indirect call and one for _call"
		);

		let indirect_opcode = &module.code_section().expect("code section to be generated").bodies()[0].code().elements()[0];
		match *indirect_opcode {
			elements::Instruction::CallIndirect(0, 0) => {},
			_ => {
				panic!(
					"Expected call_indirect to use index 0 after optimization, since previois 0th was eliminated, but got {:?}",
					indirect_opcode
				);
			}
		}
	}

}
