<cfscript>
suiteBegin("Tags: cflog / writeLog file appenders");

/*
    GitHub #286 — `<cflog>` / `writeLog()` used to format the message and
    eprintln! it to stderr; the `file=`/`log=` attribute was accepted but no log
    file was ever written.

    Every expectation below was verified against Lucee 7.0.4 (CommandBox,
    `box server start cfengine=lucee@7`) by running the same tags and diffing
    the resulting `logs/*.log` files, so this suite is a cross-engine
    conformance bar and not just a description of our own behaviour:

      - line layout: "Severity","ThreadID","Date","Time","Context","Application","Message"
      - the CSV header row is written once, when the file is created
      - `type=` maps to log4j2 severities (information|info→INFO, warning|warn→WARN,
        error→ERROR, fatal→FATAL, debug→DEBUG, trace→TRACE); anything else errors
      - no `file=`/`log=` ⇒ the `application` log
      - `log=` names a *configured* logger; an unknown name falls back to
        `application` rather than minting a new file (`file=` does create one)
      - `application="false"` blanks the Application column (default is true)
      - a path separator in `file=` is an error, not a directory traversal

    `server.cfconfig.logging.logsDirectory` reports the resolved directory, so
    these tests can read back what they wrote without assuming a cwd.
*/

// Locate the engine's log directory. RustCFML reports the resolved path on
// `server.cfconfig`; Lucee resolves the `{lucee-server}` placeholder. Both land
// on the directory `<cflog file="x">` writes `x.log` into, so this suite runs
// unchanged on either engine.
if (structKeyExists(server, "cfconfig") AND structKeyExists(server.cfconfig, "logging")) {
    logDir = server.cfconfig.logging.logsDirectory;
} else {
    logDir = expandPath("{lucee-server}/logs/");
}
assertTrue("the engine's log directory is discoverable", len(logDir) GT 0);
if (right(logDir, 1) NEQ "/" AND right(logDir, 1) NEQ "\") {
    logDir = logDir & "/";
}

// Unique per run so a stale file from an earlier run can never make a test pass.
stamp = "rcfml_286_" & hash(getTickCount() & createUUID(), "MD5");

function logLines(name) {
    var path = logDir & arguments.name & ".log";
    if (!fileExists(path)) {
        return [];
    }
    var out = [];
    for (var line in listToArray(fileRead(path), chr(10))) {
        if (len(trim(line))) {
            arrayAppend(out, line);
        }
    }
    return out;
}

/** The Message column of the last line, CSV-decoded. */
function lastMessage(name) {
    var lines = logLines(arguments.name);
    if (!arrayLen(lines)) {
        return "";
    }
    var line = lines[arrayLen(lines)];
    // Message is the 7th quoted field; take everything after the 6th comma-quote
    // boundary and strip the enclosing quotes.
    var fields = csvFields(line);
    return arrayLen(fields) GTE 7 ? fields[7] : "";
}

/** Split one Lucee-layout log line into its 7 CSV fields. */
function csvFields(line) {
    var fields = [];
    var buf = "";
    var inQuotes = false;
    var i = 1;
    while (i LTE len(arguments.line)) {
        var ch = mid(arguments.line, i, 1);
        if (ch EQ '"') {
            if (inQuotes AND mid(arguments.line, i + 1, 1) EQ '"') {
                buf &= '"';      // doubled quote = one literal quote
                i += 2;
                continue;
            }
            inQuotes = !inQuotes;
            i++;
            continue;
        }
        if (ch EQ "," AND !inQuotes) {
            arrayAppend(fields, buf);
            buf = "";
            i++;
            continue;
        }
        buf &= ch;
        i++;
    }
    arrayAppend(fields, buf);
    return fields;
}

// ── file= creates the log and writes the header exactly once ────────────────
target = stamp & "_a";
writeLog(text = "first line", file = target, type = "information");
writeLog(text = "second line", file = target, type = "information");
lines = logLines(target);

assertTrue("file= creates <name>.log", arrayLen(lines) GTE 3);
assert(
    "header row is the Lucee layout",
    lines[1],
    '"Severity","ThreadID","Date","Time","Context","Application","Message"'
);
assert("header is written once, not per line", arrayLen(lines), 3);
assert("last message round-trips", lastMessage(target), "second line");

// ── the Severity column: type= → log4j2 level ──────────────────────────────
severityFor = function(cfmlType) {
    var name = stamp & "_sev_" & cfmlType;
    writeLog(text = "x", file = name, type = cfmlType);
    var fields = csvFields(logLines(name)[2]);
    return fields[1];
};
assert("type=information → INFO", severityFor("information"), "INFO");
assert("type=info → INFO", severityFor("info"), "INFO");
assert("type=warning → WARN", severityFor("warning"), "WARN");
assert("type=warn → WARN", severityFor("warn"), "WARN");
assert("type=error → ERROR", severityFor("error"), "ERROR");
assert("type=fatal → FATAL", severityFor("fatal"), "FATAL");
assert("type=debug → DEBUG", severityFor("debug"), "DEBUG");
assert("type=trace → TRACE", severityFor("trace"), "TRACE");

