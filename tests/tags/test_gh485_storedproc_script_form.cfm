<cfscript>
suiteBegin("Script-form storedproc with procparam/procresult (GH 485)");

// `storedproc procedure=… { procparam cfsqltype=… value=… null=false; }` written
// in cfscript was a PARSE ERROR ("Expected RBrace, found Identifier"):
// `storedproc` was not a recognised script tag, so the name degraded to a bare
// expression statement, its attributes became stray assignments in the caller's
// scope, and the body block was then read as a struct literal. Lucee parses the
// form, runs the body at runtime, and leaks nothing.
//
// Two further details this covers, both measured on Lucee 7.1:
//   - `null` is a legal ATTRIBUTE NAME here, though it lexes as a literal, so
//     every attribute loop used to stop dead at `null=false`;
//   - a `procparam` under `if`/`for` must be collected when the body RUNS, not
//     by a static scan of it.

// ── cross-engine: the body is executed, and nothing leaks ──────────────────
// No `datasource` attribute, deliberately: Lucee validates a NAMED datasource
// before it runs the body, so a deliberately-bad name would prove nothing about
// the body there. With the attribute absent both engines run the body first and
// then complain, which is the behaviour under test.
request._gh485_seen = "";
request._gh485_err  = "";
try {
	storedproc procedure="gh485Proc" {
		procresult name="gh485Out";
		for ( v in [ 10, 20, 30 ] ) {
			request._gh485_seen = v;
			procparam cfsqltype="integer" value=v null=false;
		}
	}
} catch ( any e ) {
	request._gh485_err = e.message;
}

assert("a procparam under control flow is collected when the body RUNS",
	request._gh485_seen, 30);
assertTrue("a call with no resolvable datasource throws rather than parsing wrong",
	len( request._gh485_err ) GT 0);
assert("an attribute name does not leak into the caller's scope",
	structKeyExists( variables, "procedure" ), false);
assert("nor does the datasource attribute",
	structKeyExists( variables, "datasource" ), false);
assert("nor the compiler's procresult marker",
	structKeyExists( variables, "__cfproc_result_name" ), false);
assert("nor the runtime parameter array",
	structKeyExists( variables, "__cfproc_params" ), false);

structDelete( request, "_gh485_seen" );
structDelete( request, "_gh485_err" );

// ── the parenthesised and cf-prefixed spellings parse too ─────────────────
gh485ParenOk = true;
try {
	cfstoredproc( procedure="gh485Proc2", datasource="gh485_no_such_datasource" ) {
		cfprocparam( cfsqltype="varchar", value="a" );
		cfprocresult( name="gh485Out2" );
	}
} catch ( any e ) {
	// the datasource is expected to fail; a PARSE failure would not reach here
}
assertTrue("cfstoredproc( … ) { cfprocparam( … ); } parses", gh485ParenOk);

// ── RustCFML only: the exact SQL the form lowers to ───────────────────────
// Asserted through SQLite's error text, which echoes the statement back. This
// is what pins the placeholder count to the number of procparams — the part a
// silent mis-lowering would get wrong. Lucee ships no SQLite JDBC driver and
// words its errors differently, so this leg is ours alone; the behaviour above
// is the cross-engine contract.
if ( isRustCFML() ) {
	dbFile = getTempDirectory() & "/rustcfml_gh485_" & createUUID() & ".db";
	sqliteDs = { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite:" & dbFile };

	sawTwo = "";
	try {
		storedproc procedure="gh485Two" datasource=sqliteDs {
			procparam cfsqltype="char" value="A" null=false;
			procparam cfsqltype="char" value="B";
		}
	} catch ( any e ) { sawTwo = e.message; }
	assertTrue("two procparams lower to CALL gh485Two(?,?) (saw: " & sawTwo & ")",
		findNoCase( "CALL gh485Two(?,?)", sawTwo ) GT 0);

	sawNone = "";
	try {
		storedproc procedure="gh485None" datasource=sqliteDs { }
	} catch ( any e ) { sawNone = e.message; }
	assertTrue("an empty body lowers to CALL gh485None() with no placeholders (saw: " & sawNone & ")",
		findNoCase( "CALL gh485None()", sawNone ) GT 0);

	// A procresult does NOT contribute a placeholder — it names the result.
	sawOne = "";
	try {
		storedproc procedure="gh485One" datasource=sqliteDs {
			procresult name="gh485Rs";
			procparam cfsqltype="integer" value=7;
		}
	} catch ( any e ) { sawOne = e.message; }
	assertTrue("procresult adds no placeholder: CALL gh485One(?) (saw: " & sawOne & ")",
		findNoCase( "CALL gh485One(?)", sawOne ) GT 0);

	// The tag form is the reference: it must lower to the very same statement.
	sawTag = "";
	try {
		request._gh485_tagDs = sqliteDs;
		include "gh485_storedproc_tag_target.cfm";
	} catch ( any e ) { sawTag = e.message; }
	assert("the tag form lowers to the same statement as the script form",
		findNoCase( "CALL gh485Two(?,?)", sawTag ) GT 0, true);
	structDelete( request, "_gh485_tagDs" );
}

suiteEnd();
</cfscript>
