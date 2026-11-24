<cfscript>
// GH #544. Inside a query loop (script `loop query`, the cfloop and
// cfoutput tags with query=, grouped or not) the query's columns and the
// pseudo-columns currentRow / recordCount / columnList are readable without a
// prefix. Lucee keeps a query stack for the request: a bare name is looked up
// in local, then arguments, then the running loops' queries (innermost
// first), then variables. Functions the body calls see the loop's columns
// too. Nothing is written to `variables`.
//
// Queries are references, so nested loops over the same query share its
// cursor; each loop puts the cursor back when it ends, which hands an outer
// loop its own row again.
//
// Every expectation was measured on Lucee 7.1.2.191 (2026-11-24).
suiteBegin("Query loops: bare columns and pseudo-columns (GH ##544)");

q = queryNew( "id,label,extra", "integer,varchar,varchar", [ [ 1, "a", "e1" ], [ 2, "b", "e2" ], [ 3, "c", "e3" ] ] );

// --- the script form, which did not resolve bare names at all ---
seen = "";
loop query="q" { seen &= label & currentRow & recordCount & ";"; }
assert( "script loop: bare column, currentRow and recordCount", seen, "a13;b23;c33;" );
cl = "";
loop query="q" { cl = columnList; }
assert( "script loop: bare columnList", cl, "ID,LABEL,EXTRA" );
seen = "";
cfloop( query="q" ) { seen &= label; }
assert( "script cfloop(): bare column", seen, "abc" );

// --- precedence: local and arguments first, then the query, then variables ---
label = "PAGE";
currentRow = "PAGECR";
seen = "";
loop query="q" { seen &= label & currentRow; }
assert( "the query outranks a variable of the same name", seen, "a1b2c3" );
assert( "and leaves that variable as it was", label & currentRow, "PAGEPAGECR" );
structDelete( variables, "label" );
structDelete( variables, "currentRow" );

function withLocal( q ) {
	var label = "LOCAL";
	var out = "";
	loop query="arguments.q" { out &= label & currentRow; }
	return out;
}
assert( "a local outranks the query", withLocal( q ), "LOCAL1LOCAL2LOCAL3" );

function withArg( q, label ) {
	var out = "";
	loop query="arguments.q" { out &= label; }
	return out;
}
assert( "an argument outranks the query", withArg( q, "ARG" ), "ARGARGARG" );

loop query="q" { }
assert( "no column is left defined after the loop", isDefined( "label" ) || isDefined( "extra" ), false );
assert( "no pseudo-column is left defined after the loop", isDefined( "currentRow" ) || isDefined( "recordCount" ), false );

function returnsFromLoop( q ) {
	loop query="arguments.q" { return label; }
}
returnsFromLoop( q );
assert( "a return from inside the loop leaves nothing defined", isDefined( "label" ), false );

seen = "";
loop query="q" { seen &= isDefined( "label" ) & isDefined( "currentRow" ) & ";"; }
assert( "isDefined sees the loop's names", seen, "truetrue;truetrue;truetrue;" );

seen = "";
loop query="q" { label = "SET" & currentRow; }
assert( "assigning a bare column name writes variables, not the query", ( variables.label ?: "" ) & "/" & valueList( q.label ), "SET3/a,b,c" );
structDelete( variables, "label" );

// --- the bare names in the shapes the compiler fuses ---
function compareInFunction( q ) {
	loop query="arguments.q" { if ( currentRow == 2 ) return "row" & currentRow; }
	return "none";
}
assert( "compared with a constant in a function", compareInFunction( q ), "row2" );
seen = "";
loop query="q" { if ( id > 1 ) seen &= id; }
assert( "a column compared with a constant", seen, "23" );
seen = "";
loop query="q" { seen &= label.len() & uCase( label ); }
assert( "a member call on a bare column", seen, "1A1B1C" );
sq = queryNew( "info", "varchar", [ [ "x" ] ] );
querySetCell( sq, "info", { k: "v" }, 1 );
seen = "";
loop query="sq" { seen = info.k; }
assert( "a property read on a bare column holding a struct", seen, "v" );

