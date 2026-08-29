//! Declared metadata about built-in functions — the SINGLE enumerable source of
//! truth for "what is a builtin" and "which builtins are VM-intercepted".
//!
//! # Why this exists
//! `CfmlVirtualMachine::call_function` grew to **7,496 lines (20% of `cfml-vm/src/lib.rs`)**
//! because the documented recipe for adding a VM-intercepted builtin was "append a
//! `name_lower == "..."` branch to it", and nothing ever removed one. Cross-cutting
//! concerns (`sandbox_intercept`, `s3_intercept`, `resolve_file_bif_paths`) were then
//! woven into the same chain, so the question *"is this name intercepted?"* became
//! unanswerable without executing the function. That blocked compile-time BIF binding —
//! a wrong answer does not merely run slow, it **silently bypasses the sandbox**.
//!
//! Lucee does the opposite: it DECLARES its BIFs (`FunctionLibFunction` carries the
//! implementing class and argument types, read at compile time). Declarations stay
//! enumerable; chains do not. This module is that declaration.
//!
//! # The closing mechanism (the part that matters)
//! Both lists are guarded by tests that check them against reality, so they cannot drift:
//!   * `cfml-stdlib` asserts [`BUILTIN_NAMES`] equals the registration table exactly.
//!   * `cfml-vm` scans `call_function` and asserts every name it compares against is in
//!     [`VM_INTERCEPTED`].
//! Appending an intercept without declaring it therefore FAILS THE BUILD. Without that
//! guard these lists would rot exactly like the chain they replace.
//!
//! # Safety asymmetry — read before editing
//! **Over-declaring an intercept is safe** (the name simply is not compile-time bound, so
//! it keeps the old dispatch). **Under-declaring is not** (interception is skipped). When
//! in doubt, declare it intercepted.

