<cfscript>
suiteBegin("Tags: cfqueryparam script-statement form inside a script cfquery");

// ============================================================
// Background
// ============================================================
// Inside a script-syntax cfquery(){} block, cfqueryparam is callable as a
// script statement — both the positional/named form
//   cfqueryparam(value = v, cfsqltype = "cf_sql_integer");
// and the attributeCollection form
//   cfqueryparam(attributeCollection = qp);
// must bind a parameter, exactly like the <cfqueryparam> TAG inside a
// <cfquery> tag. Lucee, Adobe CF, and BoxLang all accept the script form.
//
// RustCFML 0.144.0 does not lower the script-statement cfqueryparam: the
// call site is treated as an undefined identifier and throws
//   Variable 'cfqueryparam' is undefined
// for every form, while the TAG form <cfqueryparam ...> works (control below)
// and queryExecute() with a named-param struct works (control below).
//
// Why it matters for Wheels: vendor/wheels/databaseAdapters/Base.cfc emits
// exactly this shape — a script cfquery(){} whose body calls
// cfqueryParam(attributeCollection = qp) for every bound value — on the
// INSERT (create.cfc), UPDATE (update.cfc), soft-delete, and every
// parameterized WHERE (findOne/findAll with a bind). On RustCFML this throws
// before any write or parameterized read completes, so the ORM persistence
// layer is fully blocked even though the framework boots and renders.
// ============================================================

cfqpsfQ = queryNew("id,name", "integer,varchar", [{id: 1, name: "alpha"}, {id: 2, name: "beta"}]);

// --- CONTROL (green on both engines): queryExecute named-param struct binds ---
cfqpsfQe = queryExecute("SELECT name FROM cfqpsfQ WHERE id = :tid",
    {tid: {value: 2, cfsqltype: "cf_sql_integer"}}, {dbtype: "query"});
assert("CONTROL: queryExecute named-param struct binds", cfqpsfQe.name, "beta");

// --- the gap: positional script cfqueryparam inside a script cfquery ---
cfqpsfPosVal = "(threw)";
try {
    cfquery(name = "local.cfqpsfR1", dbtype = "query") {
        writeOutput("SELECT name FROM cfqpsfQ WHERE id = ");
        cfqueryparam(value = 2, cfsqltype = "cf_sql_integer");
    }
    cfqpsfPosVal = local.cfqpsfR1.name;
} catch (any e) {
    cfqpsfPosVal = "THREW: " & e.message;
}
assert("positional cfqueryparam(value=, cfsqltype=) binds inside script cfquery",
    cfqpsfPosVal, "beta");

// --- the gap: attributeCollection script cfqueryparam (the Wheels Base.cfc shape) ---
cfqpsfAcVal = "(threw)";
try {
    cfqpsfQp = {value: 1, cfsqltype: "cf_sql_integer"};
    cfquery(name = "local.cfqpsfR2", dbtype = "query") {
        writeOutput("SELECT name FROM cfqpsfQ WHERE id = ");
        cfqueryparam(attributeCollection = cfqpsfQp);
    }
    cfqpsfAcVal = local.cfqpsfR2.name;
} catch (any e) {
    cfqpsfAcVal = "THREW: " & e.message;
}
assert("attributeCollection cfqueryparam binds inside script cfquery (Wheels Base.cfc shape)",
    cfqpsfAcVal, "alpha");

suiteEnd();
</cfscript>
