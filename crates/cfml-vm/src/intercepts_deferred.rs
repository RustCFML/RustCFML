//! Builtin names that `call_function` resolves LATER in its own body (they need VM
//! state that is not available at the intercept point).
//!
//! These were previously spelled out as a **151-line `match` arm whose entire body was the
//! comment "Will be handled at the end of this function"** — 149 lines of `| "name"`
//! patterns existing only to stop the `_` arm from claiming them. Listing them as data
//! says the same thing in a form you can read, search and diff.
//!
//! Behaviour is unchanged: matching a name here still falls through to the later handling.

/// Names deferred to the VM-access handling further down `call_function`.
pub(crate) const DEFERRED_TO_VM: &[&str] = &[
    "arraymap", "arrayfilter", "arrayreduce", "arrayeach",
    // NOTE: arrayFindAll/arrayFindAllNoCase are deliberately NOT here. Their
    // closure-predicate form is claimed by the `arrayfind*` arm ABOVE the
    // deferred arm, so deferring them only diverted the VALUE-needle form past
    // the builtin dispatch and into "function is not defined" (GH #358 notes).
    "arraysome", "arrayevery",
    "structeach", "structmap", "structfilter", "structreduce",
    "structsome", "structevery", "listeach", "listmap",
    "listfilter", "listreduce", "listsome", "listevery",
    "listreduceright", "stringeach", "stringmap", "stringfilter",
    "stringreduce", "stringsome", "stringevery", "stringsort",
    "collectioneach", "collectionmap", "collectionfilter", "collectionreduce",
    "collectionsome", "collectionevery", "each", "queryeach",
    "querymap", "queryfilter", "queryreduce", "querysort",
    "querysome", "queryevery", "queryaddrow", "querysetcell",
    "createobject", "getcurrenttemplatepath", "getmetadata", "getcomponentmetadata",
    "getcomponentstaticscope", "getapplicationmetadata", "getapplicationsettings", "__cfheader",
    "__cfapplication", "__cfcontent", "__cflocation", "__cfabort",
    "__cfexit", "__cfhtmlhead", "__cfhtmlbody", "gethttprequestdata",
    "__cfinvoke", "__cfsavecontent_start", "__cfsavecontent_end", "invoke",
    "getbasetemplatepath", "getfunctioncalledname", "gettimezone", "sleep",
    "settimezone", "getlocale", "setlocale", "gettimezoneinfo",
    "dateconvert", "expandpath", "sanitizehtml", "isdefined",
    "setencoding", "__cfparam", "queryexecute", "cfdbinfo",
    "dbinfo", "cfhttp", "queryregisterfunction", "__cftransaction_start",
    "__cftransaction_commit", "__cftransaction_rollback", "__cftransaction_end", "__writetext",
    "__cflog", "writelog", "__cfsetting", "__cflock_start",
    "__cflock_end", "__cfcookie", "fileupload", "fileuploadall",
    "__cffile_upload", "sessioninvalidate", "sessionrotate", "sessioncommit",
    "sessiongetmetadata", "applicationstop", "getauthuser", "csrfgeneratetoken",
    "csrfverifytoken", "isuserinrole", "isuserloggedin", "__cfloginuser",
    "__cflogout", "setvariable", "throw",
    "__cfcustomtag", "__cfmodule", "__cfcustomtag_start", "__cfcustomtag_end",
    "cacheput", "cacheget", "cachedelete", "cacheclear",
    "cachekeyexists", "cachecount", "cachegetall", "cachegetallids",
    "cachegetproperties", "__cfcache", "__cfloop_file_lines", "__cfloop_file_open",
    "__cfloop_file_next", "__cfloop_file_close", "__cfexecute",
    "__cfthread_run", "__cfthread_join", "__cfthread_terminate", "threadjoin",
    "threadterminate", "runasync", "_schedule", "createdynamicproxy",
    "callstackget", "callstackdump", "isinthread", "getpagecontext",
    "getbasetaglist", "getbasetagdata", "evaluate", "precisionevaluate",
];

/// True if `name_lower` is resolved later in `call_function` rather than at the intercept
/// point. Linear scan over a small static slice; the list is unsorted because it mirrors
/// the original arm's grouping, which is the useful order for a reader.
#[inline]
pub(crate) fn is_deferred(name_lower: &str) -> bool {
    DEFERRED_TO_VM.contains(&name_lower)
}