// Lucee: "Invalid value for attribute type [bogus]".
assertThrows("an unknown type= is an error", function() {
    writeLog(text = "x", file = stamp & "_badtype", type = "bogus");
});

// ── the message survives characters that would break naive CSV ─────────────
target = stamp & "_csv";
tricky = 'has a " quote, a comma and a #chr(9)#tab';
writeLog(text = tricky, file = target, type = "error");
assert("quotes/commas in the message round-trip", lastMessage(target), tricky);

// ── default target, and log= vs file= ──────────────────────────────────────
// No file=/log= ⇒ the application log. Asserted by marker presence rather than
// line counts: a long-lived Lucee install's application.log is megabytes, and
// other suites may write to it concurrently.
// `type=error` matters — Lucee's stock `application` logger is configured at
// ERROR, so an information-level line there is legitimately dropped.
marker = stamp & " default-target";
writeLog(text = marker, type = "error");
assertTrue(
    "no file=/log= writes to the application log",
    fileRead(logDir & "application.log") CONTAINS marker
);

// An unknown log= name falls back to `application` and does NOT create a file
// (whereas the same name via file= would). Verified on Lucee 7.
unknown = stamp & "_unknown_logger";
unknownMarker = stamp & " unknown-logger";
writeLog(text = unknownMarker, log = unknown, type = "error");
assertFalse(
    "an unknown log= name creates no file of its own",
    fileExists(logDir & unknown & ".log")
);
assertTrue(
    "an unknown log= name falls back to the application log",
    fileRead(logDir & "application.log") CONTAINS unknownMarker
);

// `felix` and `trace` have .log FILES in Lucee's log directory but are NOT
// configured loggers, so log= with those names still falls back to
// application. Verified empirically on Lucee 7.0.4 — reading the directory
// listing rather than the `loggers` config would get this wrong.
felixMarker = stamp & " felix-is-not-a-logger";
writeLog(text = felixMarker, log = "felix", type = "error");
assertTrue(
    "log=felix falls back to application (a log file, not a configured logger)",
    fileRead(logDir & "application.log") CONTAINS felixMarker
);

// A standard Lucee logger name via log= targets that log.
writeLog(text = stamp & " scheduler-line", log = "scheduler", type = "error");
assert(
    "log= with a standard logger name targets that log",
    lastMessage("scheduler"),
    stamp & " scheduler-line"
);

// ── a path separator in file= is refused, not traversed ────────────────────
assertThrows("file= containing / is an error", function() {
    writeLog(text = "x", file = "sub/" & stamp);
});
assertThrows("file= containing a backslash is an error", function() {
    writeLog(text = "x", file = "sub" & chr(92) & stamp);
});

// ── the tag form: attributes, and the application column ───────────────────
target = stamp & "_tag";
</cfscript>
<cflog text="via the tag" file="#target#" type="warning">
<cflog text="app column suppressed" file="#target#" type="warning" application="false">
<cfscript>
lines = logLines(target);
assertTrue("the <cflog> tag writes to file=", arrayLen(lines) GTE 3);
assert("tag form severity", csvFields(lines[2])[1], "WARN");
assert("tag form message", csvFields(lines[2])[7], "via the tag");

// The Application column carries the app name and `application="false"` blanks
// it (the attribute defaults to true in Lucee). Under the CLI runner there is
// no Application.cfc, so the name is empty either way — assert what we can
// portably: the column exists, and false never populates it.
assert("application=false leaves the Application column empty", csvFields(lines[3])[6], "");

// ── writeLog() argument forms agree ────────────────────────────────────────
// Positional signature is writeLog(text, type, application, file, log) —
// verified against Lucee 7.
target = stamp & "_pos";
writeLog("positional line", "fatal", "false", target);
fields = csvFields(logLines(target)[2]);
assert("positional writeLog severity", fields[1], "FATAL");
assert("positional writeLog message", fields[7], "positional line");

// The named form must bind by NAME, not by call order — writing file= before
// type= previously delivered the filename as the type.
target = stamp & "_named";
writeLog(text = "named line", file = target, type = "debug");
fields = csvFields(logLines(target)[2]);
assert("named writeLog binds file= and type= by name", fields[1], "DEBUG");
assert("named writeLog message", fields[7], "named line");

// ── clean up this run's files ──────────────────────────────────────────────
for (f in directoryList(logDir, false, "name")) {
    if (left(f, len(stamp)) EQ stamp) {
        try { fileDelete(logDir & f); } catch (any e) {}
    }
}

suiteEnd();
</cfscript>
