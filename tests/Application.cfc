component {

    this.name = "RustCFMLTests";

    // Session management on so session-scope tests can run (see
    // tests/core/test_session_scope_persist.cfm). Harmless for other tests.
    this.sessionManagement = true;
    this.sessionTimeout = createTimeSpan( 0, 0, 30, 0 );

    // Map "oop" to the tests/oop/ directory so createObject("component", "oop.Greeter") resolves
    this.mappings["/oop"] = getDirectoryFromPath(getCurrentTemplatePath()) & "oop/";

    // Map "core" to tests/core/ so createObject("component", "core.X") /
    // getComponentMetaData("core.X") resolve on Lucee too (RustCFML resolves the
    // unmapped package relatively; Lucee needs the explicit mapping — otherwise
    // core.* fixture tests ERROR cross-engine).
    this.mappings["/core"] = getDirectoryFromPath(getCurrentTemplatePath()) & "core/";

    // Map "tags" for any tag-based test includes
    this.mappings["/tags"] = getDirectoryFromPath(getCurrentTemplatePath()) & "tags/";

    // Map "compat_engine" so createObject/new on `compat_engine.fixtures.X`
    // resolves on Lucee too. RustCFML resolves an unmapped dotted package
    // relative to the including file; Lucee resolves dotted paths only through
    // the webroot or a mapping, so without this the fixtures are unreachable
    // there and the suite aborts mid-file.
    this.mappings["/compat_engine"] = getDirectoryFromPath(getCurrentTemplatePath()) & "compat_engine/";

    // Map "comments" so createObject("component", "comments.BlockCommentTags")
    // resolves (issue #69 fixture).
    this.mappings["/comments"] = getDirectoryFromPath(getCurrentTemplatePath()) & "comments/";

    // Distinct mapping name (NOT a real webroot subdirectory) used by
    // tags/test_mapping_include.cfm to isolate this.mappings-based cfinclude
    // resolution: a `/wheelsmapprobe/...` include can ONLY be found via this
    // mapping, never by webroot-relative path resolution. Points at tests/tags/.
    this.mappings["/wheelsmapprobe"] = getDirectoryFromPath(getCurrentTemplatePath()) & "tags/";

    // Mapping whose TARGET contains a "../" segment. Used by
    // tests/tags/test_mapping_dotdot_normalization.cfm to prove that file BIFs
    // (directoryList/expandPath/fileExists) collapse ".." consistently. Preside's
    // "/preside" mapping points at "../system"; an un-normalized resolution left
    // directoryList entries with a literal "tests/../system" prefix that
    // expandPath had already canonicalized away, breaking Preside's
    // _getAllObjectPaths prefix-strip. Points at tests/oop/ via tests/tags/../oop/.
    this.mappings["/dotdotprobe"] = getDirectoryFromPath(getCurrentTemplatePath()) & "tags/../oop/";

    // Custom tag path for tests/tags/test_customtag_path_deep_search.cfm.
    // The control tag sits at the path ROOT (ctpath_shallow.cfm); the gap tag
    // one subdirectory down (nested/ctpath_deep.cfm). Lucee searches custom
    // tag paths recursively when the engine's customTagDeepSearch flag is on
    // (server-level; the per-app this.customTagDeepSearch spelling is inert on
    // Lucee 7), so cross-engine runs of that suite need customTagDeepSearch
    // enabled in the Lucee config. String form, as Lucee also accepts.
    this.customtagpaths = getDirectoryFromPath(getCurrentTemplatePath()) & "tags/ctpathroot/";

    // Deep search is OFF by default in RustCFML too (stock-Lucee parity: a stray
    // .cfm under a tag path must not become invocable). RustCFML honours this
    // per-application spelling as a superset; on Lucee it is inert, so a
    // cross-engine run of that suite still needs server-level
    // customTagDeepSearch=true in .CFConfig.json.
    this.customTagDeepSearch = true;

    // Per-application S3 settings (Lucee parity, GitHub #334). Deliberately a
    // NON-RESOLVING host with the AWS documentation example keypair: presigning
    // is pure offline computation, so tests/s3/test_s3_app_config.cfm can prove
    // these values reach the S3 layer without a bucket, a network call, or any
    // environment variable. Nothing else in the suite touches S3.
    this.s3 = {
          accessKeyId  = "AKIAIOSFODNN7EXAMPLE"
        , awsSecretKey = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
        , host         = "app-config-probe.example.com"
    };

    // Per-application datasources (Lucee/BoxLang parity). Scoped to THIS
    // application and resolved by cfquery/queryExecute ahead of the global
    // cfconfig registry. Exercised by tests/config/test_app_datasources.cfm.
    //   rc_app_mem      — valid in-memory sqlite (struct form)
    //   rc_app_mem_str  — same, via the bare connection-string form
    //   rc_app_bad      — deliberately unreachable; used to PROVE the name is
    //                     resolved through this.datasources (a non-sqlite driver
    //                     must throw, not silently fall through to the sqlite
    //                     catch-all that an unresolved bare name would hit)
    //   rc_app_type     — declared the Lucee way with `type` instead of `driver`
    //                     (GitHub #173); must resolve to the same sqlite driver
    this.datasources = {
        "rc_app_mem"     : { driver: "sqlite", database: ":memory:" },
        "rc_app_mem_str" : "sqlite://:memory:",
        "rc_app_type"    : { type: "sqlite", database: ":memory:" },
        // connectionTimeout:1 keeps the unreachable-host discriminator fast:
        // it must still THROW (proving this.datasources is resolved, not a
        // silent sqlite fallthrough), but in ~1s instead of the 30s the r2d2
        // pool would otherwise spend retrying a doomed connection.
        "rc_app_bad"     : { driver: "postgresql", host: "127.0.0.1", port: "1", database: "definitely_absent", username: "x", password: "y", connectionTimeout: 1 }
    };

}
