//! `RUSTCFML_MAX_LIVE_THREADS` caps live cfthread OS threads process-wide: a
//! thread over the cap is reported TERMINATED with the reason instead of being
//! started (threads outlive their request, so nothing else bounds them).

use std::process::Command;

#[test]
fn threads_over_the_live_cap_are_refused_not_started() {
    let dir = std::env::temp_dir().join(format!("rustcfml-threadcap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let page = dir.join("t.cfm");
    std::fs::write(
        &page,
        r#"<cfscript>
for (i = 1; i <= 5; i++) { thread name="t#i#" { sleep(300); } }
thread action="join" name="t1,t2,t3,t4,t5";
for (i = 1; i <= 5; i++) { writeOutput(cfthread["t#i#"].status & chr(10)); }
</cfscript>"#,
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustcfml"))
        .arg(&page)
        .env("RUSTCFML_MAX_LIVE_THREADS", "2")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(text.matches("COMPLETED").count(), 2, "{text}");
    assert_eq!(text.matches("TERMINATED").count(), 3, "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