/// Every name registered by `cfml_stdlib::builtins::get_builtin_functions()`, lowercased
/// and sorted. Guarded by a test in `cfml-stdlib`.
pub const BUILTIN_NAMES: &[&str] = &[
    "$siobroadcast", "$siodisconnect", "$siogetdata", "$siojoinroom", "$sioleaveallrooms",
    "$sioleaveroom", "$sioregisterednamespaces", "$sioregisternamespace", "$sioregisternshandler",
    "$sioregistersockethandler", "$siosend", "$siosetdata", "$siosocketcount", "__cfabort",
    "__cfapplication", "__cfcache", "__cfcontent", "__cfcookie", "__cfexecute", "__cfexit",
    "__cffile_upload", "__cfheader", "__cfhtmlbody", "__cfhtmlhead", "__cfinvoke", "__cflocation",
    "__cflock_end", "__cflock_start", "__cflog", "__cfloginuser", "__cflogout",
    "__cfloop_file_close", "__cfloop_file_lines", "__cfloop_file_next",
    "__cfloop_file_open", "__cfmail", "__cfparam", "__cfparam_validate",
    "__cfprocessingdirective_collapse", "__cfsavecontent_end", "__cfsavecontent_start",
    "__cfsetting", "__cfthread_join", "__cfthread_run", "__cfthread_terminate",
    "__cftransaction_commit", "__cftransaction_end", "__cftransaction_rollback",
    "__cftransaction_start", "__dbinfo_impl", "__jsonschemavalidateresult", "__querysetrow",
    "__register_ds_timeout", "__writetext", "_schedule", "abs", "acos", "applicationstop",
    "argon2checkhash", "arrayappend", "arrayavg", "arrayclear", "arraycontains",
    "arraycontainsnocase", "arraydelete", "arraydeleteat", "arraydeletenocase", "arrayeach",
    "arrayevery", "arrayfilter", "arrayfind", "arrayfindall", "arrayfindallnocase",
    "arrayfindnocase", "arrayfirst", "arrayindexexists", "arrayinsertat", "arrayisdefined",
    "arrayisempty", "arraylast", "arraylen", "arraymap", "arraymax", "arraymedian", "arraymerge",
    "arraymid", "arraymin", "arraynew", "arraypop", "arrayprepend", "arraypush", "arrayrange",
    "arrayreduce", "arrayreduceright", "arrayresize", "arrayreverse", "arrayset", "arrayshift",
    "arrayslice", "arraysome", "arraysort", "arraysplice", "arraysum", "arrayswap", "arraytolist",
    "arraytostruct", "arrayunshift", "asc", "asin", "assertbroadcast", "atan", "bcrypthash",
    "bcryptverify", "binarydecode", "binaryencode", "bitand", "bitmaskclear", "bitmaskread",
    "bitmaskset", "bitnot", "bitor", "bitshln", "bitshrn", "bitxor", "booleanformat",
    "cacheclear", "cachecount", "cachedelete", "cacheget", "cachegetall", "cachegetallids",
    "cachegetproperties", "cachekeyexists", "cacheput", "canonicalize", "ceiling", "cfdbinfo",
    "cfdirectory", "cfdump", "cffile", "cfhttp", "cfimage", "cfzip", "charsetdecode",
    "charsetencode", "chr", "cjustify", "collectioneach", "collectionevery", "collectionfilter",
    "collectionmap", "collectionreduce", "collectionsome", "compare", "comparenocase", "cos",
    "createdate", "createdatetime", "createdynamicproxy", "createguid", "createobject",
    "createodbcdate", "createodbcdatetime", "createodbctime", "createtime", "createtimespan",
    "createuniqueid", "createuuid", "csrfgeneratetoken", "csrfverifytoken", "csvformatrow",
    "dateadd", "datecompare", "dateconvert", "datediff", "dateformat", "datepart",
    "datetimeformat", "day", "dayofweek", "dayofweekasstring", "dayofweekshortasstring",
    "dayofyear", "daysinmonth", "daysinyear", "dbinfo", "de", "debugadd", "decimalformat",
    "decodeforhtml", "decodefromurl", "decrementvalue", "decrypt", "deserializejson",
    "directorycopy", "directorycreate", "directorydelete", "directoryexists", "directorylist",
    "directoryrename", "dollarformat", "dump", "duplicate", "each", "echo", "encodefor",
    "encodeforcss", "encodeforhtml", "encodeforhtmlattribute", "encodeforjavascript",
    "encodeforurl", "encodeforxml", "encodeforxmlattribute", "encrypt", "evaluate",
    "exceptionkeyexists", "exp", "expandpath", "fileappend", "fileclose", "filecopy",
    "filedelete", "fileexists", "filegetmimetype", "fileiseof", "filemove", "fileopen",
    "fileread", "filereadbinary", "filereadline", "filesetaccessmode", "filesetattribute",
    "filesetlastmodified", "fileupload", "fileuploadall", "filewrite", "filewriteline", "find",
    "findnocase", "findoneof", "firstdayofmonth", "fix", "floor", "formatbasen",
    "generateargon2hash", "generatebcrypthash", "generatepbkdfkey", "generatescrypthash",
    "generatesecretkey", "getapplicationmetadata", "getapplicationsettings", "getauthuser",
    "getbasetagdata", "getbasetaglist", "getbasetemplatepath", "getcanonicalpath",
    "getcomponentmetadata", "getcomponentstaticscope", "getcontextroot", "getcurrenttemplatepath",
    "getdebugdata", "getdirectoryfrompath", "getenvironmentvariable", "getfilefrompath",
    "getfileinfo", "getfunctioncalledname", "getfunctionlist", "gethttprequestdata",
    "gethttptimestring", "getlocale", "getmetadata", "getnumericdate", "getpagecontext",
    "getprofilesections", "getprofilestring", "getreadableimageformats", "getrequestprofile",
    "gettagdata", "gettempdirectory", "gettempfile", "gettemplatepath", "gettickcount",
    "gettimezone", "gettimezoneinfo", "gettoken", "getvariable", "getwriteableimageformats",
    "hash", "hmac", "hour", "htmlcodeformat", "htmldocument", "htmleditformat", "htmlparse",
    "iif", "imageaddborder", "imageblur", "imageclearrect", "imagecopy", "imagecrop",
    "imagedrawarc", "imagedrawbeveledrect", "imagedrawcubiccurve", "imagedrawimage",
    "imagedrawline", "imagedrawlines", "imagedrawoval", "imagedrawpoint",
    "imagedrawquadraticcurve", "imagedrawrect", "imagedrawroundrect", "imagedrawtext",
    "imageflip", "imagegetblob", "imagegetbufferedimage", "imagegetexifmetadata",
    "imagegetexiftag", "imagegetheight", "imagegetiptcmetadata", "imagegetiptctag",
    "imagegetwidth", "imagegrayscale", "imageinfo", "imagemakecolortransparent",
    "imagemaketranslucent", "imagenegative", "imagenew", "imageoverlay", "imagepaste",
    "imageread", "imagereadbase64", "imagereadsvg", "imageresize", "imagerotate",
    "imagerotatedrawingaxis", "imagescaletofit", "imagesetantialiasing",
    "imagesetbackgroundcolor", "imagesetdrawingcolor", "imagesetdrawingstroke",
    "imagesetdrawingtransparency", "imagesharpen", "imageshear", "imagesheardrawingaxis",
    "imagetranslate", "imagetranslatedrawingaxis", "imagewrite", "imagewritebase64",
    "imagexordrawingmode", "incrementvalue", "inputbasen", "insert", "int", "invoke", "io",
    "isarray", "isbinary", "isboolean", "isclosure", "iscustomfunction", "isdate", "isdebugmode",
    "isdefined", "isempty", "isimage", "isimagefile", "isinstanceof", "isinthread", "isjson",
    "isleapyear", "isnull", "isnumeric", "isobject", "ispdfobject", "isquery", "issimplevalue",
    "isspreadsheetfile", "isspreadsheetobject", "isstruct", "isuserinrole", "isuserloggedin",
    "isvalid", "isxml", "isxmlattribute", "isxmldoc", "isxmlelem", "isxmlnode", "isxmlroot",
    "javacast", "jsstringformat", "jwtdecode", "jwtsign", "jwtverify", "lcase", "left", "len",
    "listappend", "listavg", "listchangedelims", "listcompact", "listcontains",
    "listcontainsnocase", "listdeleteat", "listeach", "listevery", "listfilter", "listfind",
    "listfindnocase", "listfirst", "listgetat", "listindexexists", "listinsertat", "listitemtrim",
    "listlast", "listlen", "listmap", "listnew", "listprepend", "listqualify", "listreduce",
    "listreduceright", "listremoveduplicates", "listrest", "listsetat", "listsome", "listsort",
    "listtoarray", "listvaluecount", "listvaluecountnocase", "ljustify", "log", "log10",
    "lscurrencyformat", "lsdateformat", "lsdatetimeformat", "lsdayofweek", "lseurocurrencyformat",
    "lsiscurrency", "lsisdate", "lsisnumeric", "lsnumberformat", "lsparsecurrency",
    "lsparsedatetime", "lsparsenumber", "lstimeformat", "lsweek", "ltrim", "max", "metaphone",
    "mid", "millisecond", "min", "minute", "month", "monthasstring", "monthshortasstring",
    "newline", "now", "nowserver", "nullvalue", "numberformat", "objectload", "objectsave",
    "paragraphformat", "parsedatetime", "pdf", "pdfinfo", "pdfpagecount", "pdfread", "pdftoimage",
    "pi", "pow", "preservesinglequotes", "profilenow", "qrcodegenerate", "quarter",
    "queryaddcolumn", "queryaddrow", "queryappend", "querycolumnarray", "querycolumncount",
    "querycolumndata", "querycolumnexists", "querycolumnlist", "querycurrentrow",
    "querydeletecolumn", "querydeleterow", "queryeach", "queryevery", "queryexecute",
    "queryfilter", "querygetcell", "querygetresult", "querygetrow", "queryinsertat",
    "querykeyexists", "querymap", "querynew", "queryprepend", "queryrecordcount", "queryreduce",
    "queryregisterfunction", "queryreverse", "queryrowdata", "queryrowswap", "querysetcell",
    "querysetrow", "queryslice", "querysome", "querysort", "quotedvaluelist", "rand",
    "randombytes", "randomize", "randrange", "readline", "reescape", "refind", "refindnocase",
    "rematch", "rematchnocase", "removechars", "repeatstring", "replace", "replacelist",
    "replacelistnocase", "replacenocase", "rereplace", "rereplacenocase", "reverse", "right",
    "rjustify", "round", "rtrim", "runasync", "s3clearbucket", "s3copy", "s3createbucket",
    "s3delete", "s3download", "s3exists", "s3generatepresignedurl", "s3generateuri",
    "s3getmetadata", "s3listbucket", "s3listbuckets", "s3move", "s3read", "s3readbinary",
    "s3upload", "s3write", "sanitizehtml", "second", "serialize", "serializejson",
    "sessioncommit", "sessiongetmetadata", "sessioninvalidate", "sessionrotate", "setencoding",
    "setlocale", "setprofilestring", "settimezone", "setvariable", "sgn", "sin", "sleep",
    "smtpconnectiontest", "soundex", "spanexcluding", "spanincluding", "spreadsheet",
    "spreadsheetaddautofilter", "spreadsheetaddchart", "spreadsheetaddcolumn",
    "spreadsheetaddconditionalformatting", "spreadsheetadddatavalidation",
    "spreadsheetaddfreezepane", "spreadsheetaddimage", "spreadsheetaddinfo",
    "spreadsheetaddpagebreaks", "spreadsheetaddrow", "spreadsheetaddrows",
    "spreadsheetaddsplitpane", "spreadsheetautosizecolumn", "spreadsheetclearcell",
    "spreadsheetclearcellrange", "spreadsheetcreatesheet", "spreadsheetdeletecolumn",
    "spreadsheetdeletecolumns", "spreadsheetdeleterow", "spreadsheetdeleterows",
    "spreadsheetformatcell", "spreadsheetformatcellrange", "spreadsheetformatcolumn",
    "spreadsheetformatrow", "spreadsheetfromjson", "spreadsheetgetcellcomment",
    "spreadsheetgetcellformat", "spreadsheetgetcellformula", "spreadsheetgetcellhyperlink",
    "spreadsheetgetcelltype", "spreadsheetgetcellvalue", "spreadsheetgetcolumncount",
    "spreadsheetgetcolumnwidth", "spreadsheetinfo", "spreadsheetmergecells", "spreadsheetnew",
    "spreadsheetread", "spreadsheetreadbinary", "spreadsheetreadcsv", "spreadsheetrenamesheet",
    "spreadsheetsetactivecell", "spreadsheetsetactivesheet", "spreadsheetsetactivesheetnumber",
    "spreadsheetsetcellcomment", "spreadsheetsetcellformula", "spreadsheetsetcellhyperlink",
    "spreadsheetsetcellrangevalue", "spreadsheetsetcellvalue", "spreadsheetsetcolumnhidden",
    "spreadsheetsetcolumnwidth", "spreadsheetsetfittopage", "spreadsheetsetfooter",
    "spreadsheetsetheader", "spreadsheetsetprintorientation", "spreadsheetsetrepeatingcolumns",
    "spreadsheetsetrepeatingrows", "spreadsheetsetrowheight", "spreadsheetsetrowhidden",
    "spreadsheetshiftcolumns", "spreadsheetshiftrows", "spreadsheettoarray", "spreadsheettocsv",
    "spreadsheettojson", "spreadsheettoquery", "spreadsheetwrite", "spreadsheetwritetocsv", "sqr",
    "sqrt", "storegetmetadata", "stringeach", "stringevery", "stringfilter", "stringmap",
    "stringreduce", "stringsome", "stringsort", "stripcr", "structappend", "structclear",
    "structcopy", "structcount", "structdelete", "structeach", "structequals", "structevery",
    "structfilter", "structfind", "structfindkey", "structfindvalue", "structget",
    "structgetmetadata", "structinsert", "structiscasesensitive", "structisempty",
    "structisordered", "structkeyarray", "structkeyexists", "structkeylist", "structkeytranslate",
    "structmap", "structnew", "structreduce", "structsetmetadata", "structsome", "structsort",
    "structtoquerystring", "structtosorted", "structupdate", "structvaluearray",
    "systemcacheclear", "systemoutput", "tan", "threadjoin", "threadterminate", "throw",
    "timeformat", "tobase64", "tobinary", "toboolean", "tonumeric", "toscript", "tostring",
    "trace", "trim", "truefalseformat", "ucase", "ucfirst", "urldecode", "urlencode",
    "urlencodedformat", "val", "validatejson", "valuearray", "valuelist", "verifybcrypthash",
    "verifyscrypthash", "week", "wrap", "writedump", "writelog", "writeoutput", "wspresence",
    "wspublish", "wssubscribe", "wsunsubscribe", "xmlchildpos", "xmlelemnew", "xmlformat",
    "xmlgetnodetype", "xmlhaschild", "xmlnew", "xmlparse", "xmlsearch", "xmltransform",
    "xmlvalidate", "xmpparse", "yamldeserialize", "yamldeserializefile", "yamlserialize", "year",
    "yesnoformat",
];

