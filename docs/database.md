# Database

[← Back to README](../README.md)

RustCFML runs parameterized queries via `queryExecute` (and the `<cfquery>` tag), with connection pooling, `cfqueryparam`, and `cftransaction`.

## Supported engines

| Engine | Driver | Feature flag | Notes |
|---|---|---|---|
| **SQLite** | `rusqlite` (bundled) | on by default | Zero-config; file-based or in-memory (`:memory:`). The default fallback datasource. |
| **MySQL / MariaDB** | `mysql` (TLS via native-tls) | `mysql_db` | MariaDB is wire-compatible and uses the same driver. TLS via `ssl_mode` — see [Working with MySQL](#working-with-mysql-and-mariadb). |
| **PostgreSQL** | `postgres` (TLS via rustls) | `postgres_db` | See [Working with PostgreSQL](#working-with-postgresql) below. On Cloudflare Workers, reached via **Hyperdrive** (see below). |
| **Microsoft SQL Server** | `tiberius` (TLS via rustls) | `mssql_db` | Also covers Azure SQL — connections are encrypted by default. See [Working with SQL Server](#working-with-sql-server). |

Drivers are feature-gated so unused subsystems compile out. SQLite is on by default; enable the others at build time:

```bash
cargo build --release --features "mysql_db,postgres_db,mssql_db"
```

**Prebuilt release binaries include all four drivers.** On the WebAssembly / Cloudflare Workers build the native drivers are not compiled — datasource access goes through a **[Hyperdrive](https://developers.cloudflare.com/hyperdrive/)** binding instead (PostgreSQL via `postgres.js`, MySQL via `mysql2`). The `queryExecute` API and parameter handling are identical, so application code is portable between the native server and the edge build. See [PostgreSQL on Cloudflare Workers](#postgresql-on-cloudflare-workers).

## Datasources

Define datasources in `.cfconfig.json` (see **[Configuration](configuration.md)**), then reference them by name:

```cfml
qry = queryExecute(
    "select id, name from users where active = ?",
    [ true ],
    { datasource = "app" }
);
```

If no `datasource` is given, the default datasource is used, falling back to an in-memory SQLite database (`:memory:`) so quick scripts and tests work with zero configuration.

## Parameters

Both placeholder styles are supported:

```cfml
// Positional — array of values
queryExecute("select * from users where id = ? and active = ?", [ 42, true ]);

// Named — struct of values (:name)
queryExecute(
    "select * from users where id = :id and active = :active",
    { id = 42, active = true }
);

// cfqueryparam-style array of structs (with optional type hints / list expansion)
queryExecute(
    "select * from users where id in (?)",
    [ { value = "1,2,3", list = true } ]
);
```

A query column passed as a parameter (e.g. `someQuery.id`) binds as its first-row scalar value, matching scalar query-column coercion elsewhere in the language.

## Return types

```cfml
queryExecute(sql, params, { returntype = "query" });   // default — a query object
queryExecute(sql, params, { returntype = "array" });   // array of row structs
queryExecute(sql, params, { returntype = "struct", columnkey = "id" }); // keyed struct
```

## Transactions

```cfml
cftransaction {
    queryExecute("update accounts set balance = balance - ? where id = ?", [ 100, 1 ]);
    queryExecute("update accounts set balance = balance + ? where id = ?", [ 100, 2 ]);
}
```

---

## Working with PostgreSQL

PostgreSQL is well supported, but it is stricter on the wire than the other engines — it uses `$1`/`$2` positional placeholders. RustCFML binds common scalar types in **binary** format (encoded at the column's exact type) and sends the rest (arrays, network/temporal-zone, extension types) in **text** format for the server to parse, matching the JDBC engines. RustCFML handles the translation so ordinary CFML works, but a few behaviours are worth knowing.

### TLS / SSL connections (Neon, Supabase, RDS, Azure, …)

Managed PostgreSQL services require an encrypted connection. RustCFML negotiates
TLS over PostgreSQL's `SSLRequest` preamble using **rustls**, honouring the
`sslmode` option in the datasource URL — so a Neon/Supabase/RDS connection
string works as-is:

```cfc
queryExecute("select 1", [], { datasource =
    "postgresql://user:pass@ep-xxx-pooler.eu-central-1.aws.neon.tech/neondb?sslmode=require&channel_binding=require" });
```

Supported `sslmode` values (libpq-compatible, default **`prefer`**):

| `sslmode` | Behaviour |
|-----------|-----------|
| `disable` | No TLS — plaintext only. |
| `allow` / `prefer` | Attempt TLS, fall back to plaintext if the server refuses. **Default** — keeps local, non-TLS databases working with zero config. |
| `require` | TLS required; the channel is encrypted but the server certificate is **not** verified (matches libpq / pgjdbc). |
| `verify-ca` / `verify-full` | TLS required **and** the server certificate is verified against the platform's native root certificate store (full chain + hostname). |

`channel_binding` (`disable`/`prefer`/`require`) is supported via SCRAM
`tls-server-end-point`. SNI is sent automatically, so pooler endpoints route
correctly.

> **Note:** `verify-ca` is treated identically to `verify-full` (it also checks
> the hostname). If you need CA-only verification without a hostname match
> (e.g. connecting by IP), use `require` or open an issue.

### Placeholders are translated automatically

You write CFML-style `?` (or `:name`) placeholders; RustCFML rewrites them to PostgreSQL's `$1`, `$2`, … form. Quoted string literals and `::type` casts are left untouched.

### Untyped CFML strings are coerced to the column type

CFML values are frequently strings (form and URL values are untyped). Because PostgreSQL binds parameters in binary, RustCFML encodes each parameter at the **target column's type** — so a string `"251"` binds correctly to an `int4` column, `"3.14"` to `numeric`, `"true"` to `boolean`, and so on. Integers and floats are likewise encoded at the column's exact width (e.g. `int2`/`int4`/`int8`), avoiding "incorrect binary data format" errors.

### Date/time, JSON, and `vector` parameters

CFML date values bind to `timestamp`, `date` and `time` columns as their wall
clock in the request time zone, and to `timestamptz` at their instant, keeping
fractional seconds. Date strings bind too; a zone-less string bound to
`timestamptz` is interpreted as UTC. ISO 8601 / RFC 3339 strings carrying a numeric offset, a `Z` (Zulu/UTC)
suffix, and/or fractional seconds also bind — `"2026-06-10T07:20:42.177+00:00"`,
`"...Z"`, `"...177"` — so a record read out (and serialized in that exact shape)
can be re-bound and saved back. For a `timestamptz` column the offset is honoured
to recover the true instant; a zone-less value is taken as UTC wall-clock.
JSON text (e.g. the output of `serializeJSON`) binds to `json` and `jsonb`
columns — the `jsonb` version prefix is written for you. Vector-literal strings
(`"[1,0,0]"`) bind to pgvector `vector` columns (INSERT and `<->` nearest-neighbour
queries), encoded in pgvector's binary wire format.

```cfml
queryExecute("update events set at = ? where id = ?",
    [ createDateTime(2024,3,15,10,30,45), 1 ], { datasource = "app" }); // -> timestamptz
queryExecute("insert into docs (id, body) values (?, ?)",
    [ 1, serializeJSON({ name = "alpha" }) ], { datasource = "app" });  // -> jsonb
queryExecute("select id from items order by embedding <-> ? limit 5",
    [ "[0.9,0.1,0]" ], { datasource = "app" });                        // -> vector
```

### Arrays and other non-scalar types (sent as text — Lucee/BoxLang parity)

RustCFML binary-encodes the common scalar types above. For everything else —
arrays (`int[]`, `text[]`, …), `interval`, `inet`/`cidr`, `macaddr`, `timetz`,
ranges, `hstore`, and so on — the parameter is sent in PostgreSQL's **text**
format and parsed by the server, exactly as Lucee and BoxLang do (both use the
JDBC driver, which sends parameters as text). So a CFML array literal binds to
an array column, and the long tail of extension/specialised types works without
a bespoke binary encoder:

```cfml
queryExecute("update t set tags = ? where id = ?", [ "{red,green,blue}", 1 ], { datasource = "app" }); // -> text[]
queryExecute("update t set window = ? where id = ?", [ "2 days 03:00:00", 1 ], { datasource = "app" }); // -> interval
queryExecute("update t set client = ? where id = ?", [ "10.0.0.5", 1 ], { datasource = "app" });        // -> inet
```

### UUID string parameters

A CFML string can be bound directly to a `uuid` column — it is parsed and sent in PostgreSQL's UUID wire format:

```cfml
queryExecute(
    "select * from profiles where id = ?",
    [ "41048aa7-27c9-4517-a93e-82bf7c76cc66" ],   // string -> uuid column
    { datasource = "app" }
);
```

### `UNKNOWN`-typed parameters

When PostgreSQL can't pin a concrete parameter type early (for example a bare `select ?`), RustCFML encodes the value as text and lets the server coerce it — so framework-generated statements that don't force a type still work.

### Server error messages surface the cause

When a query fails server-side, `cfcatch.message` carries the server's own
message (e.g. `ERROR: function zz_x() does not exist`, `ERROR: duplicate key
value violates unique constraint …`), not a bare `db error`. The driver's
error-`source()` chain is walked and appended, so failures are diagnosable from
CFML and the server log — matching what Lucee surfaces.

### Multi-statement mutations

PostgreSQL's protocol runs one parameterized statement per round-trip. RustCFML splits a multi-statement, non-`SELECT` mutation into its component statements (respecting quoted literals and comments), renumbers each statement's placeholders from `$1`, runs them in order, and returns the **total** affected row count. This makes framework "delete existing rows + insert replacements" operations work as a single `queryExecute` call:

```cfml
queryExecute(
    "
        delete from profile_role where profile_id = ?;
        insert into profile_role (profile_id, role_id) values (?, ?);
    ",
    [ profileId, profileId, roleId ],
    { datasource = "app" }
);
```

If the number of supplied parameters doesn't match what the statements consume, RustCFML raises a clear error rather than binding the wrong values.

### PostgreSQL on Cloudflare Workers

The same `queryExecute` API works in the WebAssembly/Cloudflare Workers build, where PostgreSQL is reached through a [Hyperdrive](https://developers.cloudflare.com/hyperdrive/) binding (via `postgres.js`). The placeholder rewriting and multi-statement handling described above apply there too, so application code is portable between the native server and the edge build. See **[RustCFML-Cloudflare-worker](https://github.com/RustCFML/RustCFML-Cloudflare-worker)**.

## Working with MySQL and MariaDB

For managed MySQL/MariaDB services that require an encrypted connection
(PlanetScale, Aiven, Amazon RDS, Azure Database for MySQL, …) add an
`ssl_mode` option to the datasource URL:

```cfc
queryExecute("select 1", [], { datasource =
    "mysql://user:pass@host:3306/db?ssl_mode=REQUIRED" });
```

Supported `ssl_mode` values (case-insensitive):

| `ssl_mode` | Behaviour |
|------------|-----------|
| `DISABLED` | No TLS (also the default when no `ssl_mode` is given). |
| `PREFERRED` / `REQUIRED` | TLS required; channel encrypted, server certificate **not** verified. |
| `VERIFY_CA` | TLS required + verify the certificate chain (not the hostname). |
| `VERIFY_IDENTITY` | TLS required + verify the chain **and** the hostname. |

JDBC-style options are also honoured: `useSSL=true`, `requireSSL=true`,
`verifyServerCertificate=true`. A custom root CA can be supplied with
`ssl_ca=/path/to/ca.pem` (also accepted as `sslrootcert`). Verification uses
the platform's native trust store. TLS uses the `mysql` crate's `native-tls`
backend.

> **Note:** unlike PostgreSQL, the default is **no TLS** (to preserve
> zero-config local connections), and there is no automatic "try TLS then fall
> back" — a non-`DISABLED` mode requires a successful TLS handshake. Set
> `ssl_mode=REQUIRED` (or stricter) explicitly for managed databases.

## Working with SQL Server

Microsoft SQL Server connections are **encrypted by default** — the `tiberius`
driver negotiates TLS (via rustls) and, out of the box, trusts the server
certificate. This works with Azure SQL Database and any managed instance
without extra configuration:

```cfc
queryExecute("select 1", [], { datasource =
    "sqlserver://user:pass@host:1433/db" });
```

To validate the server certificate against the platform trust store instead of
trusting it blindly, add `trustServerCertificate=false` (or `encrypt=strict`)
to the URL:

```cfc
queryExecute("select 1", [], { datasource =
    "sqlserver://user:pass@host.database.windows.net:1433/db?trustServerCertificate=false" });
```

## Tracing the SQL an app actually sends

When a framework assembles a statement for you, the database's own error text is
the only evidence of what it built — and it quotes a fragment, not the statement.
Set `RUSTCFML_SQL_TRACE=1` and every statement is written to stderr, verbatim,
with its bind values, just before it runs:

```bash
RUSTCFML_SQL_TRACE=1 rustcfml --serve 2> sql.log
```

```
[sql-trace] datasource=preside len=412 sql>>>
select `x`.`id` from `pobj_x` `x` where (a = :one)
<<<sql
[sql-trace]   :one = 42
```

Off by default, read once per process. Trace output contains your query
parameters, so treat the log as sensitive and do not leave it on in production.

## Query-of-Queries

Pass `dbtype="query"` to run an in-memory SQL `SELECT` over query variables already in scope — no datasource, no driver, no JDBC. The engine lives in `crates/cfml-qoq` and is pure Rust; it parallelises filter/projection/sort across cores (non-wasm).

```cfm
<cfscript>
employees = queryExecute("SELECT * FROM users WHERE active = 1");  // real DB
top = queryExecute(
    "SELECT name, salary FROM employees WHERE salary > :min ORDER BY salary DESC",
    { min: 50000 },
    { dbtype: "query" }                                              // QoQ
);
</cfscript>
```

Supported: `SELECT` (with `*`, `table.*`, aliases), `WHERE`, `GROUP BY`, `HAVING`, `ORDER BY` (multi-key, ASC/DESC), `DISTINCT`, `LIMIT`/`OFFSET`; `INNER`/`LEFT`/`RIGHT`/`FULL [OUTER] JOIN ... ON`, `CROSS` and comma joins; `UNION` / `UNION ALL`; `IN (SELECT ...)`, scalar subqueries in the SELECT list, derived `FROM (SELECT ...) AS t`; `CASE`, `CAST`/`CONVERT`, `BETWEEN`, `LIKE [ESCAPE]`, `IS [NOT] NULL`; positional `?` and named `:name` params (incl. `cfqueryparam`); `returntype` `query`/`array`/`struct`. Extensible — `register_native_qoq_fn` exposes a Rust function as both a BIF and a QoQ function; `queryRegisterFunction(name, udf[, "aggregate"])` registers a CFML UDF for use in SQL.

Following BoxLang, RustCFML's QoQ is a **strict superset** of Lucee's: `LIMIT`/`OFFSET`, scalar subqueries, derived tables and `CASE` are accepted here but rejected by Lucee QoQ (it uses `TOP`). Same input → more accepted; not a wrong-result divergence — but SQL that uses those features is not portable back to Lucee. Correlated subqueries are **not** supported (subqueries run once, uncorrelated). See **[Known Issues §9](known-issues.md)** for the full superset table.

Performance (1M-row source, [bdw429s/cfml-qoq-perf-tests](https://github.com/bdw429s/cfml-qoq-perf-tests), 5-run median, same machine, lower is better):

| Engine | Total (ms) | RustCFML speedup |
|---|---:|---:|
| **RustCFML** v0.112 | **1,116** | — |
| BoxLang 1.14 | 1,368 | **1.23× faster** |
| Lucee 7.0.4 | 7,884 | **7.1× faster** |
