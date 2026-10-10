//! Declared parameter names for built-in functions, so a call written with
//! named arguments binds BY NAME.
//!
//! # Why this exists
//! Builtins are registered as bare `fn(Vec<CfmlValue>) -> CfmlResult`: the
//! implementation sees positions, and nothing anywhere recorded what those
//! positions are called. A named call therefore bound in CALL ORDER, so
//! `listAppend( value = "x", list = "a" )` returned `x,a` and
//! `dateAdd( number = 1, datepart = "d", date = now() )` threw. Lucee and ACF
//! bind by name (GH #482).
//!
//! # Shape
//! One entry per builtin: its lowercased name, then one alias group per
//! parameter **in declared order**. The first alias is the canonical name; the
//! rest are the spellings Lucee accepts (`getFunctionData( name ).arguments`).
//!
//! # Coverage rule
//! A builtin that is not listed keeps the old positional binding, so adding an
//! entry can only make a call more correct — never less. An entry whose order
//! is wrong, on the other hand, silently misbinds, so only add a signature you
//! have checked against cfdocs/Lucee.
//!
//! # Provenance of the alias sets
//! The first pass wrote one canonical cfdocs name per parameter, which left 138
//! spellings Lucee accepts matching nothing — its documented aliases, and in a
//! few cases the name Lucee itself declares (`haystack`/`needle` for
//! `arrayContains`, `substring` for `replace`'s second parameter,
//! `queryFormat` for `serializeJSON`'s). An unmatched name falls back to
//! binding in CALL ORDER, so those calls misbound silently when written out of
//! order. The sets were then completed by diffing every entry against Lucee
//! 7.1.0.204's own `getFunctionData( name ).arguments`.
//!
//! Two invariants that diff checks, and that a new entry must keep:
//!   - a spelling appears in at most ONE parameter group per builtin
//!     (`param_slot` returns the first group that matches, so a repeat would
//!     silently bind to the earlier slot);
//!   - the group at index `i` is the same parameter Lucee has at index `i`.
//!     `createObject` is the one builtin where that does NOT hold — Lucee's
//!     `name` sits at a different slot — so it keeps its hand-written set.

