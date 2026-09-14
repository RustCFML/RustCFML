//! MCP *client* integration tests — CFML calling out to a remote MCP server.
//!
//! The server under test is RustCFML itself (`rustcfml mcp demo`), launched as
//! a subprocess by `mcpConnect()`. That makes this a genuine two-process round
//! trip over the real stdio transport rather than an in-memory stub, and it
//! exercises both halves of the feature against each other.

use std::path::PathBuf;
use std::process::Command;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp_app")
}

/// Run a CFML snippet through the real binary and return its stdout.
///
/// The snippet is written to a file rather than passed with `-c` because these
/// are multi-line scripts; the connect command is baked in so the child knows
/// which binary to launch.
fn run_cfml(body: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "rustcfml-mcp-client-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let page = dir.join("test.cfm");

    // All-named: CFML forbids mixing positional and named arguments, so the
    // canonical call form names every one.
    let connect = format!(
        r#"mcpConnect( transport = "stdio", command = "{} mcp demo {}" )"#,
        env!("CARGO_BIN_EXE_rustcfml"),
        fixtures_dir().display()
    );
    let source = format!("<cfscript>\nclient = {connect};\n{body}\n</cfscript>");
    std::fs::write(&page, source).expect("write page");

    let out = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
        .arg(&page)
        .output()
        .expect("run rustcfml");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "page failed: {}\nstdout: {stdout}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

#[test]
fn connecting_performs_the_handshake_and_reports_the_server() {
    let out = run_cfml(
        r#"
        info = client.info();
        writeOutput( "name=" & info.serverInfo.name & ";" );
        writeOutput( "version=" & info.serverInfo.version & ";" );
        writeOutput( "protocol=" & info.protocolVersion & ";" );
        writeOutput( "tools=" & structKeyExists( info.capabilities, "tools" ) & ";" );
        client.close();
        "#,
    );
    assert!(out.contains("name=demo;"), "got {out}");
    assert!(out.contains("version=1.2.3;"), "got {out}");
    assert!(out.contains("protocol=2025-06-18;"), "got {out}");
    assert!(out.contains("tools=true;"), "got {out}");
}

#[test]
fn listing_and_calling_a_remote_tool() {
    let out = run_cfml(
        r#"
        tools = client.tools();
        names = [];
        for ( t in tools ) { arrayAppend( names, t.name ); }
        writeOutput( "hasEcho=" & ( arrayContains( names, "echo" ) > 0 ) & ";" );

        result = client.call( "echo", { text = "ab", times = 3 } );
        writeOutput( "echo=" & result.content[ 1 ].text & ";" );

        // A tool returning structured data hands back that data directly,
        // rather than the protocol envelope.
        stats = client.call( name = "stats", arguments = { label = "hits" } );
        writeOutput( "label=" & stats.label & ";total=" & stats.total & ";" );
        client.close();
        "#,
    );
    assert!(out.contains("hasEcho=true;"), "got {out}");
    assert!(out.contains("echo=ababab;"), "got {out}");
    assert!(out.contains("label=hits;total=42;"), "got {out}");
}

#[test]
fn a_failing_remote_tool_throws_rather_than_returning_its_error_text() {
    // CFML's way of saying "this did not work" is an exception. Returning the
    // error message as if it were the answer is how bad data spreads.
    let out = run_cfml(
        r#"
        try {
            client.call( "explode" );
            writeOutput( "NOTHROWN;" );
        } catch ( any e ) {
            writeOutput( "caught=" & e.type & ";" );
            writeOutput( "msg=" & ( findNoCase( "tool blew up", e.message ) > 0 ) & ";" );
        }
        client.close();
        "#,
    );
    assert!(out.contains("caught=MCPToolError;"), "got {out}");
    assert!(out.contains("msg=true;"), "got {out}");
    assert!(!out.contains("NOTHROWN"), "got {out}");
}

#[test]
fn reading_remote_resources_and_prompts() {
    let out = run_cfml(
        r#"
        resources = client.resources();
        uris = [];
        for ( r in resources ) { arrayAppend( uris, r.uri ); }
        writeOutput( "hasReadme=" & ( arrayContains( uris, "doc://readme" ) > 0 ) & ";" );

        contents = client.read( "doc://readme" );
        writeOutput( "mime=" & contents[ 1 ].mimeType & ";" );

        // A templated resource is read by its concrete URI.
        page = client.read( "doc://pages/intro" );
        writeOutput( "page=" & page[ 1 ].text & ";" );

        prompts = client.prompts();
        writeOutput( "promptCount=" & ( arrayLen( prompts ) > 0 ) & ";" );
        got = client.prompt( "code_review", { language = "Rust" } );
        writeOutput( "role=" & got.messages[ 1 ].role & ";" );
        writeOutput( "said=" & got.messages[ 1 ].content.text & ";" );
        client.close();
        "#,
    );
    assert!(out.contains("hasReadme=true;"), "got {out}");
    assert!(out.contains("mime=text/markdown;"), "got {out}");
    assert!(out.contains("page=page=intro uri=doc://pages/intro;"), "got {out}");
    assert!(out.contains("promptCount=true;"), "got {out}");
    assert!(out.contains("role=assistant;"), "got {out}");
    assert!(out.contains("said=I review Rust code.;"), "got {out}");
}

#[test]
fn ping_and_close_behave_and_a_closed_client_refuses_work() {
    let out = run_cfml(
        r#"
        writeOutput( "ping=" & client.ping() & ";" );
        writeOutput( "open=" & client.isClosed() & ";" );
        client.close();
        writeOutput( "closed=" & client.isClosed() & ";" );
        try {
            client.tools();
            writeOutput( "STILLWORKS;" );
        } catch ( any e ) {
            writeOutput( "refused=" & ( findNoCase( "has been closed", e.message ) > 0 ) & ";" );
        }
        "#,
    );
    assert!(out.contains("ping=true;"), "got {out}");
    assert!(out.contains("open=false;"), "got {out}");
    assert!(out.contains("closed=true;"), "got {out}");
    assert!(out.contains("refused=true;"), "got {out}");
    assert!(!out.contains("STILLWORKS"), "got {out}");
}

#[test]
fn a_bad_connection_fails_at_connect_not_at_first_use() {
    let dir = std::env::temp_dir().join(format!("rustcfml-mcp-bad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let page = dir.join("bad.cfm");
    std::fs::write(
        &page,
        r#"<cfscript>
        try {
            client = mcpConnect( transport = "stdio", command = "definitely-not-a-real-binary-xyz" );
            writeOutput( "CONNECTED;" );
        } catch ( any e ) {
            writeOutput( "failed=true;" );
        }
        try {
            mcpConnect( transport = "carrier-pigeon", url = "x" );
        } catch ( any e ) {
            writeOutput( "unknownTransport=" & ( findNoCase( "unknown transport", e.message ) > 0 ) & ";" );
        }
        </cfscript>"#,
    )
    .expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_rustcfml")).arg(&page).output().expect("run");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("failed=true;"), "got {stdout}");
    assert!(!stdout.contains("CONNECTED"), "got {stdout}");
    assert!(stdout.contains("unknownTransport=true;"), "got {stdout}");
}
