<cfscript>
// GH #390 (create path) and #391 (alter path). `<cfdbinfo type="columns">` on
// MySQL/MariaDB reported a `tinyint(1)` — which is also how both servers store
// `BOOLEAN` — as type_name "tinyint". Lucee reports "BIT", because MySQL
// Connector/J's `tinyInt1isBit` defaults to true and presents TINYINT(1) as
// JDBC BIT. Preside's PresideObjectService.dbSync() compares each property's
// expected DB type against this value, so every boolean column looked changed
// on every sync.
//
// Two further defects found while measuring, both affecting EVERY MySQL column:
//   * DATA_TYPE was hardcoded 0 (java.sql.Types.NULL) instead of the JDBC code.
//   * COLUMN_SIZE was 0 for every numeric and temporal column — our driver
//     surfaces a SQL NULL in an information_schema numeric column as an empty
//     STRING, so the NUMERIC_PRECISION fallback never ran.
//
// Every expectation below was measured against Lucee 7.1.0.204 + Connector/J
// on MariaDB 12.1. The type-mapping rules themselves are unit-tested in Rust
// (crates/cfml-stdlib/src/dbinfo.rs, mod type_mapping_tests), which is the
// coverage that runs everywhere; this suite is the live-driver check.
//
// Live test, gated on RUSTCFML_TEST_MYSQL_DS. To run locally:
//   docker run --rm -d -p 3306:3306 -e MARIADB_ROOT_PASSWORD=root mariadb:11
//   RUSTCFML_TEST_MYSQL_DS=mysql://root:root@127.0.0.1:3306/test \
//     cargo run --features all-databases -- tests/runner.cfm

suiteBegin("cfdbinfo MySQL column types (skipped without RUSTCFML_TEST_MYSQL_DS)");

mydsn = "";
try {
	mydsn = server.system.environment.RUSTCFML_TEST_MYSQL_DS ?: "";
} catch (any e) {
	mydsn = "";
}

