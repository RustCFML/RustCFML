use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_vm::compile_file_cached;
use std::collections::HashMap;
use std::sync::Arc;

const VROOT: &str = "/app";

#[test]
fn cached_file_compilation_records_function_source_file() {
    let mut files = HashMap::new();
    files.insert(
        "lib/db.cfc".to_string(),
        r#"
<cfcomponent>
    <cffunction name="read">
        <cfargument name="table_name" />
        <cfreturn arguments.table_name />
    </cffunction>
</cfcomponent>
"#
        .as_bytes()
        .to_vec(),
    );

    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));
    let component_path = format!("{}/lib/db.cfc", VROOT);
    let program = compile_file_cached(&component_path, None, vfs.as_ref()).unwrap();
    let function = program
        .functions
        .iter()
        .find(|function| function.name.eq_ignore_ascii_case("read"))
        .expect("compiled read function");

    assert_eq!(Some(component_path.as_str()), function.source_file.as_deref());
}
