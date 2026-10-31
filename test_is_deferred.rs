// Quick test to verify is_deferred works for threaddata
use std::collections::HashSet;

const DEFERRED_TO_VM: &[&str] = &[
    "arraymap", "arrayfilter", "arrayreduce", "arrayeach",
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
    "expandpath", "sanitizehtml", "isdefined",
    "setencoding", "__cfparam", "queryexecute", "cfdbinfo",
    "dbinfo", "cfhttp", "queryregisterfunction", "__cftransaction_start",
    "__cftransaction_commit", "__cftransaction_rollback", "__cftransaction_end", "__writetext",
    "__cflog", "writelog", "__cfsetting", "__cflock_start",
    "__cflock_end", "__cfcookie",
    "sessioninvalidate", "sessionrotate", "sessioncommit",
    "sessiongetmetadata", "applicationstop", "getauthuser", "csrfgeneratetoken",
    "csrfverifytoken", "isuserinrole", "isuserloggedin", "__cfloginuser",
    "__cflogout", "setvariable", "throw",
    "__cfcustomtag", "__cfmodule", "__cfcustomtag_start", "__cfcustomtag_end",
    "cacheput", "cacheget", "cachedelete", "cacheclear",
    "cachekeyexists", "cachecount", "cachegetall", "cachegetallids",
    "cachegetproperties", "__cfcache", "__cfloop_file_lines", "__cfloop_file_open",
    "__cfloop_file_next", "__cfloop_file_close", "__cfexecute",
    "__cfthread_run", "__cfthread_join", "__cfthread_terminate", "threadjoin",
    "threadterminate", "threaddata", "runasync", "_schedule", "createdynamicproxy",
    "callstackget", "callstackdump", "isinthread", "getpagecontext",
    "getbasetaglist", "getbasetagdata", "evaluate", "precisionevaluate",
];

fn is_deferred(name_lower: &str) -> bool {
    static SORTED: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| {
        let mut v: Vec<&'static str> = DEFERRED_TO_VM.to_vec();
        v.sort_unstable();
        v
    });
    SORTED.binary_search(&name_lower).is_ok()
}

fn main() {
    // Test that threaddata is in the list
    println!("threaddata in DEFERRED_TO_VM: {}", DEFERRED_TO_VM.contains(&"threaddata"));
    println!("is_deferred('threaddata'): {}", is_deferred("threaddata"));
    println!("is_deferred('threadData'): {}", is_deferred("threadData"));
    println!("is_deferred('THREADDATA'): {}", is_deferred("THREADDATA"));
    println!("is_deferred('ThreadData'): {}", is_deferred("ThreadData"));
    
    // Check for duplicates
    let set: HashSet<&str> = DEFERRED_TO_VM.iter().copied().collect();
    println!("DEFERRED_TO_VM length: {}", DEFERRED_TO_VM.len());
    println!("Unique items: {}", set.len());
    if set.len() != DEFERRED_TO_VM.len() {
        println!("WARNING: Duplicates found!");
    }
    
    // Print sorted list to verify threaddata is there
    let mut sorted = DEFERRED_TO_VM.to_vec();
    sorted.sort_unstable();
    println!("\nSorted list:");
    for (i, name) in sorted.iter().enumerate() {
        if name.contains("thread") {
            println!("{}: {}", i, name);
        }
    }
}
