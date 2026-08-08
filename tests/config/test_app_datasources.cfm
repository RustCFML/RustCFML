<cfscript>
suiteBegin("cfconfig — per-application datasources (this.datasources)");

// Per-application datasources defined in tests/Application.cfc via
// `this.datasources` must be resolved by queryExecute/cfquery for THIS
// application, ahead of the process-global cfconfig registry. This is the
// Lucee/BoxLang behaviour; previously RustCFML ignored this.datasources.
//
// RustCFML-only: these exercise RustCFML's in-memory sqlite datasources
// (rc_app_mem / rc_app_mem_str / rc_app_bad) declared in tests/Application.cfc,
// which don't exist on the Lucee test server. Skip the whole suite there.
if (isRustCFML()) {

// 1. Valid in-memory sqlite datasource (struct form) — a basic query works.
ok = false;
try {
    r = queryExecute("SELECT 1 AS n", [], { datasource: "rc_app_mem" });
    ok = (r.n[1] == 1);
} catch (any e) {
    ok = false;
}
assert("this.datasources struct form resolves (rc_app_mem)", ok, true);

// 2. Same via the bare connection-string form.
okStr = false;
try {
    r2 = queryExecute("SELECT 2 AS n", [], { datasource: "rc_app_mem_str" });
    okStr = (r2.n[1] == 2);
} catch (any e) {
    okStr = false;
}
assert("this.datasources string form resolves (rc_app_mem_str)", okStr, true);

// 3. DISCRIMINATOR: rc_app_bad is defined with a non-sqlite driver pointing at
//    an unreachable server. If this.datasources is honoured, the name resolves
//    to that (postgres) URL and the query MUST fail (connection refused, or
//    "driver not available" when the feature isn't compiled). If it were
//    ignored, the bare name would fall through to the sqlite catch-all and
//    "SELECT 1" would silently succeed — so a throw here proves real
//    per-application resolution, not an accidental sqlite pass.
assertThrows(
    "this.datasources is actually resolved (bad driver throws, no sqlite fallthrough)",
    function() {
        queryExecute("SELECT 1 AS n", [], { datasource: "rc_app_bad" });
    }
);

// 4. Lucee `type` key (GitHub #173) — Lucee/ACF/Preside declare the driver as
//    `type:"MySQL"` rather than RustCFML's native `driver` key. It must be
//    accepted as an alias; rc_app_type uses { type:"sqlite", … } and should
//    resolve and query exactly like the `driver`-keyed form.
okType = false;
try {
    rT = queryExecute("SELECT 4 AS n", [], { datasource: "rc_app_type" });
    okType = (rT.n[1] == 4);
} catch (any e) {
    okType = false;
}
assert("this.datasources `type` key is accepted as a driver alias (GitHub 173)", okType, true);

// 5. SAFETY (GitHub #173): an undefined datasource name must raise a clear
//    error, NOT silently fall back to a throwaway in-memory SQLite db (which
//    used to make config typos "work" against the wrong database).
assertThrows(
    "undefined datasource name errors instead of silently using sqlite (GitHub 173)",
    function() {
        queryExecute("SELECT 1 AS n", [], { datasource: "rc_app_undefined_xyz" });
    }
);

// 5b. The same safety must hold on the TRANSACTION path, which resolves its
//     datasource through cftransaction_start / the lazy begin rather than the
//     query builtin. It skipped the name check entirely, so `transaction { }`
//     on an unknown name opened (and CREATED) a SQLite file named after the
//     datasource and reported success — writes went to a throwaway db.
assertThrows(
    "unknown datasource in transaction{} errors instead of creating a sqlite file (GitHub 315)",
    function() {
        transaction {
            queryExecute("SELECT 1 AS n", [], { datasource: "rc_app_undefined_txn" });
        }
    }
);
assertFalse("no stray sqlite file created for the unknown datasource", fileExists(expandPath("./rc_app_undefined_txn")));

// 6. THREADS (GitHub #315): a cfthread body runs on a freshly-built child VM
//    that never loads Application.cfc, and the thread seed used to carry no
//    datasource config at all. Every per-application name was therefore
//    unresolvable inside a thread, and `this.datasource` (the default) fell
//    through to the process-wide `:memory:` sqlite catch-all — so transactional
//    background writes reported success against an empty throwaway database.
threadDsOk = "";
thread name="rcAppDsThread" {
    try {
        var rq = queryExecute("SELECT 5 AS n", [], { datasource: "rc_app_mem" });
        thread.plain = (rq.n[1] == 5) ? "OK" : "WRONG(#rq.n[1]#)";
    } catch (any e) {
        thread.plain = "ERR: " & e.message;
    }
    // The #315 shape: the same query wrapped in a transaction block, which
    // resolves its datasource through a different path (cftransaction_start /
    // the lazy begin) than a bare query.
    try {
        transaction {
            var tq = queryExecute("SELECT 6 AS n", [], { datasource: "rc_app_mem" });
            thread.txn = (tq.n[1] == 6) ? "OK" : "WRONG(#tq.n[1]#)";
        }
    } catch (any e) {
        thread.txn = "ERR: " & e.message;
    }
}
thread action="join" name="rcAppDsThread";
assert("this.datasources resolves inside a cfthread (GitHub 315)", cfthread.rcAppDsThread.plain, "OK");
assert("this.datasources resolves inside transaction{} in a cfthread (GitHub 315)", cfthread.rcAppDsThread.txn, "OK");

// 7. A nested thread (spawned from a thread) inherits the same config — the
//    seed is rebuilt from the child VM, so it must carry the fields forward.
thread name="rcAppDsOuter" {
    thread name="rcAppDsInner" {
        try {
            var iq = queryExecute("SELECT 7 AS n", [], { datasource: "rc_app_mem" });
            thread.inner = (iq.n[1] == 7) ? "OK" : "WRONG(#iq.n[1]#)";
        } catch (any e) {
            thread.inner = "ERR: " & e.message;
        }
    }
    thread action="join" name="rcAppDsInner";
    thread.result = cfthread.rcAppDsInner.inner;
}
thread action="join" name="rcAppDsOuter";
assert("this.datasources resolves in a NESTED cfthread (GitHub 315)", cfthread.rcAppDsOuter.result, "OK");

} // end if (isRustCFML())

suiteEnd();
</cfscript>