/// Every name `call_function` dispatches on before reaching the generic builtin path,
/// lowercased and sorted. Includes internal `__`-prefixed tag helpers that are not
/// registered builtins. Guarded by a source-scanning test in `cfml-vm`.
pub const VM_INTERCEPTED: &[&str] = &[
    "$siobroadcast", "$siodisconnect", "$siogetdata", "$siojoinroom", "$sioleaveallrooms",
    "$sioleaveroom", "$sioregisterednamespaces", "$sioregisternamespace", "$sioregisternshandler",
    "$sioregistersockethandler", "$siosend", "$siosetdata", "$siosocketcount", "__cfabort",
    "__cfapplication", "__cfcache", "__cfcontent", "__cfcookie", "__cfcustomtag",
    "__cfcustomtag_end", "__cfcustomtag_start", "__cfdirectory", "__cfdocument", "__cfdump",
    "__cfexecute", "__cfexit", "__cffile_upload", "__cfheader", "__cfhtmlbody", "__cfhtmlhead",
    "__cfhttp", "__cfinvoke", "__cflocation", "__cflock_end", "__cflock_start", "__cflog",
    "__cfloginuser", "__cflogout", "__cfloop_file_close", "__cfloop_file_lines",
    "__cfloop_file_next", "__cfloop_file_open", "__cfmodule", "__cfparam",
    "__cfsavecontent_end", "__cfsavecontent_start", "__cfsetting", "__cfspreadsheet",
    "__cfthread_join", "__cfthread_run", "__cfthread_terminate", "__cftransaction_commit",
    "__cftransaction_end", "__cftransaction_rollback", "__cftransaction_start", "__variables",
    "__writetext", "_schedule", "abort", "application", "applicationstop", "arguments", "array",
    "arraycontains", "arraydelete", "arraydeletenocase", "arrayeach", "arrayevery", "arrayfilter",
    "arrayfind", "arrayfindall", "arrayfindallnocase", "arrayfindnocase", "arraymap",
    "arrayreduce", "arraysome", "arraysort", "assertbroadcast", "attributes", "bcrypt",
    "cacheclear", "cachecount", "cachedelete", "cacheget", "cachegetall", "cachegetallids",
    "cachegetproperties", "cachekeyexists", "cacheput", "callstackdump", "callstackget",
    "cfabort", "cfcache", "cfcatch", "cfcontent", "cfcookie", "cfdbinfo", "cfdirectory",
    "cfdocument", "cfdump", "cfexecute", "cffile", "cfheader", "cfhtmlbody", "cfhtmlhead",
    "cfhttp", "cfimage", "cfinvoke", "cflocation", "cflog", "cfmodule", "cfpdf", "cfsetting",
    "cfspreadsheet", "cfzip", "cgi", "client", "collectioneach", "collectionevery",
    "collectionfilter", "collectionmap", "collectionreduce", "collectionsome", "component",
    "cookie", "createdynamicproxy", "createobject", "csrfgeneratetoken", "csrfverifytoken",
    "dateconvert", "dbinfo", "debugadd", "delete", "directory", "directorycopy",
    "directorycreate", "directorydelete", "directoryexists", "directorylist", "directoryrename",
    "dump", "each", "echo", "evaluate", "expand", "expandpath", "false", "file", "fileappend",
    "filecopy", "filedelete", "fileexists", "filemove", "fileopen", "fileread", "filereadbinary",
    "filesetaccessmode", "filesetattribute", "filesetlastmodified", "fileupload", "fileuploadall",
    "filewrite", "filewritebinary", "filewriteline", "form", "getapplicationmetadata",
    "getapplicationsettings", "getauthuser", "getbasetagdata", "getbasetaglist",
    "getbasetemplatepath", "getcomponentmetadata", "getcomponentstaticscope",
    "getcurrenttemplatepath", "getdebugdata", "getfileinfo", "getfunctioncalledname",
    "getfunctionlist", "gethttprequestdata", "getlocale", "getmetadata", "getpagecontext",
    "getprofilesections", "getprofilestring", "getrequestprofile", "gettimezone",
    "gettimezoneinfo", "getvariable", "html", "image", "imagewrite", "imagewritebase64", "invoke",
    "io", "isdebugmode", "isdefined", "isinthread", "isuserinrole", "isuserloggedin", "json",
    "label", "listeach", "listevery", "listfilter", "listmap", "listreduce", "listreduceright",
    "listsome", "local", "local2utc", "location", "name", "no", "numeric", "output",
    "precisionevaluate", "profilenow", "queryaddrow", "queryappend", "queryeach", "queryevery",
    "queryexecute", "queryfilter", "querymap", "queryreduce", "queryregisterfunction",
    "querysetcell", "querysetrow", "querysome", "querysort", "request", "runasync",
    "sanitizehtml", "server", "session", "sessioncommit", "sessiongetmetadata",
    "sessioninvalidate", "sessionrotate", "setencoding", "setlocale", "setprofilestring",
    "settimezone", "setvariable", "sleep", "spreadsheetwrite", "static", "string", "stringeach",
    "stringevery", "stringfilter", "stringmap", "stringreduce", "stringsome", "stringsort",
    "structeach", "structevery", "structfilter", "structget", "structmap", "structreduce",
    "structsome", "this", "thread", "threadjoin", "threadterminate", "throw", "top", "trace",
    "true", "url", "utc2local", "var", "variables", "writedump", "writelog", "writeoutput",
    "wspresence", "wspublish", "wssubscribe", "wsunsubscribe", "yes",
];

