//! RustCFML WebAssembly bindings

use cfml_codegen::compiler::CfmlCompiler;
use cfml_common::VERSION;
use cfml_compiler::parser::Parser;
use cfml_compiler::tag_parser;
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::CfmlVirtualMachine;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn version() -> String {
    VERSION.to_string()
}

#[wasm_bindgen]
pub struct CfmlEngine {
    output: String,
}

#[wasm_bindgen]
impl CfmlEngine {
    #[wasm_bindgen(constructor)]
    pub fn new() -> CfmlEngine {
        CfmlEngine {
            output: String::new(),
        }
    }

    #[wasm_bindgen]
    pub fn execute(&mut self, code: &str) -> Result<String, JsValue> {
        // Pre-process: convert CFML tags to script if needed
        let source = if tag_parser::has_cfml_tags(code) {
            tag_parser::tags_to_script(code)
        } else {
            code.to_string()
        };

        // Parse
        let mut parser = Parser::new(source);
        let ast = parser.parse().map_err(|e| {
            JsValue::from_str(&format!("Parse Error [line {}, col {}]: {}", e.line, e.column, e.message))
        })?;

        // Compile
        let compiler = CfmlCompiler::new();
        let program = compiler.compile(ast);

        // Execute
        let mut vm = CfmlVirtualMachine::new(program);

        // Register builtins
        for (name, value) in get_builtins() {
            vm.globals.insert(name, value);
        }
        for (name, func) in get_builtin_functions() {
            vm.builtins.insert(name, func);
        }
        vm.refresh_builtin_index();

        match vm.execute() {
            Ok(_) => {
                self.output = vm.get_output();
                Ok(self.output.clone())
            }
            Err(e) => Err(JsValue::from_str(&format!("{}", e))),
        }
    }

    #[wasm_bindgen]
    pub fn get_output(&self) -> String {
        self.output.clone()
    }
}

#[wasm_bindgen(start)]
pub fn main() {
    // Future: add console_error_panic_hook for better wasm error messages
}
