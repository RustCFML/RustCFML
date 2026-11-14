<cfscript>
// SQL Server TLS through rustls (known issue §121). On macOS the old
// native-tls backend (Secure Transport) could not complete a handshake with a
// stock SQL Server 2022 at all; rustls connects on every platform.
//
// Live test, gated on RUSTCFML_TEST_MSSQL_DS (as the other MSSQL suites). Point
// it at a server with SQL Server's self-generated certificate, for instance:
//   docker run --rm -d -p 1433:1433 -e ACCEPT_EULA=Y -e MSSQL_SA_PASSWORD=Str0ng!Passw0rd \
//     --platform linux/amd64 mcr.microsoft.com/mssql/server:2022-latest
//   RUSTCFML_TEST_MSSQL_DS='mssql://sa:Str0ng!Passw0rd@127.0.0.1:1433/master?trustServerCertificate=true' \
//     cargo run -- tests/runner.cfm
suiteBegin("SQL Server TLS via rustls (skipped without RUSTCFML_TEST_MSSQL_DS)");

tlsDsn = "";
try {
    tlsDsn = server.system.environment.RUSTCFML_TEST_MSSQL_DS ?: "";
} catch (any e) {
    tlsDsn = "";
}

if (!len(tlsDsn)) {
    assertTrue("skipped: RUSTCFML_TEST_MSSQL_DS not set", true);
} else {
    // The same server, with the trust option forced on: the self-generated
    // certificate is X.509 v1, which only trust-all accepts.
    tlsBase = listFirst(tlsDsn, "?");
    tlsTrusted = tlsBase & "?trustServerCertificate=true&encrypt=true";
    tlsConn = queryExecute(
        "SELECT encrypt_option FROM sys.dm_exec_connections WHERE session_id = @@SPID",
        [], { datasource: tlsTrusted }
    );
    assert("the session is encrypted", uCase(tlsConn.encrypt_option), "TRUE");

    // encrypt=false: TLS for the login only, as the JDBC driver does, and no
    // certificate check — so the self-generated certificate is fine here.
    tlsLogin = queryExecute(
        "SELECT encrypt_option FROM sys.dm_exec_connections WHERE session_id = @@SPID",
        [], { datasource: tlsBase & "?encrypt=false" }
    );
    assert("encrypt=false leaves the session unencrypted after the login", uCase(tlsLogin.encrypt_option), "FALSE");

    // Without trust, the self-generated certificate must be REJECTED: the
    // certificate is checked, not waved through.
    tlsRejected = false;
    try {
        queryExecute("SELECT 1 AS x", [], { datasource: tlsBase & "?trustServerCertificate=false&encrypt=true" });
    } catch (any e) {
        tlsRejected = findNoCase("certificate", e.message) > 0;
    }
    assertTrue("an untrusted self-generated certificate is rejected", tlsRejected);
}

suiteEnd();
</cfscript>