/// True if `lower` (already lowercased) is dispatched by the VM before the generic
/// builtin path, and therefore must NOT be compile-time bound.
///
/// The `$sio` family is matched by PREFIX in the chain (`name_lower.starts_with("$sio")`),
/// so it is handled as a prefix here rather than enumerated.
#[inline]
pub fn is_vm_intercepted(lower: &str) -> bool {
    lower.starts_with("$sio") || VM_INTERCEPTED.binary_search(&lower).is_ok()
}

/// True if `lower` is a registered builtin that the VM does NOT intercept — i.e. it is
/// safe for codegen to bind at compile time.
///
/// Note that "the VM intercepts it" covers more than a handler in the dispatch
/// chain: a builtin that CREATES OR REMOVES A FILE must be declared intercepted
/// too, because `call_function` does bookkeeping *around* such a call (retiring
/// cached negative existence answers, flushing compiled-template caches) that a
/// compile-time-bound call skips entirely. `imageWrite` was not declared, so
/// `if ( !fileExists( t ) ) { imageWrite( img, t ); } fileExists( t )` answered
/// false with the file sitting on disk.
#[inline]
pub fn is_pure_builtin(lower: &str) -> bool {
    BUILTIN_NAMES.binary_search(&lower).is_ok() && !is_vm_intercepted(lower)
}

