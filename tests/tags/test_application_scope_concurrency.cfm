<cfscript>
suiteBegin( "application scope is live across concurrent requests" );

/*
 * The application scope must be SHARED and LIVE: a write made by one request is
 * immediately visible to every other in-flight request. Verified against Lucee
 * 7.0.4.34.
 *
 * Until v0.593.0 RustCFML gave each request a private COPY of the scope and
 * republished the whole thing at request end, so:
 *   - a write was invisible to concurrent requests, and
 *   - the last request to finish overwrote the entire scope (lost updates).
 * That breaks every guard-once idiom under concurrency
 * (`if ( !StructKeyExists( application, "x" ) ) { expensive(); … }`). Preside's
 * `_reloadRequired()` is exactly that shape, so 8 concurrent cold requests each
 * re-booted the whole framework, and `StructClear( application )` plus racing
 * write-backs could leave a half-built framework installed permanently.
 *
 * ⚠️ These need a SERVER: the defect is cross-request, so it is invisible to a
 * single-request test — which is why a 7500-assertion suite never caught it.
 * Discover the live port from cgi.server_port and skip on the CLI, the pattern
 * tests/tags/test_tags_cfscript_statements.cfm already uses.
 */

serverPort = structKeyExists( cgi, "server_port" ) ? trim( cgi.server_port ) : "";
if ( serverPort == "" || serverPort == "0" ) {
    writeOutput( chr(10) & "  skipped application-scope concurrency subtests (no cgi.server_port — run via rustcfml --serve)" & chr(10) );
} else {
    baseUrl    = "http://127.0.0.1:" & serverPort;
    targetPath = "/tests/tags/app_scope_visibility_target.cfm";
    runKey     = "appvis_" & replace( createUUID(), "-", "", "all" );

    // ---- 1. a write from THIS request is visible to another request ----------
    application[ runKey ] = "set-by-runner";
    http url="#baseUrl##targetPath#?mode=read&key=#runKey#" method="GET" result="readBack";
    assert( "target responds", readBack.statuscode, "200 OK" );
    assert( "a write made by the still-running caller is visible to another request",
            trim( readBack.filecontent ), "set-by-runner" );

    // ---- 2. and an UPDATE is visible too (not just the first write) ----------
    application[ runKey ] = "updated-by-runner";
    http url="#baseUrl##targetPath#?mode=read&key=#runKey#" method="GET" result="readBack2";
    assert( "a subsequent update is visible to another request",
            trim( readBack2.filecontent ), "updated-by-runner" );

    // ---- 3. a write from ANOTHER request, made mid-flight, is visible here ---
    // The target writes then sleeps, so it is still running when we read.
    holdKey = runKey & "_hold";
    thread name="appvis_holder" baseUrl=baseUrl targetPath=targetPath holdKey=holdKey {
        http url="#attributes.baseUrl##attributes.targetPath#?mode=hold&key=#attributes.holdKey#&value=held-by-target&ms=2500"
             method="GET" result="local.ignored";
    }
    sleep( 900 ); // the target has written by now but has NOT returned
    assertTrue( "a mid-flight write by another request is visible here",
                ( application[ holdKey ] ?: "" ) == "held-by-target" );
    thread action="join" name="appvis_holder" timeout=20000;

    // ---- 4. concurrent read-modify-write does not lose updates --------------
    // 6 parallel requests each increment the same key under an exclusive lock.
    // With a per-request snapshot scope every increment but one is silently
    // dropped; with a live scope the total is exactly 6.
    bumpKey = runKey & "_bump";
    application[ bumpKey ] = 0;
    for ( i = 1; i <= 6; i++ ) {
        thread name="appvis_bump_#i#" baseUrl=baseUrl targetPath=targetPath bumpKey=bumpKey {
            http url="#attributes.baseUrl##attributes.targetPath#?mode=bump&key=#attributes.bumpKey#"
                 method="GET" result="local.ignored";
        }
    }
    for ( i = 1; i <= 6; i++ ) {
        thread action="join" name="appvis_bump_#i#" timeout=30000;
    }
    assert( "6 concurrent locked increments all land (no lost updates)",
            application[ bumpKey ], 6 );

    // ---- 5. a DELETE by another request is visible here too ------------------
    // The destructive direction matters as much as the additive one: with a shared
    // scope, another request removing a key must be observable, which is what makes
    // `StructClear( application )` / `applicationStop()` genuinely destructive to
    // in-flight requests (Lucee's behaviour).
    delKey = runKey & "_del";
    application[ delKey ] = "present";
    http url="#baseUrl##targetPath#?mode=delete&key=#delKey#" method="GET" result="delResult";
    assert( "the other request saw the key and deleted it", trim( delResult.filecontent ), "deleted" );
    assertFalse( "a delete by another request is visible here",
                 structKeyExists( application, delKey ) );

    // ---- tidy up: do not leave probe keys in a long-lived application scope ---
    structDelete( application, runKey );
    structDelete( application, holdKey );
    structDelete( application, bumpKey );
    structDelete( application, delKey );
}

suiteEnd();
</cfscript>
