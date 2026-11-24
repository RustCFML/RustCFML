<cfscript>
// GH #539. `<cfdbinfo type="columns">` on SQL Server read INFORMATION_SCHEMA,
// which has no identity or computed flag and no length for a `(max)` column,
// and whose NULL NUMERIC_PRECISION gave every int, decimal, date and bit
// column COLUMN_SIZE 0 — the value schema-sync code compares. Lucee reports
// mssql-jdbc's getColumns: the `sp_columns_100` catalogue procedure plus the
// driver's fix-ups, and so does RustCFML now.
//
// Every expectation below was measured against Lucee 7.1.2.191 + mssql-jdbc
// on Azure SQL Edge.
//
// Live test, gated on RUSTCFML_TEST_MSSQL_DS. To run locally:
//   docker run --rm -d -p 1433:1433 -e ACCEPT_EULA=1 \
//     -e MSSQL_SA_PASSWORD='Str0ng!Passw0rd' mcr.microsoft.com/azure-sql-edge
//   RUSTCFML_TEST_MSSQL_DS='mssql://sa:Str0ng!Passw0rd@127.0.0.1:1433/master?trustServerCertificate=true' \
//     cargo run -- tests/runner.cfm

suiteBegin("cfdbinfo SQL Server columns (skipped without RUSTCFML_TEST_MSSQL_DS)");

msdsn = "";
try {
	msdsn = server.system.environment.RUSTCFML_TEST_MSSQL_DS ?: "";
} catch (any e) {
	msdsn = "";
}

if (msdsn == "") {
	writeOutput("  (skipped - set RUSTCFML_TEST_MSSQL_DS to enable)" & chr(10));
	assertTrue("suite skipped without a SQL Server datasource", true);
} else {
	tbl = "zz_rcfml_dbinfo_" & lCase(left(replace(createUUID(), "-", "", "all"), 8));
	try {
		queryExecute(
			"CREATE TABLE #tbl# ( id int IDENTITY(1,1) PRIMARY KEY, c_vc varchar(20),
			   c_nvc nvarchar(50), c_nvmax nvarchar(max), c_vbmax varbinary(max),
			   c_dec decimal(10,2), c_dt2 datetime2, c_bit bit, c_comp AS (id * 2),
			   c_flt float, c_uid uniqueidentifier, c_nchar nchar(5), c_time time,
			   c_geo geography, c_xml xml )",
			[], { datasource: msdsn } );

		dbinfo type="columns" datasource="#msdsn#" table="#tbl#" name="cols";
		c = {};
		for (r in cols) {
			c[ lCase(r.column_name) ] = r;
		}

		// The two the issue singled out for schema-sync code.
		assert("an int has COLUMN_SIZE 10", c.id.column_size, 10);
		assert("an identity column is IS_AUTOINCREMENT", c.id.is_autoincrement, "YES");
		assert("an identity column's TYPE_NAME", c.id.type_name, "int identity");
		assert("a plain column is not IS_AUTOINCREMENT", c.c_vc.is_autoincrement, "NO");
		assert("a computed column is IS_GENERATEDCOLUMN", c.c_comp.is_generatedcolumn, "YES");
		assert("decimal(10,2) size", c.c_dec.column_size, 10);
		assert("decimal(10,2) scale", c.c_dec.decimal_digits, 2);
		assert("a bit has COLUMN_SIZE 1", c.c_bit.column_size, 1);
		assert("datetime2 size", c.c_dt2.column_size, 27);
		assert("datetime2 scale", c.c_dt2.decimal_digits, 7);

		// A (max) column: Integer.MAX_VALUE, not 0 or -1.
		assert("nvarchar(max) is Types.NVARCHAR", c.c_nvmax.data_type, -9);
		assert("nvarchar(max) size", c.c_nvmax.column_size, 2147483647);
		assert("nvarchar(max) octet length", c.c_nvmax.char_octet_length, 2147483647);
		assert("varbinary(max) size", c.c_vbmax.column_size, 2147483647);

		// mssql-jdbc's ODBC → JDBC type rewrites.
		assert("nvarchar is Types.NVARCHAR", c.c_nvc.data_type, -9);
		assert("nchar is Types.NCHAR", c.c_nchar.data_type, -15);
		assert("float is Types.DOUBLE", c.c_flt.data_type, 8);
		assert("uniqueidentifier is Types.CHAR", c.c_uid.data_type, 1);
		assert("time is Types.TIME", c.c_time.data_type, 92);
		assert("xml is Types.LONGNVARCHAR", c.c_xml.data_type, -16);
		assert("a CLR type is Types.VARBINARY", c.c_geo.data_type, -3);

		// The catalogue's own values come through.
		assert("nvarchar(50) buffer length is in bytes", c.c_nvc.buffer_length, 100);
		assert("nvarchar(50) octet length is in bytes", c.c_nvc.char_octet_length, 100);
		assert("SQL_DATA_TYPE keeps the ODBC code", c.c_nvc.sql_data_type, -9);
		assert("SOURCE_DATA_TYPE is the TDS type of an int", c.id.source_data_type, 56);
		assert("SOURCE_DATA_TYPE is the TDS type of a varchar", c.c_vc.source_data_type, 39);
		assert("NUM_PREC_RADIX is NULL for a string", c.c_vc.num_prec_radix, "");
		assert("NUM_PREC_RADIX is 2 for a float", c.c_flt.num_prec_radix, 2);
		assert("CHAR_OCTET_LENGTH is NULL for an int", c.id.char_octet_length, "");

		// mssql-jdbc's SS_* columns sit between IS_GENERATEDCOLUMN and the
		// IS_PRIMARYKEY enrichment, in columns_minimal too.
		names = [];
		for (m in getMetadata(cols)) names.append(m.name);
		gen = arrayFindNoCase(names, "IS_GENERATEDCOLUMN");
		assert("SS_IS_SPARSE follows IS_GENERATEDCOLUMN", names[gen + 1] ?: "", "SS_IS_SPARSE");
		assert("SS_XML_SCHEMACOLLECTION_NAME precedes IS_PRIMARYKEY",
			names[arrayFindNoCase(names, "IS_PRIMARYKEY") - 1] ?: "", "SS_XML_SCHEMACOLLECTION_NAME");
		assert("SS_IS_SPARSE is 0 for a plain column", c.c_vc.ss_is_sparse, 0);
		dbinfo type="columns_minimal" datasource="#msdsn#" table="#tbl#" name="cm";
		minimal = [];
		for (m in getMetadata(cm)) minimal.append(m.name);
		assert("columns_minimal ends with the SS_* columns", minimal[arrayLen(minimal)], "SS_XML_SCHEMACOLLECTION_NAME");
		assert("columns_minimal has 32 columns", arrayLen(minimal), 32);

		queryExecute("DROP TABLE #tbl#", [], { datasource: msdsn });
	} catch (any e) {
		try { queryExecute("DROP TABLE #tbl#", [], { datasource: msdsn }); } catch (any e2) {}
		rethrow;
	}
}

suiteEnd();
</cfscript>
