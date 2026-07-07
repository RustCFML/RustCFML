# RustCFML Development Status

Compatibility target: [cfdocs.org/functions](https://cfdocs.org/functions) and [cfdocs.org/tags](https://cfdocs.org/tags)

---

## What's Implemented

### Core Language
Full CFScript support: variables, operators (arithmetic, comparison, logical, string, bitwise), control flow (if/else, for, for-in, while, do-while, switch/case, break/continue, try/catch/finally, throw/rethrow), functions (UDFs, closures, arrow functions, recursion, access modifiers), string interpolation (`#expr#`), spread operator, elvis operator (`?:`), null coalescing (`??`), null-safe navigation (`?.`). `throw()` function form with named args.

### Data Types
Null, Boolean, Integer (i64), Double (f64), String (CI comparison), Array (1-based), Struct (CI keys), Function, Closure, Component, Query, Binary.

### Tag Syntax (50+ tags)
Tag preprocessor converts CFML tags to CFScript. Supports: cfset, cfoutput, cfif/cfelseif/cfelse, cfloop, cfscript, cffunction, cfargument, cfreturn, cfinclude, cfdump, cfthrow, cftry/cfcatch/cffinally, cfabort, cfparam, cfcomponent, cfinterface, cfproperty, cfhttp, cfhttpparam, cfquery, cfqueryparam, cfheader, cfcontent, cflocation, cfdirectory, cfsavecontent, cfinvoke, cftransaction, cfswitch/cfcase/cfdefaultcase, cfbreak, cfcontinue, cfwhile, cfrethrow, cflock, cfsilent, cflog, cfsetting, cfcookie, cffile, cfloginuser, cflogout, cfmodule, cf_ prefix custom tags, cfmail/cfmailparam/cfmailpart, cfcache, cfexecute, cfstoredproc/cfprocparam/cfprocresult, cfimport (taglib with .tld support), cfthread, cfzip, cfdbinfo. CFScript `lock` block syntax supported. Custom tags support self-closing and body modes, caller write-back, thisTag scope, this.customTagPaths.

### Standard Library (400+ functions)
- **String (50+)**: len, ucase, lcase, trim, ltrim, rtrim, find, findNoCase, findOneOf, mid, left, right, replace, replaceNoCase, replaceList, replaceListNoCase, reverse, repeatString, insert, removeChars, spanIncluding, spanExcluding, compare, compareNoCase, asc, chr, reFind, reFindNoCase, reReplace, reReplaceNoCase, reMatch, reMatchNoCase, wrap, stripCr, toBase64, toBinary, urlEncodedFormat, urlDecode, htmlEditFormat, htmlCodeFormat, encodeForHTML, xmlFormat, paragraphFormat, ucFirst, jsStringFormat, reEscape, getToken, newLine, lJustify, rJustify, cJustify, numberFormat, decimalFormat, formatBaseN, inputBaseN
- **Array (46+)**: arrayNew, arrayLen, arrayAppend, arrayPrepend, arrayDeleteAt, arrayInsertAt, arraySet, arraySwap, arrayContains, arrayContainsNoCase, arrayFind, arrayFindNoCase, arrayFindAll, arrayFindAllNoCase, arraySort, arrayReverse, arraySlice, arrayToList, arrayMerge, arrayClear, arrayIsDefined, arrayMin, arrayMax, arrayAvg, arraySum, arrayFirst, arrayLast, arrayIsEmpty, arrayDelete, arrayPop, arrayShift, arrayPush, arrayUnshift, arrayIndexExists, arrayResize, arrayMedian, arrayMid, arrayReduceRight, arraySplice, arrayRange, arrayToStruct, arrayDeleteNoCase + higher-order: map, filter, reduce, each, some, every
- **Struct (31+)**: structNew, structCount, structKeyExists, structKeyList, structKeyArray, structDelete, structInsert, structUpdate, structFind, structFindKey, structFindValue, structClear, structCopy, structAppend, structIsEmpty, structSort, structGet, structValueArray, structEquals, isEmpty, structToSorted, structIsOrdered, structIsCaseSensitive, structToQueryString, structGetMetadata, structSetMetadata + higher-order: each, map, filter, reduce, some, every
- **Math (25+)**: abs, ceiling, floor, round, int, fix, max, min, sgn, sqr, exp, log, log10, trig functions, pi, rand, randRange, randomize, bitwise operations
- **Date/Time (39)**: now, createDate, createDateTime, createTime, ODBC date functions, date part accessors, dateAdd, dateDiff, dateFormat, timeFormat, dateTimeFormat, parseDateTime, datePart, dateCompare, getTickCount, millisecond, dateConvert, getNumericDate, getHTTPTimeString, nowServer
- **List (32+)**: listLen, listFirst, listLast, listRest, listGetAt, listSetAt, listDeleteAt, listInsertAt, listFind, listFindNoCase, listContains, listContainsNoCase, listAppend, listPrepend, listSort, listRemoveDuplicates, listToArray, listValueCount, listValueCountNoCase, listQualify, listChangeDelims, listCompact, listSome, listEvery, listAvg, listItemTrim, listIndexExists, listReduceRight + higher-order: each, map, filter, reduce
- **Query (26)**: queryNew, queryAddRow, querySetCell, queryAddColumn, queryGetRow, queryGetCell, queryRecordCount, queryColumnCount, queryColumnList, queryDeleteRow, queryDeleteColumn, queryColumnExists, queryRowData, querySlice, queryGetResult, queryKeyExists, queryColumnData/queryColumnArray, queryCurrentRow, queryAppend, queryInsertAt, queryPrepend, queryReverse, queryRowSwap, querySetRow, valueList, quotedValueList + higher-order: each, map, filter, reduce, sort, some, every
- **Type checking (12)**: isNull, isDefined, isNumeric, isBoolean, isDate, isArray, isStruct, isQuery, isSimpleValue, isBinary, isValid, isCustomFunction
- **Conversion (15)**: toString, toNumeric, toBoolean, val, javacast, parseNumber, createTimeSpan, yesNoFormat, booleanFormat, trueFalseFormat, nullValue, incrementValue, decrementValue, de, dollarFormat
- **JSON (3)**: serializeJSON, deserializeJSON, isJSON
- **File I/O (23+)**: fileRead, fileWrite, fileAppend, fileExists, fileDelete, fileMove, fileCopy, fileOpen, fileClose, fileReadLine, fileWriteLine, fileReadBinary, fileGetMimeType, fileIsEOF, fileUpload, fileUploadAll, fileSetAccessMode, fileSetAttribute, fileSetLastModified, directoryCreate, directoryExists, directoryDelete, directoryList, directoryRename, directoryCopy, getTempDirectory, getTempFile, getFileInfo, expandPath
- **Security**: encrypt/decrypt (AES/DES/DESEDE/Blowfish/CFMX_COMPAT), hmac, generateSecretKey, hash (MD5/SHA family), createUUID, createGUID, encodeForHTML/URL/CSS/JavaScript, generatePBKDFKey, generateBCryptHash/verifyBCryptHash, generateSCryptHash/verifySCryptHash, generateArgon2Hash/argon2CheckHash, csrfGenerateToken/csrfVerifyToken
- **Encoding/Decoding**: charsetDecode, charsetEncode, encodeForHTMLAttribute, encodeForXML, encodeForXMLAttribute, encodeFor, decodeForHTML, decodeFromURL, urlEncode, canonicalize
- **Locale (13)**: lsDateFormat, lsTimeFormat, lsDateTimeFormat, lsCurrencyFormat, lsEuroCurrencyFormat, lsIsDate, lsIsNumeric, lsIsCurrency, lsParseCurrency, lsParseDateTime, lsNumberFormat, lsWeek, lsDayOfWeek
- **Error Handling**: cfcatch.tagContext (stack trace array with template, line, id, raw_trace, column), exceptionKeyExists
- **XML/HTML (11+)**: xmlParse, xmlSearch, isXML, xmlNew, xmlElemNew, xmlChildPos, xmlGetNodeType, xmlHasChild, isXMLDoc/Elem/Node/Root/Attribute, htmlParse
- **Caching (8)**: cachePut, cacheGet, cacheDelete, cacheClear, cacheKeyExists, cacheCount, cacheGetAll, cacheGetAllIds
- **Higher-Order Generics**: collectionEach/Map/Filter/Reduce/Some/Every, stringEach/Map/Filter/Reduce/Some/Every/Sort, each (generic)
- **Utility (23+)**: writeOutput, writeDump, dump, sleep, duplicate, writeLog, systemOutput, trace, location, applicationStop, getApplicationMetadata, getApplicationSettings, getFileFromPath, getCanonicalPath, getTemplatePath, setLocale, getLocale, setTimeZone, getTimeZone, getBaseTemplatePath, getCurrentTemplatePath, getDirectoryFromPath, setVariable, getVariable, getEnvironmentVariable
- **Session/Auth**: sessionInvalidate, sessionRotate, sessionGetMetaData, getAuthUser, isUserLoggedIn, isUserInRole
- **Bitmask**: bitMaskClear, bitMaskRead, bitMaskSet
- **Zip**: cfzip (actions: zip, unzip, list, read, readBinary, delete)
- **Stack/Precision**: callStackGet, callStackDump, precisionEvaluate
- **INI**: getProfileString, setProfileString, getProfileSections
- **Misc**: soundex, metaphone, toScript

### Member Functions
String, Array, Struct, Number member functions with method chaining.

### Scopes
local, variables, arguments, request, application (persistent), server, session (CFID cookie), cookie. Case-insensitive lookup, closure scope capture with write-back.

### Components & Interfaces
extends (dot-path), super.method(), isInstanceOf, getMetadata, createObject, interface/implements with inheritance, implicit accessors, onMissingMethod, component mappings (this.mappings). CFC may also declare `extends="rust:Name"` to inherit from a registered native class: default + explicit `super(args)` construction, super.X dispatch, implicit method fall-through, optional `this.X` property fall-through via `CfmlNative::get_property`/`set_property`.

### Web Server (Axum)
`--serve` mode with: HTTP response infrastructure, form scope (url-encoded + multipart), url scope, cgi scope (remote_addr, server_name from Host header, all http_* headers), cookie scope, session management (onSessionStart/onSessionEnd lifecycle, configurable timeout), file uploads, Application.cfc lifecycle, URL rewrite engine (Tuckey-compatible). Real named locks with cflock/cfscript lock (RwLock-based concurrency). Bracket assignment for component mappings. **Bytecode cache**: mtime-based caching skips recompilation for unchanged files across requests (includes, custom tags, CFCs, Application.cfc).

**Session storage** has four backends selected by `sessionStorage`: `memory` (default, in-process), `memcached`, `cluster` (memberlist + Automerge CRDT), and `datasource` (SQL — SQLite/MySQL/PostgreSQL/MSSQL, auto-created `cf_session_data` table, last-write-wins, throttled sliding expiry, best-effort `onSessionEnd` sweep). **Lazy session creation is the engine-wide default** (no record/cookie/`onSessionStart` until a session write — a privacy-friendly divergence from Lucee; opt out with `this.lazySessionCreation = false`). The **session scope is data-only** across every store: writing a component/closure/function/native object throws with the offending key path instead of silently serialising to null. See [known-issues.md §12](known-issues.md).

### Database
queryExecute with SQLite, MySQL, PostgreSQL, MSSQL. Connection pooling, cfqueryparam, cftransaction, structured query params, `result=` metadata delivery (incl. dotted names + attributeCollection), cfdbinfo metadata (version/columns/tables/index/foreignkeys/dbnames/procedures/users/terms) on all four drivers.

### Threading (Real OS Threads)
cfthread tag with action=run/join/terminate. **Thread bodies run on real OS threads** (concurrently, on separate cores); join blocks with an optional timeout, and no name joins all outstanding threads. `application`/`server`/`session`/`request` scopes are shared live across threads (guard concurrent writes with cflock); `variables` is copied at spawn; `attributes` are bound from the parent at spawn; the per-thread `thread` scope plus status/output/error/elapsedtime surface as `cfthread.NAME`. Errors set status TERMINATED without aborting the parent. Default-on behind the `real-threads` Cargo feature; wasm targets (no `std::thread`) and feature-off builds fall back to the synchronous-inline path. **Two deliberate Lucee divergences:** `terminate` is cooperative (the body aborts at loop back-edges, not mid-instruction — Rust can't safely force-kill a thread), and a `cftransaction` can't span the parent↔child boundary (the DB connection is thread-bound). See [Threading](threads.md).

### Infrastructure
CLI (file exec, `-c` inline, `-d` debug, `-r` REPL, `--serve`), WASM target, error handling with line/column info.

---

## Query-of-Queries (QoQ)

`queryExecute(sql, params, {dbtype:"query"})` runs in-memory SQL `SELECT` against query
variables in scope. Pure-Rust engine in `crates/cfml-qoq` (no JDBC/HSQLDB). Supported:

- `SELECT` (incl. `*`, `table.*`, aliases), `WHERE`, `GROUP BY`, `HAVING`, `ORDER BY`
  (multi-key, ASC/DESC), `DISTINCT`
- Joins: `INNER` / `LEFT` / `RIGHT` / `FULL` `[OUTER] JOIN ... ON`, `CROSS` and comma joins
- Subqueries: `IN (SELECT …)`, scalar `(SELECT …)` in the SELECT list, derived `FROM (SELECT …) AS t`
- `UNION` / `UNION ALL`, `CASE`, `CAST`/`CONVERT`, `BETWEEN`, `LIKE [ESCAPE]`, `IN (…)`, `IS [NOT] NULL`
- Params: positional `?` and named `:name` (from the array/struct 2nd arg, incl. `cfqueryparam`)
- `returntype` = `query` (default) / `array` / `struct`
- Built-in scalar + aggregate SQL functions; **extensible** via `register_native_qoq_fn`
  (native Rust) and `queryRegisterFunction(name, udf[, "aggregate"])` (CFML UDFs in SQL)
- Native `CfmlValue` types preserved through projection; ORDER BY parallelised (rayon, non-wasm)

Lucee QoQ lacks `LIMIT`/`OFFSET`, scalar subqueries, derived tables and `CASE` (it uses `TOP`);
RustCFML supports those as a BoxLang-faithful superset — see `docs/known-issues.md`.

---

## Remaining Work

### Low Priority
- (none currently)

### Partial Support
- JWT: `JwtSign` / `JwtVerify` / `JwtDecode` (Lucee crypto-extension names) for the
  HMAC algorithms HS256 / HS384 / HS512. RSA (RS*/PS*) and ECDSA (ES*) — which need
  asymmetric keys — are not yet implemented and fail with a clear error. The Adobe
  `createSignedJWT` / `verifySignedJWT` / `createEncryptedJWT` / `verifyEncryptedJWT`
  names are not provided.

### Partially Supported
- **Image functions** — Tiers 1–3 implemented (read/write/resize/scaleToFit/crop/
  any-angle rotate/flip/info/blob/base64; the full `imageDraw*`/`imageSetDrawing*`
  drawing surface with text + compositing; blur/sharpen/negative/grayscale/translucent
  filters; translate/shear transforms; EXIF + IPTC metadata; and `<cfimage>`
  read/write/resize/info/convert/writeToBrowser/border/captcha). Backed by pure-Rust
  `image`/`imageproc`/`ab_glyph`/`kamadak-exif` (native + wasm). Rasterisation is
  visually equivalent to, but not pixel-identical with, Lucee's Java2D renderer, and
  text uses a bundled DejaVu Sans font. See [known-issues.md §18](known-issues.md).

### Explicitly Unsupported
- Spreadsheet functions (40+), ORM (20+), SOAP/WSDL, Flash/Flex UI tags, Exchange, PDF, LDAP, Registry, .NET, Gateway
- cfschedule, cfwddx

### Taffy Framework
Believed feature-complete. cgi.remote_addr, cgi.server_name (from Host header), and all request headers (cgi.http_*) available for rate limiting and CORS.

---

## Architecture

```
CFML Source → Tag Preprocessor (tag_parser.rs) → CFScript
  → Lexer (lexer.rs) → Tokens
  → Parser (parser.rs) → AST (ast.rs)
  → Compiler (compiler.rs) → Bytecode
  → VM (lib.rs) → Execution
```

```
crates/
├── cfml-common/     # CfmlValue, CfmlError, Position
├── cfml-compiler/   # Lexer, Parser, AST, Tag Preprocessor
├── cfml-codegen/    # AST → BytecodeOp
├── cfml-vm/         # Stack-based VM
├── cfml-stdlib/     # 400+ built-in functions
├── cli/             # CLI + Axum web server
└── wasm/            # WebAssembly target
```

Reference: [cfdocs.org/functions](https://cfdocs.org/functions) | [cfdocs.org/tags](https://cfdocs.org/tags) | BoxLang ANTLR grammar | Lucee expression grammar

*Last Updated: 2026-06-15* | 3965 assertions across 478 suites