// --- functions called from inside the loop ---
function readsBare() {
	try { return label & currentRow; } catch ( any e ) { return "UNDEFINED"; }
}
seen = "";
loop query="q" { seen &= readsBare() & ";"; }
assert( "a called function sees the loop's columns", seen, "a1;b2;c3;" );

svc = createObject( "component", "QueryLoopScopeService" );
seen = "";
loop query="q" { seen &= svc.readLabel() & svc.readExtra() & ";"; }
assert( "a component method sees the loop's column over its own variables", seen, "ae1;be2;ce3;" );
assert( "outside a loop the component reads its own variables", svc.readLabel() & svc.readExtra(), "SERVICEUNDEFINED" );
assert( "a loop in a method writes nothing to the component", svc.leaves(), "ae1ae1|SERVICE|falsefalse" );

// --- the issue's real-world shape: querySetCell(..., currentRow) in a method ---
tq = queryNew( "label", "varchar", [ [ "x" ], [ "y" ] ] );
svc.translate( tq );
assert( "querySetCell with a bare currentRow, on a query passed by reference", valueList( tq.label ), "X1,Y2" );

// --- by reference: nested loops over the same query ---
seen = "";
loop query="q" {
	inner = "";
	loop query="q" { inner &= currentRow; }
	seen &= label & currentRow & q.currentRow & "(" & inner & ");";
}
assert( "an inner loop over the same query hands the outer its row back", seen, "a11(123);b22(123);c33(123);" );
seen = "";
loop query="q" { seen &= svc.rowsOf( q ) & label & currentRow & ";"; }
assert( "so does a loop over it in a called method", seen, "123a1;123b2;123c3;" );

// --- the query changing under the loop ---
mq = duplicate( q );
seen = "";
loop query="mq" { querySetCell( mq, "label", "Z" & currentRow, currentRow ); seen &= label; }
assert( "a cell set in the loop is read back in the loop", seen, "Z1Z2Z3" );
gq = queryNew( "v", "integer", [ [ 1 ], [ 2 ] ] );
seen = "";
loop query="gq" { if ( recordCount < 4 ) queryAddRow( gq, { v: 9 } ); seen &= v & "/" & recordCount & ";"; }
assert( "rows added in the loop: recordCount grows, the row count looped does not", seen, "1/3;2/4;" );
rq = queryNew( "v", "integer", [ [ 1 ], [ 2 ], [ 3 ] ] );
seen = "";
loop query="rq" { seen &= v; rq = queryNew( "v", "integer", [ [ 100 ] ] ); }
assert( "reassigning the query variable: the loop runs on over the original", seen, "123" );
assert( "and the variable keeps its new value", rq.recordCount, 1 );

// --- cell values ---
nq = queryNew( "id,label", "integer,varchar", [ [ 1, nullValue() ] ] );
seen = "";
loop query="nq" { seen = "[" & label & "]" & isNull( label ); }
assert( "a NULL cell reads as an empty string", seen, "[]false" );
cq = queryNew( "currentRow,x", "integer,varchar", [ [ 99, "y" ] ] );
seen = "";
loop query="cq" { seen = currentRow; }
assert( "a real column named currentRow outranks the pseudo-column", seen, 99 );

// --- nested loops over different queries ---
q2 = queryNew( "other", "varchar", [ [ "z" ] ] );
seen = "";
loop query="q" { loop query="q2" { seen &= label & other & currentRow & ";"; } }
assert( "an outer loop's columns stay visible in an inner loop", seen, "az1;bz1;cz1;" );

// --- query of queries inside the loop ---
seen = "";
loop query="q" { qq = queryExecute( "select label from q where id > 1", {}, { dbtype: "query" } ); seen &= label & qq.recordCount & ";"; }
assert( "a query of queries over the looping query", seen, "a2;b2;c2;" );
// A loop column named like the table hides an UNSCOPED table name on Lucee
// (its query of queries resolves tables through the same cascade, finds the
// column's string and throws). RustCFML resolves the table: see
// docs/known-issues.md §13. The scoped name works on both.
tbl = queryNew( "x", "integer", [ [ 7 ], [ 8 ] ] );
clash = queryNew( "id,tbl", "integer,varchar", [ [ 1, "n1" ] ] );
seen = "";
loop query="clash" { seen = queryExecute( "select x from variables.tbl", {}, { dbtype: "query" } ).recordCount; }
assert( "a scoped table name is unaffected by a same-named column", seen, 2 );
if ( isRustCFML() ) {
	loop query="clash" { seen = queryExecute( "select x from tbl", {}, { dbtype: "query" } ).recordCount; }
	assert( "RustCFML: an unscoped table name still finds the table", seen, 2 );
}

