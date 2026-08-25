<!---
    cfqueryparam accumulation must be per-call, not per-component-instance.

    Shape: ONE component instance shared by N concurrent threads (the
    application-scoped singleton every framework keeps for its data layer —
    moopa's record_writer lives in application.lib.db, and each request's
    db.save() goes through that same object). Its cfquery body calls a method
    on the same instance WHILE the statement's cfqueryparams are being
    collected:

        <cfquery>
            SELECT <cfqueryparam .../> AS a
            <cfloop ...>
                <cfset local.p = paramFor(...) />        <!--- same instance --->
                , <cfqueryparam attributeCollection="#local.p#" /> AS b#k#
            </cfloop>
        </cfquery>

    Under concurrency the bind lists of DIFFERENT threads cross. Two failure
    modes, both observed:
      - bind-count errors: "Wrong number of parameters passed to query.
        Got 2, needed 3" (PostgreSQL wording: "N positional parameter(s)
        supplied but the SQL only consumes M" / "statement needs 20
        placeholder(s) but only 3 of 3 supplied remain");
      - WORSE: no error, but the row comes back with another thread's values
        (thread 3 reads b1 = "t7"). A silent wrong-answer under load.

    The same instance running the same three placeholders with inline
    cfqueryparams (no method call in the body) is clean, and the helper shape
    is clean when each thread has its own instance and when run sequentially.
    So the accumulator is keyed to the component instance / call frame rather
    than to the executing request or thread.

    Repro class: two users saving records through a framework's shared
    write path at the same moment (titan: a quote's line editor autosaving on
    each keystroke — overlapping requests through moopa's db.save threw the
    bind-count errors above and, less visibly, could persist one request's
    values under another's parameters).

    Live in-process test on an inline in-memory SQLite datasource — needs no
    server and no PG. Lucee ships no SQLite JDBC driver, so the live legs are
    skipped there with one informational pass (the test_dml_returns_empty_query
    convention); with the sqlite-jdbc jar on Lucee's classpath the file runs and
    is green on Lucee 7 (cfthread, shared instance, same fixture).
    Every leg is caught so an engine that throws reports a count rather than
    aborting the file.
--->
<cfscript>
suiteBegin("cfqueryparam collection is per-call under concurrent use of one component instance");

THREADS = 8;
ITERS   = 40;
memDs   = { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite::memory:" };

function newFixture() {
    return createObject("component", "database.SharedQueryInstanceFixture").init(memDs);
}

// Run `iters` calls of `method` on `obj` for thread tag `tag`; return
// {errors, wrong}. wrong = rows that came back with values that were bound by
// SOME OTHER caller (the tag or counter does not match what this caller bound).
function hammer(required any obj, required string method, required string tag, required numeric iters) {
    var out = { errors: 0, wrong: 0, firstError: "" };
    var i = 0;
    var got = "";
    var want = "";
    for (i = 1; i <= arguments.iters; i++) {
        want = i & "|" & arguments.tag & "|" & arguments.tag;
        try {
            got = invoke(arguments.obj, arguments.method, { tag: arguments.tag, i: i });
            if (got != want) { out.wrong++; }
        } catch (any e) {
            out.errors++;
            if (out.firstError == "") { out.firstError = e.message ?: ""; }
        }
    }
    return out;
}

// Fan `method` across THREADS threads, each with its own tag; `sharedObj`
// is used by every thread when supplied, otherwise each thread creates its
// own instance. Returns the summed {errors, wrong, firstError}.
function fanOut(required string method, any sharedObj) {
    var t = 0;
    var names = [];
    var total = { errors: 0, wrong: 0, firstError: "" };
    var res = {};
    var runId = replace(createUUID(), "-", "", "all");
    for (t = 1; t <= THREADS; t++) {
        var tname = "qp_" & runId & "_" & t;
        arrayAppend(names, tname);
        if (structKeyExists(arguments, "sharedObj")) {
            thread name=tname action="run" tag="t#t#" method=arguments.method obj=arguments.sharedObj {
                thread.res = hammer(attributes.obj, attributes.method, attributes.tag, ITERS);
            }
        } else {
            thread name=tname action="run" tag="t#t#" method=arguments.method {
                thread.res = hammer(newFixture(), attributes.method, attributes.tag, ITERS);
            }
        }
    }
    for (t = 1; t <= arrayLen(names); t++) {
        thread action="join" name=names[t] timeout=60000;
        res = cfthread[names[t]].res ?: { errors: ITERS, wrong: 0, firstError: "thread did not complete: " & (cfthread[names[t]].error.message ?: "no result") };
        total.errors += res.errors;
        total.wrong  += res.wrong;
        if (total.firstError == "" && res.firstError != "") { total.firstError = res.firstError; }
    }
    return total;
}

driverAvailable = true;
try { queryExecute("SELECT 1 AS ok", [], { datasource: memDs }); }
catch (any e) { driverAvailable = false; }

if (!driverAvailable) {
    assertTrue("shared-instance cfqueryparam legs skipped (no SQLite JDBC driver on this engine)", true);
} else {

shared = newFixture();

// ---- 0. Sequential control: the helper shape is fine when nobody overlaps ----
seq = hammer(shared, "selectViaHelper", "seq", ITERS);
assert("sequential: helper-built params on one instance bind correctly (" & ITERS & " calls, errors)", seq.errors, 0);
assert("sequential: helper-built params on one instance bind correctly (wrong rows)", seq.wrong, 0);

// ---- 1. THE GAP: shared instance, method call inside the query body ---------
gap = fanOut("selectViaHelper", shared);
assert("shared instance + method call inside query body: " & THREADS & " threads x " & ITERS & " — no bind-count errors [first: " & gap.firstError & "]", gap.errors, 0);
assert("shared instance + method call inside query body: no row carries another thread's bound values", gap.wrong, 0);

// ---- 2. Control: same instance, same placeholders, inline params -------------
inline = fanOut("selectInline", shared);
assert("shared instance, inline cfqueryparams only: no bind-count errors [first: " & inline.firstError & "]", inline.errors, 0);
assert("shared instance, inline cfqueryparams only: no crossed rows", inline.wrong, 0);

// ---- 3. Control: helper shape, but one instance PER THREAD -------------------
perThread = fanOut("selectViaHelper");
assert("per-thread instances + method call inside query body: no bind-count errors [first: " & perThread.firstError & "]", perThread.errors, 0);
assert("per-thread instances + method call inside query body: no crossed rows", perThread.wrong, 0);

} // driverAvailable

suiteEnd();
</cfscript>