if (mydsn == "") {
	writeOutput("  (skipped - set RUSTCFML_TEST_MYSQL_DS to enable)" & chr(10));
	assertTrue("suite skipped without a MySQL datasource", true);
} else {
	tbl = "zz_rcfml_dbinfo_" & lCase(left(replace(createUUID(), "-", "", "all"), 8));
	try {
		queryExecute(
			"CREATE TABLE #tbl# ( c_bit bit(1), c_bool boolean, c_tiny1 tinyint(1),
			   c_tiny4 tinyint(4), c_tinyu tinyint(1) unsigned, c_int int, c_bigint bigint,
			   c_vc varchar(50), c_dec decimal(10,2), c_double double, c_dt datetime )",
			[], { datasource: mydsn } );

		dbinfo type="columns" datasource="#mydsn#" table="#tbl#" name="cols";
		byName = {};
		for (r in cols) {
			byName[ lCase(r.column_name) ] = { type: r.type_name, dt: r.data_type, size: r.column_size };
		}

		// The headline: tinyint(1) and boolean read back as BIT.
		assert("a real bit(1) is BIT",          uCase(byName.c_bit.type),   "BIT");
		assert("boolean is BIT",                uCase(byName.c_bool.type),  "BIT");
		assert("tinyint(1) is BIT",             uCase(byName.c_tiny1.type), "BIT");
		// The measured BOUNDARY: the driver quirk is signed-only, and any other
		// width keeps its own type. Getting this wrong would turn every small
		// integer column into a boolean.
		assert("tinyint(1) UNSIGNED stays TINYINT", uCase(byName.c_tinyu.type), "TINYINT UNSIGNED");
		assert("tinyint(4) stays TINYINT",          uCase(byName.c_tiny4.type), "TINYINT");

		// DATA_TYPE is the java.sql.Types code, not 0.
		assert("BIT reports Types.BIT",         byName.c_bit.dt,    -7);
		assert("boolean reports Types.BIT",     byName.c_bool.dt,   -7);
		assert("tinyint(4) reports Types.TINYINT", byName.c_tiny4.dt, -6);
		assert("int reports Types.INTEGER",     byName.c_int.dt,    4);
		assert("bigint reports Types.BIGINT",   byName.c_bigint.dt, -5);
		assert("varchar reports Types.VARCHAR", byName.c_vc.dt,     12);
		assert("decimal reports Types.DECIMAL", byName.c_dec.dt,    3);
		assert("double reports Types.DOUBLE",   byName.c_double.dt, 8);
		assert("datetime reports Types.TIMESTAMP", byName.c_dt.dt,  93);

		// COLUMN_SIZE is populated for numeric and temporal columns, not just
		// character ones.
		assert("a BIT column is size 1",   byName.c_bit.size,    1);
		assert("int size",                 byName.c_int.size,    10);
		assert("bigint size",              byName.c_bigint.size, 19);
		assert("varchar size",             byName.c_vc.size,     50);
		assert("decimal size",             byName.c_dec.size,    10);
		assert("datetime size is its display width", byName.c_dt.size, 19);

		// GH #539: the rest of Connector/J's getColumns values, measured on
		// Lucee 7.1.2.191 + MySQL 8.4 (utf8mb4, so 4 bytes a character).
		queryExecute(
			"ALTER TABLE #tbl# ADD COLUMN c_dt3 datetime(3), ADD COLUMN c_json json,
			   ADD COLUMN c_year year, ADD COLUMN c_ttext tinytext, ADD COLUMN c_tblob tinyblob,
			   ADD COLUMN c_geo geometry, ADD COLUMN c_pt point, ADD COLUMN c_flt73 float(7,3),
			   ADD COLUMN c_enum enum('a','b'), ADD COLUMN c_char char(5) CHARACTER SET utf8mb4,
			   ADD COLUMN c_vc4 varchar(20) CHARACTER SET utf8mb4",
			[], { datasource: mydsn } );
		dbinfo type="columns" datasource="#mydsn#" table="#tbl#" name="cols";
		full = {};
		for (r in cols) {
			full[ lCase(r.column_name) ] = r;
		}
		assert("BUFFER_LENGTH is Connector/J's constant 65535", full.c_int.buffer_length, 65535);
		assert("CHAR_OCTET_LENGTH is the byte length of a varchar", full.c_vc4.char_octet_length, 80);
		assert("CHAR_OCTET_LENGTH is the byte length of a char", full.c_char.char_octet_length, 20);
		assert("CHAR_OCTET_LENGTH of an enum is its longest member in bytes", full.c_enum.char_octet_length, 4);
		assert("CHAR_OCTET_LENGTH is NULL for a numeric column", full.c_int.char_octet_length, "");
		assert("CHAR_OCTET_LENGTH is NULL for a temporal column", full.c_dt.char_octet_length, "");
		assert("datetime(3) size counts the fraction", full.c_dt3.column_size, 23);
		assert("json size", full.c_json.column_size, 1073741824);
		assert("year is Types.DATE", full.c_year.data_type, 91);
		assert("tinytext is Types.VARCHAR", full.c_ttext.data_type, 12);
		assert("tinyblob is Types.VARBINARY", full.c_tblob.data_type, -3);
		assert("geometry is Types.BINARY", full.c_geo.data_type, -2);
		assert("geometry size", full.c_geo.column_size, 65535);
		assert("a point is presented as GEOMETRY", uCase(full.c_pt.type_name), "GEOMETRY");
		assert("float(7,3) keeps its scale", full.c_flt73.decimal_digits, 3);
		assert("a plain double has scale 0", full.c_double.decimal_digits, 0);

		queryExecute("DROP TABLE #tbl#", [], { datasource: mydsn });
	} catch (any e) {
		try { queryExecute("DROP TABLE IF EXISTS #tbl#", [], { datasource: mydsn }); } catch (any e2) {}
		rethrow;
	}
}

suiteEnd();
</cfscript>