// ---------------------------------------------------------------------------
// Extension-provided builtins
// ---------------------------------------------------------------------------

/// Names contributed by dynamically loaded `.rcx` extensions, lowercased.
///
/// Codegen needs this to bind an extension's BIF at compile time exactly as it
/// binds a compiled-in one — the difference between ~325 ns and ~130 ns per
/// call, because the generic path (a `LoadGlobal`, the locals/`variables`/
/// globals chain walk, a per-call `to_lowercase`, and `call_function`'s
/// intercept chain) dwarfs the ABI crossing itself.
///
/// Safe to consult from codegen because extensions load **once, at process
/// start, before anything is compiled**, and are never unloaded (there is no
/// `dlclose`). The set is therefore write-once-then-read-only in practice, and
/// the VM's `CallBuiltin` handler still falls back to generic resolution if a
/// name it was told about is somehow absent — so a stale bytecode cache degrades
/// in speed, never in correctness.
static FOREIGN_BUILTINS: std::sync::RwLock<Option<std::collections::BTreeSet<String>>> =
    std::sync::RwLock::new(None);

/// Record the BIF names a loaded extension provides. Called by the loader,
/// before the first template is compiled.
pub fn register_foreign_builtin_names<I, S>(names: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut guard = FOREIGN_BUILTINS.write().unwrap_or_else(|e| e.into_inner());
    let set = guard.get_or_insert_with(std::collections::BTreeSet::new);
    for n in names {
        set.insert(n.as_ref().to_ascii_lowercase());
    }
    ANY_FOREIGN.store(!set.is_empty(), std::sync::atomic::Ordering::Relaxed);
}

