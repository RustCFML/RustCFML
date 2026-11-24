<cfscript>
suiteBegin("queryExecute params: list=true expansion, named params on every driver, :: escaping (Lucee 7.1 parity)");

// Expected values were produced by Lucee 7.1.2.191 against PostgreSQL 18,
// MySQL 8.4 and SQL Server (Azure SQL Edge), 2026-11-15.
//
// - GH #511: a NAMED list=true param expands to one placeholder per element
//   (PostgreSQL bound it as one string).
// - GH #512: empty list elements are dropped ("/1/3/" with separator "/" binds
//   two values).
// - SQL Server ignored named params altogether and sent `:name` to the server.
// - GH #513: with params, `::` outside quotes is an escaped `:` — a cast is
//   written `::::`. Without params the SQL is sent as written.
//
// SQLite runs every time; set RUSTCFML_TEST_PG_URL, RUSTCFML_TEST_MYSQL_DS or
// RUSTCFML_TEST_MSSQL_DS to run against a live server.

function qpEnv( required string name ) {
	try {
		return server.system.environment[ arguments.name ] ?: "";
	} catch ( any e ) {
		return "";
	}
}

qpTargets = [ { name: "sqlite", dialect: "sqlite", ds: { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite:" & getTempDirectory() & "rcfml_qp_" & getTickCount() & ".db" } } ];
if ( len( qpEnv( "RUSTCFML_TEST_PG_URL" ) ) ) arrayAppend( qpTargets, { name: "postgres", dialect: "postgres", ds: qpEnv( "RUSTCFML_TEST_PG_URL" ) } );
if ( len( qpEnv( "RUSTCFML_TEST_MYSQL_DS" ) ) ) arrayAppend( qpTargets, { name: "mysql", dialect: "mysql", ds: qpEnv( "RUSTCFML_TEST_MYSQL_DS" ) } );
if ( len( qpEnv( "RUSTCFML_TEST_MSSQL_DS" ) ) ) arrayAppend( qpTargets, { name: "mssql", dialect: "mssql", ds: qpEnv( "RUSTCFML_TEST_MSSQL_DS" ) } );

qpTable = "zz_rcfml_qp_" & lCase( left( replace( createUUID(), "-", "", "all" ), 8 ) );

function qpRun( required string sql, any params ) {
	try {
		var q = queryExecute( arguments.sql, arguments.params, { datasource: qpDs } );
		return valueList( q.v );
	} catch ( any e ) {
		return "ERROR " & e.message;
	}
}

for ( target in qpTargets ) {
	qpDs = target.ds;
	try {
		queryExecute( "create table #qpTable# ( v int, s varchar(10) )", [], { datasource: qpDs } );
	} catch ( any e ) {
		assertTrue( "#target.name#: skipped, could not create the table: " & e.message, true );
		continue;
	}
	try {
		for ( i = 1; i <= 4; i++ ) {
			queryExecute( "insert into #qpTable# values ( #i#, 'k#i#' )", [], { datasource: qpDs } );
		}
		sel = "select v from #qpTable#";
		assert( "#target.name#: named int list", qpRun( "#sel# where v in (:ids) order by v", { ids: { value: "1,3", cfsqltype: "integer", list: true } } ), "1,3" );
		assert( "#target.name#: named varchar list", qpRun( "#sel# where s in (:ids) order by v", { ids: { value: "k2,k4", cfsqltype: "varchar", list: true } } ), "2,4" );
		assert( "#target.name#: a named list used twice", qpRun( "#sel# where v in (:ids) or v in (:ids) order by v", { ids: { value: "2,3", cfsqltype: "integer", list: true } } ), "2,3" );
		assert( "#target.name#: positional list, separator / with empty ends", qpRun( "#sel# where v in (?) order by v", [ { value: "/1/3/", cfsqltype: "integer", list: true, separator: "/" } ] ), "1,3" );
		assert( "#target.name#: named list with an empty element", qpRun( "#sel# where v in (:ids) order by v", { ids: { value: "1,,3", cfsqltype: "integer", list: true } } ), "1,3" );
		assert( "#target.name#: list elements are trimmed", qpRun( "#sel# where v in (:ids) order by v", { ids: { value: " 1 , 2 ", cfsqltype: "integer", list: true } } ), "1,2" );
		assert( "#target.name#: an array value is one bind per element", qpRun( "#sel# where v in (:ids) order by v", { ids: { value: [ 1, 4 ], cfsqltype: "integer", list: true } } ), "1,4" );
		assert( "#target.name#: a named scalar", qpRun( "#sel# where v = :x", { x: { value: 3, cfsqltype: "integer" } } ), "3" );
		assert( "#target.name#: a plain named value", qpRun( "#sel# where v = :x", { x: 2 } ), "2" );
		assert( "#target.name#: a camelCase name", qpRun( "#sel# where v = :someValue", { someValue: 4 } ), "4" );
		// A positional array may mix plain values and param structs; each
		// element is read on its own. Only a LEADING struct used to switch
		// struct handling on, so a later one was bound as its own text
		// ("{value: k3, …}") and its list=true was never expanded. Lucee
		// 7.1.2.191 on MySQL 8.4 gives these results (on PostgreSQL it binds
		// an untyped number as text, so `v = ?` with a plain 1 errors there).
		assert( "#target.name#: a plain value, then a param struct", qpRun( "#sel# where v = ? or s = ? order by v", [ 1, { value: "k3", cfsqltype: "varchar" } ] ), "1,3" );
		assert( "#target.name#: a plain value, then a list param", qpRun( "#sel# where v = ? or v in (?) order by v", [ 1, { value: "2,4", cfsqltype: "integer", list: true } ] ), "1,2,4" );
		assert( "#target.name#: a param struct, then a plain value", qpRun( "#sel# where v = ? or s = ? order by v", [ { value: 1, cfsqltype: "integer" }, "k3" ] ), "1,3" );
		// `::` inside a quoted string is never touched.
		assert( "#target.name#: :: inside quotes with params", qpRun( "select 'a::::b' as v", [] ), "a::::b" );

		if ( target.dialect == "postgres" ) {
			assert( "postgres: :::: is a cast with array params", qpRun( "select '12'::::int as v", [] ), "12" );
			assert( "postgres: :::: is a cast with struct params", qpRun( "select '12'::::int as v", {} ), "12" );
			assert( "postgres: :::: after a named param", qpRun( "select :x::::int as v", { x: { value: "12", cfsqltype: "varchar" } } ), "12" );
			assert( "postgres: :::: after a positional param", qpRun( "select ?::::int as v", [ "12" ] ), "12" );
			assert( "postgres: :: with params is a single colon, as on Lucee", left( qpRun( "select '12'::int as v", [] ), 5 ), "ERROR" );
			assert( "postgres: :: without params is sent as written",
				queryExecute( "select '12'::int as v", nullValue(), { datasource: qpDs } ).v, 12 );
			assert( "postgres: :::: in a double-quoted identifier is left alone",
				queryExecute( 'select 12 as "a::::b"', [], { datasource: qpDs } ).columnList, "a::::b" );
		}
		if ( target.dialect == "mysql" ) {
			assert( "mysql: := is not a param", qpRun( "select @rcfml_qp := 12 as v", [] ), "12" );
		}
	} finally {
		try { queryExecute( "drop table #qpTable#", [], { datasource: qpDs } ); } catch ( any e ) {}
	}
}

suiteEnd();
</cfscript>
