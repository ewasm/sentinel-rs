use parity_wasm::{elements, builder};


pub fn minify_hack(module: elements::Module)
	-> Result<elements::Module, elements::Module>
{
	let mbuilder = builder::from_module(module);

	// back to minified module
	let module = mbuilder.build();

	Ok(module)
}