/// `(builtin, [[alias, …] per parameter])`. Kept sorted by name; looked up with
/// a binary search.
static SIGNATURES: &[(&str, &[&[&str]])] = &[
    ("abs", &[&["number", "value"]]),
    ("arrayappend", &[&["array"], &["value", "object", "obj"], &["merge"]]),
    ("arrayavg", &[&["array"]]),
    (
        "arraycontains",
        &[
            &["array", "haystack", "arr"],
            &["value", "needle", "object", "obj", "o"],
            &["substringmatch"],
        ],
    ),
    (
        "arraycontainsnocase",
        &[
            &["array", "haystack", "arr"],
            &["value", "needle", "object", "obj", "o"],
            &["substringmatch"],
        ],
    ),
    ("arraydelete", &[&["array"], &["value"], &["scope"]]),
    ("arraydeleteat", &[&["array"], &["position", "index"]]),
    ("arrayeach", &[&["array"], &["closure", "callback", "udf", "function"]]),
    ("arrayfilter", &[&["array"], &["closure", "callback", "udf", "function", "filter"]]),
    (
        "arrayfind",
        &[
            &["array"],
            &["value", "value_or_closure", "object", "closure", "function", "udf", "callback"],
        ],
    ),
    ("arrayfindnocase", &[&["array"], &["value", "object"]]),
    ("arrayinsertat", &[&["array"], &["position", "index"], &["value"]]),
    ("arrayisempty", &[&["array"]]),
    ("arraylen", &[&["array"]]),
    ("arraymap", &[&["array"], &["closure", "callback", "udf", "function"]]),
    ("arraymax", &[&["array"]]),
    ("arraymin", &[&["array"]]),
    ("arraynew", &[&["dimension", "dimensions"]]),
    ("arrayprepend", &[&["array"], &["value"]]),
    (
        "arrayreduce",
        &[
            &["array", "object"],
            &["closure", "callback", "udf", "function"],
            &["initialvalue", "initial", "initalvalue"],
        ],
    ),
    ("arrayresize", &[&["array"], &["size", "minimum_size", "minimum"]]),
    (
        "arrayset",
        &[
            &["array"],
            &["start", "start_pos", "startpos"],
            &["end", "end_pos", "endpos"],
            &["value"],
        ],
    ),
    ("arrayslice", &[&["array"], &["offset", "start"], &["length", "count"]]),
    (
        "arraysort",
        &[
            &["array"],
            &["sorttype", "closure", "callback", "sorttype_or_closure", "sort_type", "function", "udf"],
            &["sortorder", "sort_order"],
            &["localesensitive", "locale_sensitive"],
        ],
    ),
    ("arraysum", &[&["array"]]),
    ("arrayswap", &[&["array"], &["position1", "index1"], &["position2", "index2"]]),
    ("arraytolist", &[&["array"], &["delimiter", "delimiters"]]),
    ("asc", &[&["string", "character"]]),
    (
        "binarydecode",
        &[
            &["string", "value", "encoded_binary", "encodedbinary"],
            &["encoding", "binaryencoding"],
        ],
    ),
    ("binaryencode", &[&["binarydata", "binary", "data"], &["encoding", "binaryencoding"]]),
    ("booleanformat", &[&["value", "boolean"]]),
    ("ceiling", &[&["number", "value"]]),
    (
        "charsetdecode",
        &[
            &["string", "value", "encoded_binary", "encodedbinary"],
            &["encoding", "charset"],
        ],
    ),
    ("charsetencode", &[&["binarydata", "binary", "data"], &["encoding", "charset"]]),
    ("chr", &[&["number", "value"]]),
    ("cjustify", &[&["string"], &["length"]]),
    ("compare", &[&["string1"], &["string2"]]),
    ("comparenocase", &[&["string1"], &["string2"]]),
    ("createdate", &[&["year"], &["month", "months"], &["day", "days"]]),
    (
        "createdatetime",
        &[
            &["year", "years"],
            &["month", "months"],
            &["day", "days"],
            &["hour", "hours"],
            &["minute", "minutes"],
            &["second", "seconds"],
        ],
    ),
    ("createobject", &[&["type"], &["classname", "component", "name"]]),
    ("createodbcdate", &[&["date"]]),
    ("createodbcdatetime", &[&["date"]]),
    ("createodbctime", &[&["date"]]),
    ("createtime", &[&["hour", "hours"], &["minute", "minutes"], &["second", "seconds"]]),
    ("dateadd", &[&["datepart"], &["number", "count"], &["date"]]),
    ("datecompare", &[&["date1"], &["date2"], &["datepart"]]),
    ("datediff", &[&["datepart"], &["date1"], &["date2"]]),
    ("dateformat", &[&["date"], &["mask", "format"]]),
    ("datepart", &[&["datepart", "part"], &["date"]]),
    ("datetimeformat", &[&["date", "datetime"], &["mask", "format"], &["timezone"]]),
    ("day", &[&["date"]]),
    ("dayofweek", &[&["date"]]),
    ("dayofyear", &[&["date"]]),
    ("daysinmonth", &[&["date"]]),
    ("daysinyear", &[&["date"]]),
    ("decimalformat", &[&["number", "value"]]),
    ("decrementvalue", &[&["number", "value"]]),
    ("deserializejson", &[&["json", "var", "value", "jsonvar", "data"], &["strictmapping"]]),
    ("directorycreate", &[&["path", "directory"], &["createpath", "doparent"], &["ignoreexists"]]),
    ("directorydelete", &[&["path", "directory"], &["recurse", "recursive"]]),
    ("directoryexists", &[&["path", "directory", "absolute_path", "absolutepath"]]),
    ("dollarformat", &[&["number", "value"]]),
    ("duplicate", &[&["object", "var", "value", "variable_name", "obj"], &["deep", "deepcopy"]]),
    ("expandpath", &[&["path", "relativepath", "relative_path"]]),
    (
        "fileappend",
        &[
            &["filepath", "file", "path"],
            &["data", "content"],
            &["charset", "encoding"],
        ],
    ),
    ("filecopy", &[&["source"], &["destination"]]),
    ("filedelete", &[&["filepath", "file", "path", "source"]]),
    ("fileexists", &[&["filepath", "file", "path", "source"]]),
    (
        "filegetmimetype",
        &[
            &["filepath", "file", "path", "source", "fileobject"],
            &["strict", "checkheader", "checknotextension"],
        ],
    ),
    ("fileinfo", &[&["filepath", "file", "path", "resource"]]),
    ("filemove", &[&["source"], &["destination"]]),
    (
        "fileopen",
        &[
            &["filepath", "file", "path", "source"],
            &["mode"],
            &["charset", "encoding"],
            &["seekable"],
        ],
    ),
    ("fileread", &[&["filepath", "file", "path"], &["charset", "encoding", "charsetorbuffersize"]]),
    ("filereadbinary", &[&["filepath", "file", "path", "source"]]),
    (
        "filewrite",
        &[
            &["filepath", "file", "path", "source"],
            &["data", "content"],
            &["charset", "encoding"],
        ],
    ),
    ("find", &[&["substring"], &["string"], &["start"]]),
    ("findnocase", &[&["substring"], &["string"], &["start"]]),
    ("findoneof", &[&["set"], &["string"], &["start"]]),
    ("fix", &[&["number", "value"]]),
    ("floor", &[&["number", "value"]]),
    ("formatbasen", &[&["number", "value"], &["radix"]]),
    ("getdirectoryfrompath", &[&["path"]]),
    ("getfilefrompath", &[&["path"]]),
    ("getfileinfo", &[&["filepath", "file", "path", "source"]]),
    ("getmetadata", &[&["value", "object", "var"]]),
    ("hash", &[&["string", "input", "value"], &["algorithm"], &["encoding"], &["numiterations"]]),
    ("hour", &[&["date"]]),
    ("htmlcodeformat", &[&["string", "text"], &["version"]]),
    ("htmleditformat", &[&["string", "text"], &["version"]]),
    ("incrementvalue", &[&["number", "value"]]),
    ("inputbasen", &[&["string", "value"], &["radix"]]),
    ("insert", &[&["substring"], &["string"], &["position"]]),
    ("int", &[&["number", "value"]]),
    ("isarray", &[&["value", "var", "object"], &["number", "dimension"]]),
    ("isboolean", &[&["value", "var", "object"]]),
    ("isdate", &[&["date", "value", "string"]]),
    ("isdefined", &[&["variable_name", "variablename", "variable", "name", "value"]]),
    ("isempty", &[&["value", "var", "object"]]),
    ("isjson", &[&["var", "value", "json"]]),
    ("isnull", &[&["value", "var", "object"]]),
    ("isnumeric", &[&["string", "value", "number"]]),
    ("isnumericdate", &[&["number", "value"]]),
    ("isobject", &[&["value", "var", "object"]]),
    ("isquery", &[&["value", "var", "query"]]),
    ("issimplevalue", &[&["value", "var", "object"]]),
    ("isstruct", &[&["value", "var", "object", "variable"]]),
    ("isvalid", &[&["type"], &["value", "var"], &["min", "pattern", "min_or_pattern"], &["max"]]),
    ("javacast", &[&["type"], &["variable", "value", "object"]]),
    ("lcase", &[&["string", "value"]]),
    ("left", &[&["string"], &["count", "number"]]),
    ("len", &[&["string", "value", "object", "var", "data"]]),
    ("listappend", &[&["list"], &["value"], &["delimiters", "delimiter"], &["includeemptyfields"]]),
    (
        "listchangedelims",
        &[
            &["list"],
            &["new_delimiter", "newdelimiter"],
            &["delimiters", "delimiter"],
        ],
    ),
    ("listcontains", &[&["list"], &["substring", "value"], &["delimiters", "delimiter"]]),
    ("listcontainsnocase", &[&["list"], &["substring", "value"], &["delimiters", "delimiter"]]),
    ("listdeleteat", &[&["list"], &["position", "index"], &["delimiters", "delimiter"]]),
    (
        "listeach",
        &[
            &["list"],
            &["closure", "callback", "udf", "function"],
            &["delimiters", "delimiter"],
        ],
    ),
    (
        "listfilter",
        &[
            &["list"],
            &["closure", "callback", "udf", "function", "filter"],
            &["delimiters", "delimiter"],
        ],
    ),
    ("listfind", &[&["list"], &["value"], &["delimiters", "delimiter"]]),
    ("listfindnocase", &[&["list"], &["value"], &["delimiters", "delimiter"]]),
    ("listfirst", &[&["list"], &["delimiters", "delimiter"], &["includeemptyfields"]]),
    (
        "listgetat",
        &[
            &["list"],
            &["position", "index"],
            &["delimiters", "delimiter"],
            &["includeemptyfields"],
        ],
    ),
    (
        "listinsertat",
        &[
            &["list"],
            &["position", "index"],
            &["value"],
            &["delimiters", "delimiter"],
        ],
    ),
    ("listlast", &[&["list"], &["delimiters", "delimiter"], &["includeemptyfields"]]),
    ("listlen", &[&["list"], &["delimiters", "delimiter"], &["includeemptyfields"]]),
    (
        "listmap",
        &[
            &["list"],
            &["closure", "callback", "udf", "function"],
            &["delimiters", "delimiter"],
        ],
    ),
    ("listprepend", &[&["list"], &["value"], &["delimiters", "delimiter"]]),
    ("listqualify", &[&["list"], &["qualifier"], &["delimiters", "delimiter"], &["elements"]]),
    ("listremoveduplicates", &[&["list"], &["delimiter", "delimiters"], &["ignorecase"]]),
    ("listrest", &[&["list"], &["delimiters", "delimiter"], &["includeemptyfields"]]),
    ("listsetat", &[&["list"], &["position", "index"], &["value"], &["delimiters", "delimiter"]]),
    (
        "listsort",
        &[
            &["list"],
            &["sorttype", "sort_type"],
            &["sortorder", "sort_order"],
            &["delimiters", "delimiter"],
            &["includeemptyfields"],
        ],
    ),
    (
        "listtoarray",
        &[
            &["list"],
            &["delimiters", "delimiter"],
            &["includeemptyfields"],
            &["multicharacterdelimiter"],
        ],
    ),
    ("listvaluecount", &[&["list"], &["value"], &["delimiters", "delimiter"]]),
    ("listvaluecountnocase", &[&["list"], &["value"], &["delimiters", "delimiter"]]),
    ("ljustify", &[&["string"], &["length"]]),
    ("lsdateformat", &[&["date"], &["mask", "format"], &["locale"]]),
    ("lsnumberformat", &[&["number", "value"], &["mask", "format"], &["locale"]]),
    ("ltrim", &[&["string", "value"]]),
    ("max", &[&["number1"], &["number2"]]),
    ("mid", &[&["string"], &["start"], &["count", "length"]]),
    ("min", &[&["number1"], &["number2"]]),
    ("minute", &[&["date"]]),
    ("month", &[&["date"]]),
    ("numberformat", &[&["number", "value"], &["mask", "format"]]),
    ("paragraphformat", &[&["string", "text"]]),
    (
        "parsedatetime",
        &[
            &["date", "datetime", "string"],
            &["format", "mask", "popconversion"],
            &["timezone"],
        ],
    ),
    ("quarter", &[&["date"]]),
    (
        "queryaddcolumn",
        &[
            &["query"],
            &["columnname", "column", "name"],
            &["datatype", "type", "datatype_or_array"],
            &["array", "data"],
        ],
    ),
    ("queryaddrow", &[&["query"], &["number", "rowdata", "row", "numberordata", "data"]]),
    ("querycolumnarray", &[&["query"]]),
    ("querycolumndata", &[&["query"], &["columnname", "column", "name"]]),
    ("queryeach", &[&["query"], &["closure", "callback", "udf", "function"]]),
    ("queryfilter", &[&["query"], &["closure", "callback", "udf", "function", "filter"]]),
    (
        "querygetcell",
        &[
            &["query"],
            &["columnname", "column", "name", "column_name"],
            &["rownumber", "row", "row_number"],
        ],
    ),
    ("querymap", &[&["query"], &["closure", "callback", "udf", "function"]]),
    (
        "querynew",
        &[
            &["columnlist", "columnnames", "columns", "names"],
            &["columntypelist", "columntypes", "types"],
            &["data", "rowdata"],
        ],
    ),
    ("queryrowdata", &[&["query"], &["row", "rownumber"]]),
    (
        "querysetcell",
        &[
            &["query"],
            &["column", "columnname", "name", "column_name"],
            &["value"],
            &["row", "rownumber", "row_number", "number"],
        ],
    ),
    ("rand", &[&["algorithm"]]),
    ("randrange", &[&["number1", "from"], &["number2", "to"], &["algorithm"]]),
    (
        "refind",
        &[
            &["reg_expression", "regex", "regular_expression"],
            &["string", "text"],
            &["start"],
            &["returnsubexpressions"],
            &["scope"],
        ],
    ),
    (
        "refindnocase",
        &[
            &["reg_expression", "regex", "regular_expression"],
            &["string", "text"],
            &["start"],
            &["returnsubexpressions"],
            &["scope"],
        ],
    ),
    ("rematch", &[&["reg_expression", "regex", "regular_expression"], &["string", "text"]]),
    ("rematchnocase", &[&["reg_expression", "regex", "regular_expression"], &["string", "text"]]),
    ("removechars", &[&["string"], &["start"], &["count", "length"]]),
    ("repeatstring", &[&["string"], &["count", "number"]]),
    (
        "replace",
        &[
            &["string"],
            &["substring1", "substring", "search", "sub1", "find"],
            &["substring2", "replacement", "replace", "sub2", "repl"],
            &["scope"],
        ],
    ),
    (
        "replacelist",
        &[
            &["string"],
            &["list1", "list_1"],
            &["list2", "list_2"],
            &["delimiter", "delimiters", "delimiter_list1", "delimiterlist1"],
        ],
    ),
    (
        "replacenocase",
        &[
            &["string"],
            &["substring1", "substring", "search"],
            &["substring2", "replacement", "replace"],
            &["scope"],
        ],
    ),
    (
        "rereplace",
        &[
            &["string", "text"],
            &["reg_expression", "regex", "regular_expression"],
            &["substring", "replacement", "replace"],
            &["scope"],
        ],
    ),
    (
        "rereplacenocase",
        &[
            &["string", "text"],
            &["reg_expression", "regex", "regular_expression"],
            &["substring", "replacement", "replace"],
            &["scope"],
        ],
    ),
    ("reverse", &[&["string", "value"]]),
    ("right", &[&["string"], &["count", "number"]]),
    ("rjustify", &[&["string"], &["length"]]),
    ("round", &[&["number", "value"]]),
    ("rtrim", &[&["string", "value"]]),
    ("second", &[&["date"]]),
    (
        "serializejson",
        &[
            &["var", "data", "value"],
            &["serializequerybycolumns", "queryformat"],
            &["usesecurejsonprefix", "usesecurejsonprefixorcharset", "charset", "charsetname"],
        ],
    ),
    ("sgn", &[&["number", "value"]]),
    ("sleep", &[&["duration", "time", "millis"]]),
    ("spanexcluding", &[&["string"], &["set"]]),
    ("spanincluding", &[&["string"], &["set"]]),
    ("sqr", &[&["number", "value"]]),
    ("stripcr", &[&["string"]]),
    (
        "structappend",
        &[
            &["struct1", "struct"],
            &["struct2", "data"],
            &["overwrite", "overwriteflag"],
        ],
    ),
    ("structclear", &[&["struct", "structure", "object"]]),
    ("structcopy", &[&["struct", "structure", "object"]]),
    ("structcount", &[&["struct", "structure", "object"]]),
    (
        "structdelete",
        &[
            &["struct", "structure", "object"],
            &["key"],
            &["indicatenotexists", "indicatenotexisting", "indicateexists"],
        ],
    ),
    ("structeach", &[&["struct", "structure"], &["closure", "callback", "udf", "function"]]),
    ("structfilter", &[&["struct"], &["closure", "callback", "udf", "function", "filter"]]),
    ("structfind", &[&["struct", "structure", "object"], &["key"], &["defaultvalue", "default"]]),
    ("structfindkey", &[&["top", "struct"], &["value", "key"], &["scope"]]),
    ("structfindvalue", &[&["top", "struct"], &["value", "key"], &["scope"]]),
    ("structget", &[&["pathdesired", "path"]]),
    (
        "structinsert",
        &[
            &["struct", "structure", "object"],
            &["key"],
            &["value"],
            &["overwrite", "allowoverwrite"],
        ],
    ),
    ("structisempty", &[&["struct", "structure", "object"]]),
    ("structkeyarray", &[&["struct", "structure", "object"]]),
    ("structkeyexists", &[&["struct", "structure", "object"], &["key"]]),
    ("structkeylist", &[&["struct", "structure", "object"], &["delimiter", "delimiters"]]),
    ("structmap", &[&["struct", "structure"], &["closure", "callback", "udf", "function"]]),
    ("structnew", &[&["type"]]),
    (
        "structsort",
        &[
            &["base", "struct"],
            &["sorttype", "sorttypeorsortfunc"],
            &["sortorder"],
            &["pathtosubelement", "path"],
        ],
    ),
    ("structupdate", &[&["struct", "structure", "object"], &["key"], &["value"]]),
    ("tobase64", &[&["string", "value", "binarydata", "strorbin"], &["encoding", "charset"]]),
    ("tobinary", &[&["string", "value", "base64", "data"]]),
    ("tostring", &[&["value", "var", "object"], &["encoding", "charset"]]),
    ("trim", &[&["string", "value"]]),
    ("ucase", &[&["string", "value"]]),
    ("ucfirst", &[&["string"], &["doall", "all"], &["dolowerifallupper", "dolowerifalluppercase"]]),
    ("urldecode", &[&["string", "urlencodedstring"], &["charset", "encoding"]]),
    ("urlencodedformat", &[&["string"], &["charset", "encoding"]]),
    ("val", &[&["string", "value", "number"]]),
    ("valuearray", &[&["query", "query_column"], &["column", "columnname", "name"]]),
    (
        "valuelist",
        &[
            &["query_column", "column", "query", "querycolumn"],
            &["delimiter", "delimiters"],
        ],
    ),
    ("week", &[&["date"]]),
    ("wrap", &[&["string"], &["limit"], &["strip"]]),
    ("writeoutput", &[&["string", "value", "var", "data"]]),
    ("xmlformat", &[&["string", "text"]]),
    ("year", &[&["date"]]),
    ("yesnoformat", &[&["value", "boolean"]]),
];

