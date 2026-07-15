<cfscript>
suiteBegin("Tags: cfdirectory sort");

// cfdirectory action="list" must honour the `sort` attribute (Lucee/ACF) rather
// than returning entries in OS enumeration order. Masa CMS runs its schema
// migrations via `<cfdirectory sort="name asc">` then a driving loop, so an
// unsorted listing runs migrations out of order.
function names(dir, sortSpec) {
    var q = "";
    if (len(sortSpec)) {
        q = directoryListQuery(dir, sortSpec);
    }
    var out = [];
    for (var row in q) { arrayAppend(out, row.name); }
    return arrayToList(out, ",");
}

// Helper: <cfdirectory> tag form with a sort, returning the query.
function directoryListQuery(dir, sortSpec) {
    var rs = "";
    cfdirectory(action="list", directory=dir, filter="*.txt", sort=sortSpec, name="rs");
    return rs;
}

tmp = getTempDirectory() & "rustcfml_cfdirsort_" & createUUID() & "/";
directoryCreate(tmp);
// Written in a deliberately non-sorted order.
fileWrite(tmp & "5.4.3178.txt", "x");
fileWrite(tmp & "5.0.629.txt", "x");
fileWrite(tmp & "5.1.288.txt", "x");
fileWrite(tmp & "5.10.1.txt", "x");

try {
    assert("sort='name asc' orders ascending by name (textual)",
        names(tmp, "name asc"), "5.0.629.txt,5.1.288.txt,5.10.1.txt,5.4.3178.txt");
    assert("sort='name desc' orders descending by name",
        names(tmp, "name desc"), "5.4.3178.txt,5.10.1.txt,5.1.288.txt,5.0.629.txt");
    // Bare column name defaults to ascending.
    assert("sort='name' defaults to ascending",
        names(tmp, "name"), "5.0.629.txt,5.1.288.txt,5.10.1.txt,5.4.3178.txt");
} catch (any e) {
    assert("cfdirectory sort test error", "ERROR: " & e.message, "no error");
}
directoryDelete(tmp, true);

suiteEnd();
</cfscript>
