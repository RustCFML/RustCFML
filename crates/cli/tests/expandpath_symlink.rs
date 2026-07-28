//! Regression test: `expandPath` is a virtual-path operation (Lucee/ACF
//! parity) — it must normalize lexically (`.`/`..`/`//`) but NEVER resolve
//! symlinks. Canonicalizing rewrote paths through symlinked directories
//! (e.g. a Preside extension symlinked into application/extensions) to their
//! real location, breaking framework security checks that prefix-compare
//! expandPath results (Preside's StaticAssetDownload 404'd every
//! symlinked-extension admin asset).

#[cfg(unix)]
#[test]
fn expandpath_preserves_symlinks_and_normalizes_lexically() {
    let tmp = std::env::temp_dir().join(format!("rcfml_ep_symlink_{}", std::process::id()));
    let real = tmp.join("real_target");
    let app = tmp.join("app");
    std::fs::create_dir_all(real.join("sub")).unwrap();
    std::fs::create_dir_all(&app).unwrap();
    std::os::unix::fs::symlink(&real, app.join("linked")).unwrap();
    std::fs::write(real.join("sub/file.txt"), "x").unwrap();

    std::fs::write(
        app.join("probe.cfm"),
        "<cfoutput>#expandPath( './linked/sub/file.txt' )#|#expandPath( './linked/../linked/./sub/' )#</cfoutput>",
    )
    .unwrap();

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rustcfml"))
        .arg(app.join("probe.cfm"))
        .output()
        .expect("run rustcfml");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parts: Vec<&str> = stdout.trim().split('|').collect();
    assert_eq!(parts.len(), 2, "unexpected output: {stdout}");

    let linked = app.join("linked");
    assert_eq!(
        parts[0],
        linked.join("sub/file.txt").to_string_lossy(),
        "symlink must be preserved, not resolved to {real:?}"
    );
    assert_eq!(
        parts[1],
        format!("{}/", linked.join("sub").to_string_lossy()),
        "lexical ../. normalization (with trailing slash mirrored)"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