/// True if `lower` is a BIF provided by a loaded extension.
///
/// The `is_none()` fast path matters: with no extensions loaded — the
/// overwhelmingly common case — this is one relaxed read and no lock contention
/// on a path codegen walks for every call site it compiles.
#[inline]
pub fn is_foreign_builtin(lower: &str) -> bool {
    if !ANY_FOREIGN.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    FOREIGN_BUILTINS
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|s| s.contains(lower)))
        .unwrap_or(false)
}

/// Set once any extension registers a name, so the common "no extensions"
/// case never takes the lock.
static ANY_FOREIGN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Every extension-provided BIF name, for diagnostics.
pub fn foreign_builtin_names() -> Vec<String> {
    FOREIGN_BUILTINS
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|s| s.iter().cloned().collect()))
        .unwrap_or_default()
}


#[cfg(test)]
mod foreign_builtin_tests {
    use super::*;

    #[test]
    fn extension_names_become_compile_time_bindable() {
        // Nothing registered: the fast path must answer without taking a lock.
        assert!(!is_foreign_builtin("zzz_not_an_extension_bif"));

        register_foreign_builtin_names(["demoGreet", "DEMOSTATS"]);
        // Matched lowercased, like every CFML identifier.
        assert!(is_foreign_builtin("demogreet"));
        assert!(is_foreign_builtin("demostats"));
        assert!(!is_foreign_builtin("demogreets"));

        // An extension name must NOT be mistaken for a compiled-in one: the two
        // registries are consulted separately, and only the compiled-in table
        // can be dispatched through a bare fn pointer.
        assert!(!is_pure_builtin("demogreet"));
        assert!(is_pure_builtin("len"));

        assert!(foreign_builtin_names().contains(&"demogreet".to_string()));
    }
}