// --- closures created in the loop do not keep its scope ---
fns = [];
loop query="q" { arrayAppend( fns, function() { try { return label; } catch ( any e ) { return "UNDEFINED"; } } ); }
f1 = fns[ 1 ];
assert( "a closure called after the loop does not see its columns", f1(), "UNDEFINED" );
</cfscript>

<!--- Tag forms: the same scope, and nothing copied into variables. --->
<cfset gq = queryNew( "dept,name,n", "varchar,varchar,integer", [ ["A","x",1],["A","y",2],["B","z",3],["B","w",4],["C","v",5] ] )>
<cfset name = "PAGE">
<cfsavecontent variable="out"><cfoutput query="gq">#name##currentRow#</cfoutput></cfsavecontent>
<cfset assert("cfoutput query: bare column and currentRow", out, "x1y2z3w4v5")>
<cfsavecontent variable="out"><cfloop query="gq"><cfoutput>#name##currentRow#</cfoutput></cfloop></cfsavecontent>
<cfset assert("cfloop query: bare column and currentRow", out, "x1y2z3w4v5")>
<cfsavecontent variable="out"><cfoutput query="gq" group="dept">[#dept# #currentRow#:<cfoutput>#name##currentRow#,</cfoutput>]</cfoutput></cfsavecontent>
<cfset assert("grouped cfoutput: group and detail rows", out, "[A 1:x1,y2,][B 3:z3,w4,][C 5:v5,]")>
<cfsavecontent variable="out"><cfloop query="gq" group="dept"><cfoutput>{#dept#</cfoutput><cfloop><cfoutput>#name##n#</cfoutput></cfloop>}</cfloop></cfsavecontent>
<cfset assert("grouped cfloop: group and detail rows", out, "{Ax1y2}{Bz3w4}{Cv5}")>
<cfsavecontent variable="out"><cfloop query="gq" group="dept" startrow="2" endrow="4"><cfoutput>(#dept#:#currentRow#)</cfoutput></cfloop></cfsavecontent>
<cfset assert("grouped cfloop with a row window", out, "(A:2)(B:3)")>
<cfsavecontent variable="out"><cfoutput query="gq" group="dept">#dept#<cfoutput group="name">#name#</cfoutput>|</cfoutput></cfsavecontent>
<cfset assert("two group levels", out, "Axy|Bzw|Cv|")>
<cfsavecontent variable="out"><cfoutput query="gq" group="dept">#dept#=#gq.name#;</cfoutput></cfsavecontent>
<cfset assert("grouped cfoutput: q.col reads the group's first row", out, "A=x;B=z;C=v;")>
<cfset assert("after the tag loops the page variable is unchanged", name, "PAGE")>
<cfset assert("and no column is left defined", isDefined("dept") OR isDefined("n"), false)>
<cfsavecontent variable="out"><cfoutput query="gq">#name#-<cfloop query="gq">#currentRow#</cfloop>-#name##currentRow#;</cfoutput></cfsavecontent>
<cfset assert("cfoutput: an inner cfloop over the same query hands the row back", out, "x-12345-x1;y-12345-y2;z-12345-z3;w-12345-w4;v-12345-v5;")>

<!--- Leaving a loop early by any route takes its query off the scope. --->
<cf_query_loop_scope_exit>
<cfset assert("a custom tag left with cfexit mid-loop leaves nothing defined", isDefined("exitcol"), false)>
<cfscript>
function throwsFromLoop() {
	var tq2 = queryNew( "thrown", "varchar", [ [ "a" ] ] );
	loop query="tq2" { throw( "boom" ); }
}
try { throwsFromLoop(); } catch ( any e ) {}
assert( "an error thrown out of a loop leaves nothing defined", isDefined( "thrown" ), false );
</cfscript>

<cfscript>
suiteEnd();
</cfscript>