/// The positional slot a named argument binds to, or `None` when this builtin
/// has no declared signature or does not know the name.
pub fn param_slot(builtin_lc: &str, arg_lc: &str) -> Option<usize> {
    let params = signature(builtin_lc)?;
    params
        .iter()
        .position(|aliases| aliases.iter().any(|a| *a == arg_lc))
}

/// Whether a signature is declared for this builtin.
#[inline]
pub fn has_signature(builtin_lc: &str) -> bool {
    signature(builtin_lc).is_some()
}

#[inline]
fn signature(builtin_lc: &str) -> Option<&'static [&'static [&'static str]]> {
    SIGNATURES
        .binary_search_by(|(n, _)| (*n).cmp(builtin_lc))
        .ok()
        .map(|i| SIGNATURES[i].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is binary-searched, so it must stay sorted and duplicate-free.
    #[test]
    fn table_is_sorted_and_unique() {
        for w in SIGNATURES.windows(2) {
            assert!(w[0].0 < w[1].0, "out of order: {} then {}", w[0].0, w[1].0);
        }
    }

    /// Every alias must be lower-case (lookups pass a lower-cased name) and no
    /// alias may appear twice in one signature, which would make the binding
    /// depend on table order.
    #[test]
    fn aliases_are_lowercase_and_unique_per_function() {
        for (name, params) in SIGNATURES {
            let mut seen: Vec<&str> = Vec::new();
            for aliases in *params {
                for a in *aliases {
                    assert_eq!(*a, a.to_ascii_lowercase(), "{name}: alias {a} is not lower-case");
                    assert!(!seen.contains(a), "{name}: alias {a} is declared twice");
                    seen.push(a);
                }
            }
        }
    }

    #[test]
    fn binds_by_name_not_position() {
        assert_eq!(param_slot("listappend", "list"), Some(0));
        assert_eq!(param_slot("listappend", "value"), Some(1));
        assert_eq!(param_slot("left", "count"), Some(1));
        assert_eq!(param_slot("dateadd", "datepart"), Some(0));
        assert_eq!(param_slot("dateadd", "date"), Some(2));
        assert_eq!(param_slot("replace", "substring2"), Some(2));
        assert_eq!(param_slot("listappend", "nosuchthing"), None);
        assert!(!has_signature("somethingelse"));
    }
}
