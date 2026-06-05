<cfscript>
suiteBegin("Tags: Script cfquery statement body");

// The script-form cfquery(...) { ... } body may contain CFML statements
// (assignments, loops, conditionals) interleaved with SQL and cfqueryparam.
// This is the standard idiom for building dynamic queries on Lucee, Adobe CF
// 2018+, and BoxLang — e.g. Wheels' database adapter builds every query this
// way (databaseAdapters/Base.cfc).
//
// The body is kept inside a never-called function so the test needs no
// datasource: we are only verifying that the statement body PARSES. Reaching
// the assertion below means the file compiled. (The sibling
// test_tags_script_syntax_body.cfm covers cfsavecontent/cflock/transaction;
// this covers the cfquery case. The same parse behaviour applies to a script
// cfhttp(...) { ... } body.)

function buildDynamicQuery() {
    cfquery(name = "q") {
        local.sql = "";
        for (local.i = 1; local.i <= 3; local.i++) {
            if (local.i > 1) {
                local.sql &= " UNION ";
            }
            local.sql &= "SELECT " & local.i;
        }
        writeOutput(local.sql);
    }
}

assertTrue(
    "script cfquery(...) with a statement body parses",
    isCustomFunction(buildDynamicQuery)
);

suiteEnd();
</cfscript>
