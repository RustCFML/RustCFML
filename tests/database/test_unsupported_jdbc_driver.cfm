<cfscript>
suiteBegin("A datasource for an unbundled JDBC driver throws instead of opening SQLite");

// rc_app_h2 (tests/Application.cfc) is declared the Lucee/ACF way:
//   { class: "org.h2.Driver", connectionString: "jdbc:h2:file:<tmp>/rc_app_h2/test;MODE=MySQL" }
// RustCFML bundles SQLite, MySQL, PostgreSQL and SQL Server drivers, not H2.
// Today the datasource falls through to the sqlite catch-all: the whole JDBC URL
// is opened as a SQLite database FILE (a path named "jdbc:h2:file:/...;MODE=MySQL"
// relative to the working directory), and queries silently run against that
// empty database. A datasource the engine cannot serve must throw, as
// rc_app_bad does for an unreachable server (config/test_app_datasources.cfm).

assertThrows(
	"a query on an org.h2.Driver datasource throws",
	function() {
		queryExecute( "SELECT 1 AS n", [], { datasource: "rc_app_h2" } );
	}
);

suiteEnd();
</cfscript>