#[cfg(test)]
mod sortedness {
    use super::*;

    /// Both lists are queried with `binary_search`, which is only correct on a
    /// SORTED slice — and nothing was checking that.
    ///
    /// It had rotted: six out-of-order pairs in `BUILTIN_NAMES` made 55 names
    /// unfindable, so `is_pure_builtin` answered false for the whole `query*`
    /// family, `queryExecute`, `pi`, `pow`, `createUUID` and the `pdf*` set —
    /// silently excluding every one of them from compile-time binding and the
    /// ~200 ns/call it saves. The failure direction was benign (a missed
    /// optimisation, never a wrong dispatch), which is exactly why it went
    /// unnoticed: nothing breaks, things are just slower.
    ///
    /// For `VM_INTERCEPTED` the same rot would NOT be benign — a declared-
    /// intercepted name that the search misses becomes compile-time bound and
    /// skips its interception, which is the sandbox-bypass shape. Hence a test
    /// rather than a convention.
    #[test]
    fn declaration_lists_are_sorted() {
        for (label, list) in [("BUILTIN_NAMES", BUILTIN_NAMES), ("VM_INTERCEPTED", VM_INTERCEPTED)]
        {
            let bad: Vec<_> = list.windows(2).filter(|w| w[0] >= w[1]).collect();
            assert!(
                bad.is_empty(),
                "{} must be sorted for binary_search; out of order at {:?}",
                label,
                bad
            );
        }
    }

    /// Every declared name must be findable by the search the code actually
    /// uses, which is a stronger statement than "sorted" and the property that
    /// was really broken.
    #[test]
    fn every_declared_name_is_findable() {
        for name in BUILTIN_NAMES {
            assert!(BUILTIN_NAMES.binary_search(name).is_ok(), "unfindable: {name}");
        }
        for name in VM_INTERCEPTED {
            assert!(is_vm_intercepted(name), "declared intercepted but not detected: {name}");
        }
    }
}
