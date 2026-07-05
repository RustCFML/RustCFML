//! Regression: `<cfcontent file="…" type="…">` serving a BINARY file.
//!
//! Preside serves every /preside/system/assets/* image through its
//! StaticAssetDownload handler with `content file="#assetFile#" type="image/png";
//! abort;`. Two bugs made those come back as `text/html` whitespace:
//!
//!   1. The `file=` handler read the file with `std::fs::read_to_string`, which
//!      ERRORS on any non-UTF-8 (binary) file, so `response_body` was never set
//!      and the response fell back to the template's whitespace output_buffer.
//!      Fix: read raw bytes into a `CfmlValue::Binary`.
//!
//!   2. `cfcontent type=` set only `response_content_type`, but Preside's
//!      getPageContext().getResponse().getContentType() shim reads the
//!      response_headers Content-Type. Its `_resetHttpResponseWithoutCookies()`
//!      does getContentType() → reset() → setContentType(saved) on flush, so the
//!      cfcontent type was lost and the hardcoded text/html default re-applied.
//!      Fix: write the type into BOTH channels, and let getContentType() fall
//!      back to response_content_type.
//!
//! Verified against Lucee 7: the favicon serves as image/png with its exact bytes.

use cfml_codegen::compiler::CfmlCompiler;
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vfs::{EmbeddedFs, Vfs};
use cfml_compiler::{parser::Parser, tag_parser};
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, MemoryStore, ServerState, SessionStore};
use std::collections::HashMap;
use std::sync::Arc;

const VROOT: &str = "/app";

// A tiny non-UTF-8 byte sequence: a PNG signature + a byte (0xFF) that is invalid
// as standalone UTF-8, so read_to_string would have errored on it.
const PNG_BYTES: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xFF, 0x00, 0x42];

const APP: &str = r##"
component {
    this.name = "cfcontent-binary-test";
    function onRequest(targetPage) { include "#targetPage#"; }
}
"##;

struct Served {
    body: Option<CfmlValue>,
    content_type: Option<String>,
    header_ct: Option<String>,
}

fn serve(page: &str) -> Served {
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    files.insert("Application.cfc".to_string(), APP.as_bytes().to_vec());
    files.insert("index.cfm".to_string(), page.as_bytes().to_vec());
    files.insert("logo.png".to_string(), PNG_BYTES.to_vec());
    let vfs: Arc<dyn Vfs> = Arc::new(EmbeddedFs::new(files, VROOT.to_string()));

    let page_path = format!("{}/index.cfm", VROOT);
    let source = vfs.read_to_string(&page_path).unwrap();
    let processed = if tag_parser::has_cfml_tags(&source) {
        tag_parser::tags_to_script(&source)
    } else {
        source
    };
    let ast = Parser::new(processed).parse().unwrap();
    let program = CfmlCompiler::new().compile(ast);

    let mut server_state = ServerState::with_production(false);
    server_state.sessions = Arc::new(MemoryStore::new()) as Arc<dyn SessionStore>;

    let mut vm = CfmlVirtualMachine::new(program);
    vm.vfs = vfs;
    vm.source_file = Some(page_path.clone());
    vm.base_template_path = Some(page_path);
    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }
    for scope in ["url", "cgi", "form"] {
        vm.globals
            .entry(scope.to_string())
            .or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
    }
    vm.server_state = Some(server_state);

    let _ = vm.execute_with_lifecycle();

    let header_ct = vm
        .response_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.clone());
    Served {
        body: vm.response_body.take(),
        content_type: vm.response_content_type.take(),
        header_ct,
    }
}

#[test]
fn cfcontent_file_serves_binary_bytes_verbatim() {
    let s = serve(r##"<cfcontent file="#expandPath('/logo.png')#" type="image/png">"##);
    match s.body {
        Some(CfmlValue::Binary(bytes)) => {
            assert_eq!(bytes, PNG_BYTES, "binary file must stream verbatim");
        }
        other => panic!("expected Binary response_body, got {:?}", other),
    }
}

#[test]
fn cfcontent_type_sets_content_type_on_both_channels() {
    let s = serve(r##"<cfcontent file="#expandPath('/logo.png')#" type="image/png">"##);
    assert_eq!(s.content_type.as_deref(), Some("image/png"));
    assert_eq!(
        s.header_ct.as_deref(),
        Some("image/png"),
        "cfcontent type= must also land on the response_headers Content-Type so \
         getPageContext().getResponse().getContentType() sees it"
    );
}

#[test]
fn cfcontent_type_survives_a_getcontenttype_setcontenttype_roundtrip() {
    // Mirrors Preside's _resetHttpResponseWithoutCookies(): capture the current
    // content type, reset, then re-apply it. Must round-trip as image/png, not
    // decay to the text/html default.
    let s = serve(
        r##"<cfcontent file="#expandPath('/logo.png')#" type="image/png">
<cfset resp = getPageContext().getResponse()>
<cfset ct = resp.getContentType()>
<cfset resp.setContentType( ct )>"##,
    );
    assert_eq!(
        s.header_ct.as_deref(),
        Some("image/png"),
        "getContentType()->setContentType() round-trip must preserve image/png"
    );
}
