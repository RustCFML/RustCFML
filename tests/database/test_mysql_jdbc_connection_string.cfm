<cfscript>
// A Lucee-style MySQL datasource: a Connector/J `connectionString` carrying
// JDBC-only properties (useUnicode, characterEncoding, allowPublicKeyRetrieval),
// authenticated by SEPARATE `username`/`password` keys. Lucee hands the
// properties to the JDBC driver and uses the keys to log in. RustCFML used to
// reject the first unknown property ("Unknown URL parameter") and, once past
// that, ignored the credentials (access denied for user '').
//
// Live test, gated on RUSTCFML_TEST_MYSQL_DS (mysql://user:pass@host:port/db).

suiteBegin("MySQL JDBC connectionString + separate credentials (skipped without RUSTCFML_TEST_MYSQL_DS)");

jdbcDsn = "";
try {
    jdbcDsn = server.system.environment.RUSTCFML_TEST_MYSQL_DS ?: "";
} catch (any e) {
    jdbcDsn = "";
}

if (jdbcDsn == "") {
    writeOutput("  (skipped - set RUSTCFML_TEST_MYSQL_DS to enable)" & chr(10));
    assertTrue("skipped without a MySQL test datasource", true);
} else {
    jdbcParts = reFind("^mysql://([^:@]*):?([^@]*)@([^/]+)/([^?]*)", jdbcDsn, 1, true);
    jdbcPart = function(n) { return mid(jdbcDsn, jdbcParts.pos[n + 1], jdbcParts.len[n + 1]); };
    jdbcDs = {
        class: "com.mysql.cj.jdbc.Driver",
        bundleName: "com.mysql.cj",
        connectionString: "jdbc:mysql://" & jdbcPart(3) & "/" & jdbcPart(4)
            & "?useUnicode=true&characterEncoding=UTF-8&useSSL=false&allowPublicKeyRetrieval=true",
        username: urlDecode(jdbcPart(1)),
        password: urlDecode(jdbcPart(2))
    };
    try {
        jdbcQ = queryExecute("SELECT 41 + 1 AS answer, CURRENT_USER() AS who", [], { datasource: jdbcDs });
        assert("query runs through the JDBC connectionString", jdbcQ.answer[1], 42);
        assertTrue("authenticated as the separate username", jdbcQ.who[1] CONTAINS urlDecode(jdbcPart(1)));
        jdbcP = queryExecute("SELECT ? AS v", [ "héllo" ], { datasource: jdbcDs });
        assert("parameters and UTF-8 round-trip", jdbcP.v[1], "héllo");
    } catch (any e) {
        assertTrue("JDBC connectionString datasource failed: " & e.message, false);
    }
}

suiteEnd();
</cfscript>
