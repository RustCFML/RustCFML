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

/// `(builtin, [[alias, …] per parameter])`. Kept sorted by name; looked up with
/// a binary search.
static SIGNATURES: &[(&str, &[&[&str]])] = &[
    ("abs", &[&["number", "value"]]),
    ("arrayappend", &[&["array"], &["value"], &["merge"]]),
    ("arrayavg", &[&["array"]]),
    ("arraycontains", &[&["array"], &["value"], &["substringmatch"]]),
    ("arraycontainsnocase", &[&["array"], &["value"], &["substringmatch"]]),
    ("arraydelete", &[&["array"], &["value"], &["scope"]]),
    ("arraydeleteat", &[&["array"], &["position", "index"]]),
    ("arrayeach", &[&["array"], &["closure", "callback", "udf", "function"]]),
    ("arrayfilter", &[&["array"], &["closure", "callback", "udf", "function"]]),
    ("arrayfind", &[&["array"], &["value"]]),
    ("arrayfindnocase", &[&["array"], &["value"]]),
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
        &[&["array"], &["closure", "callback", "udf", "function"], &["initialvalue", "initial"]],
    ),
    ("arrayresize", &[&["array"], &["size"]]),
    ("arrayset", &[&["array"], &["start"], &["end"], &["value"]]),
    ("arrayslice", &[&["array"], &["offset", "start"], &["length", "count"]]),
    (
        "arraysort",
        &[&["array"], &["sorttype", "closure", "callback"], &["sortorder"], &["localesensitive"]],
    ),
    ("arraysum", &[&["array"]]),
    ("arrayswap", &[&["array"], &["position1", "index1"], &["position2", "index2"]]),
    ("arraytolist", &[&["array"], &["delimiter", "delimiters"]]),
    ("asc", &[&["string", "character"]]),
    ("binarydecode", &[&["string", "value"], &["encoding", "binaryencoding"]]),
    ("binaryencode", &[&["binarydata", "binary", "data"], &["encoding", "binaryencoding"]]),
    ("booleanformat", &[&["value", "boolean"]]),
    ("ceiling", &[&["number", "value"]]),
    ("charsetdecode", &[&["string", "value"], &["encoding", "charset"]]),
    ("charsetencode", &[&["binarydata", "binary", "data"], &["encoding", "charset"]]),
    ("chr", &[&["number", "value"]]),
    ("cjustify", &[&["string"], &["length"]]),
    ("compare", &[&["string1"], &["string2"]]),
    ("comparenocase", &[&["string1"], &["string2"]]),
    ("createdate", &[&["year"], &["month"], &["day"]]),
    (
        "createdatetime",
        &[&["year"], &["month"], &["day"], &["hour"], &["minute"], &["second"]],
    ),
    ("createobject", &[&["type"], &["classname", "component", "name"]]),
    ("createodbcdate", &[&["date"]]),
    ("createodbcdatetime", &[&["date"]]),
    ("createodbctime", &[&["date"]]),
    ("createtime", &[&["hour"], &["minute"], &["second"]]),
    ("dateadd", &[&["datepart"], &["number", "count"], &["date"]]),
    ("datecompare", &[&["date1"], &["date2"], &["datepart"]]),
    ("datediff", &[&["datepart"], &["date1"], &["date2"]]),
    ("dateformat", &[&["date"], &["mask", "format"]]),
    ("datepart", &[&["datepart"], &["date"]]),
    ("datetimeformat", &[&["date"], &["mask", "format"], &["timezone"]]),
    ("day", &[&["date"]]),
    ("dayofweek", &[&["date"]]),
    ("dayofyear", &[&["date"]]),
    ("daysinmonth", &[&["date"]]),
    ("daysinyear", &[&["date"]]),
    ("decimalformat", &[&["number", "value"]]),
    ("decrementvalue", &[&["number", "value"]]),
    ("deserializejson", &[&["json", "var", "value"], &["strictmapping"]]),
    ("directorycreate", &[&["path", "directory"], &["createpath"], &["ignoreexists"]]),
    ("directorydelete", &[&["path", "directory"], &["recurse"]]),
    ("directoryexists", &[&["path", "directory"]]),
    ("dollarformat", &[&["number", "value"]]),
    ("duplicate", &[&["object", "var", "value"], &["deep"]]),
    ("expandpath", &[&["path", "relativepath"]]),
    ("fileappend", &[&["filepath", "file", "path"], &["data", "content"], &["charset", "encoding"]]),
    ("filecopy", &[&["source"], &["destination"]]),
    ("filedelete", &[&["filepath", "file", "path"]]),
    ("fileexists", &[&["filepath", "file", "path"]]),
    ("filegetmimetype", &[&["filepath", "file", "path"], &["strict"]]),
    ("fileinfo", &[&["filepath", "file", "path"]]),
    ("filemove", &[&["source"], &["destination"]]),
    (
        "fileopen",
        &[&["filepath", "file", "path"], &["mode"], &["charset", "encoding"], &["seekable"]],
    ),
    ("fileread", &[&["filepath", "file", "path"], &["charset", "encoding"]]),
    ("filereadbinary", &[&["filepath", "file", "path"]]),
    ("filewrite", &[&["filepath", "file", "path"], &["data", "content"], &["charset", "encoding"]]),
    ("find", &[&["substring"], &["string"], &["start"]]),
    ("findnocase", &[&["substring"], &["string"], &["start"]]),
    ("findoneof", &[&["set"], &["string"], &["start"]]),
    ("fix", &[&["number", "value"]]),
    ("floor", &[&["number", "value"]]),
    ("formatbasen", &[&["number", "value"], &["radix"]]),
    ("getdirectoryfrompath", &[&["path"]]),
    ("getfilefrompath", &[&["path"]]),
    ("getfileinfo", &[&["filepath", "file", "path"]]),
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
    ("isdefined", &[&["variable_name", "variablename", "variable", "name"]]),
    ("isempty", &[&["value", "var", "object"]]),
    ("isjson", &[&["var", "value", "json"]]),
    ("isnull", &[&["value", "var", "object"]]),
    ("isnumeric", &[&["string", "value", "number"]]),
    ("isnumericdate", &[&["number", "value"]]),
    ("isobject", &[&["value", "var", "object"]]),
    ("isquery", &[&["value", "var", "query"]]),
    ("issimplevalue", &[&["value", "var", "object"]]),
    ("isstruct", &[&["value", "var", "object"]]),
    ("isvalid", &[&["type"], &["value", "var"], &["min", "pattern"], &["max"]]),
    ("javacast", &[&["type"], &["variable", "value", "object"]]),
    ("lcase", &[&["string", "value"]]),
    ("left", &[&["string"], &["count", "number"]]),
    ("len", &[&["string", "value", "object", "var", "data"]]),
    (
        "listappend",
        &[&["list"], &["value"], &["delimiters", "delimiter"], &["includeemptyfields"]],
    ),
    ("listchangedelims", &[&["list"], &["new_delimiter", "newdelimiter"], &["delimiters", "delimiter"]]),
    ("listcontains", &[&["list"], &["substring", "value"], &["delimiters", "delimiter"]]),
    ("listcontainsnocase", &[&["list"], &["substring", "value"], &["delimiters", "delimiter"]]),
    ("listdeleteat", &[&["list"], &["position", "index"], &["delimiters", "delimiter"]]),
    ("listeach", &[&["list"], &["closure", "callback", "udf", "function"], &["delimiters", "delimiter"]]),
    ("listfilter", &[&["list"], &["closure", "callback", "udf", "function"], &["delimiters", "delimiter"]]),
    ("listfind", &[&["list"], &["value"], &["delimiters", "delimiter"]]),
    ("listfindnocase", &[&["list"], &["value"], &["delimiters", "delimiter"]]),
    ("listfirst", &[&["list"], &["delimiters", "delimiter"], &["includeemptyfields"]]),
    (
        "listgetat",
        &[&["list"], &["position", "index"], &["delimiters", "delimiter"], &["includeemptyfields"]],
    ),
    ("listinsertat", &[&["list"], &["position", "index"], &["value"], &["delimiters", "delimiter"]]),
    ("listlast", &[&["list"], &["delimiters", "delimiter"], &["includeemptyfields"]]),
    ("listlen", &[&["list"], &["delimiters", "delimiter"], &["includeemptyfields"]]),
    ("listmap", &[&["list"], &["closure", "callback", "udf", "function"], &["delimiters", "delimiter"]]),
    ("listprepend", &[&["list"], &["value"], &["delimiters", "delimiter"]]),
    ("listqualify", &[&["list"], &["qualifier"], &["delimiters", "delimiter"], &["elements"]]),
    ("listremoveduplicates", &[&["list"], &["delimiter", "delimiters"], &["ignorecase"]]),
    ("listrest", &[&["list"], &["delimiters", "delimiter"], &["includeemptyfields"]]),
    ("listsetat", &[&["list"], &["position", "index"], &["value"], &["delimiters", "delimiter"]]),
    (
        "listsort",
        &[&["list"], &["sorttype"], &["sortorder"], &["delimiters", "delimiter"], &["includeemptyfields"]],
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
    ("parsedatetime", &[&["date", "datetime", "string"], &["format", "mask"], &["timezone"]]),
    ("quarter", &[&["date"]]),
    ("queryaddcolumn", &[&["query"], &["columnname", "column", "name"], &["datatype", "type"], &["array", "data"]]),
    ("queryaddrow", &[&["query"], &["number", "rowdata", "row"]]),
    ("querycolumnarray", &[&["query"]]),
    ("querycolumndata", &[&["query"], &["columnname", "column", "name"]]),
    ("queryeach", &[&["query"], &["closure", "callback", "udf", "function"]]),
    ("queryfilter", &[&["query"], &["closure", "callback", "udf", "function"]]),
    ("querygetcell", &[&["query"], &["columnname", "column", "name"], &["rownumber", "row"]]),
    ("querymap", &[&["query"], &["closure", "callback", "udf", "function"]]),
    (
        "querynew",
        &[&["columnlist", "columnnames", "columns"], &["columntypelist", "columntypes", "types"], &["data", "rowdata"]],
    ),
    ("queryrowdata", &[&["query"], &["row", "rownumber"]]),
    (
        "querysetcell",
        &[&["query"], &["column", "columnname", "name"], &["value"], &["row", "rownumber"]],
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
    ("replace", &[&["string"], &["substring1", "substring", "search"], &["substring2", "replacement", "replace"], &["scope"]]),
    (
        "replacelist",
        &[&["string"], &["list1", "list_1"], &["list2", "list_2"], &["delimiter", "delimiters"]],
    ),
    (
        "replacenocase",
        &[&["string"], &["substring1", "substring", "search"], &["substring2", "replacement", "replace"], &["scope"]],
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
    ("serializejson", &[&["var", "data", "value"], &["serializequerybycolumns", "queryformat"], &["usesecurejsonprefix"]]),
    ("sgn", &[&["number", "value"]]),
    ("sleep", &[&["duration", "time", "millis"]]),
    ("spanexcluding", &[&["string"], &["set"]]),
    ("spanincluding", &[&["string"], &["set"]]),
    ("sqr", &[&["number", "value"]]),
    ("stripcr", &[&["string"]]),
    ("structappend", &[&["struct1", "struct"], &["struct2", "data"], &["overwrite"]]),
    ("structclear", &[&["struct"]]),
    ("structcopy", &[&["struct"]]),
    ("structcount", &[&["struct"]]),
    ("structdelete", &[&["struct"], &["key"], &["indicatenotexists"]]),
    ("structeach", &[&["struct"], &["closure", "callback", "udf", "function"]]),
    ("structfilter", &[&["struct"], &["closure", "callback", "udf", "function"]]),
    ("structfind", &[&["struct"], &["key"], &["defaultvalue", "default"]]),
    ("structfindkey", &[&["top", "struct"], &["value"], &["scope"]]),
    ("structfindvalue", &[&["top", "struct"], &["value"], &["scope"]]),
    ("structget", &[&["pathdesired", "path"]]),
    ("structinsert", &[&["struct"], &["key"], &["value"], &["overwrite", "allowoverwrite"]]),
    ("structisempty", &[&["struct"]]),
    ("structkeyarray", &[&["struct"]]),
    ("structkeyexists", &[&["struct"], &["key"]]),
    ("structkeylist", &[&["struct"], &["delimiter", "delimiters"]]),
    ("structmap", &[&["struct"], &["closure", "callback", "udf", "function"]]),
    ("structnew", &[&["type"]]),
    ("structsort", &[&["base", "struct"], &["sorttype"], &["sortorder"], &["pathtosubelement"]]),
    ("structupdate", &[&["struct"], &["key"], &["value"]]),
    ("tobase64", &[&["string", "value", "binarydata"], &["encoding", "charset"]]),
    ("tobinary", &[&["string", "value", "base64"]]),
    ("tostring", &[&["value", "var", "object"], &["encoding", "charset"]]),
    ("trim", &[&["string", "value"]]),
    ("ucase", &[&["string", "value"]]),
    ("ucfirst", &[&["string"], &["doall", "all"], &["dolowerifallupper"]]),
    ("urldecode", &[&["string", "urlencodedstring"], &["charset", "encoding"]]),
    ("urlencodedformat", &[&["string"], &["charset", "encoding"]]),
    ("val", &[&["string", "value", "number"]]),
    ("valuearray", &[&["query"], &["column", "columnname", "name"]]),
    ("valuelist", &[&["query_column", "column", "query"], &["delimiter", "delimiters"]]),
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
