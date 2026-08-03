//! CFML Built-in Functions - Standard Library
//!
//! Implements the core CFML built-in function library including:
//! - String functions
//! - Array functions
//! - Struct functions
//! - Math functions
//! - Date/Time functions
//! - Type checking functions
//! - Conversion functions
//! - List functions
//! - JSON functions
//! - Output functions
//! - Query functions
//! - System functions

use cfml_common::dynamic::{build_implements_meta, CfmlAccess, CfmlClosureBody, CfmlFunction, CfmlQuery, CfmlQueryData, CfmlStruct, CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};
use std::collections::HashMap;
use regex::Regex;
use once_cell::sync::Lazy;
use serde_json;
use chrono::{NaiveDateTime, NaiveDate, NaiveTime, Datelike, Timelike, Local, Utc, TimeZone};

// Pre-compiled regex patterns used by isValid() and other builtins.
// Hoisted to module-level Lazy statics to avoid recompiling on every call.
static EMAIL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}$")
        .expect("EMAIL_REGEX pattern is valid")
});
static UUID_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{16}$")
        .expect("UUID_REGEX pattern is valid")
});
static GUID_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$")
        .expect("GUID_REGEX pattern is valid")
});
static ZIPCODE_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\d{5}(-\d{4})?$").expect("ZIPCODE_REGEX pattern is valid")
});
static SSN_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\d{3}-\d{2}-\d{4}$").expect("SSN_REGEX pattern is valid")
});

// Bounded cache of compiled regexes keyed by the full pattern string (the
// `(?i)` case-insensitivity prefix is folded into the key by callers, so the
// key uniquely identifies the compiled program). reFind/reReplace/reMatch/
// isValid previously recompiled the same handful of framework patterns on
// every call — profiling stock Wheels (`/posts`) showed regex NFA/DFA
// construction (`regex_automata` compiler/onepass) at ~3.5% of request CPU,
// because Wheels routing / pluralization / view helpers lean on reReplace &
// reFind with fixed patterns.
//
// Entries are handed out as `Arc<CfRegex>` and never deep-cloned. That detail is
// load-bearing, and the comment here used to claim the opposite: `regex::Regex`'s
// `Clone` shares the compiled program via `Arc` but deliberately builds a
// BRAND-NEW, EMPTY scratch `Pool`. A cloned regex therefore starts with a cold
// lazy-DFA cache and re-determinizes states on its first match, which made every
// cache *hit* nearly as expensive as a miss. On a live Preside request that was
// ~1.6 GiB of allocation churn (`SparseSet::resize`, `Lazy::add_state`,
// `Pool::new`). Sharing one `Arc` keeps the warm pool alive across calls.
static REGEX_CACHE: Lazy<std::sync::RwLock<HashMap<String, std::sync::Arc<CfRegex>>>> =
    Lazy::new(|| std::sync::RwLock::new(HashMap::new()));
const REGEX_CACHE_CAP: usize = 4096;

/// A compiled CFML regex, backed by the fast `regex` crate where possible and
/// falling back to `fancy-regex` for patterns the `regex` crate rejects —
/// chiefly lookaround (`(?=…)`, `(?!…)`, `(?<=…)`, `(?<!…)`) and backreferences
/// (`\1`), both of which appear in real framework code (e.g. Preside's
/// `_escapeAlias` uses `\bas\b\s+(\w+)(?!\s*[`"\[])$`). Java/Lucee/ACF regex
/// supports these; the `regex` crate does not. The fast variant stays the hot
/// path (the cache holds whichever variant compiled); only patterns that fail
/// to compile on `regex` pay the backtracking cost.
#[derive(Clone)]
enum CfRegex {
    Std(Regex),
    Fancy(fancy_regex::Regex),
}

/// One captured group resolved to (byte-start, matched text). `None` = the group
/// did not participate in the match. Index 0 is the whole match.
type CapList = Vec<Option<(usize, String)>>;

impl CfRegex {
    fn is_match(&self, text: &str) -> bool {
        match self {
            CfRegex::Std(r) => r.is_match(text),
            CfRegex::Fancy(r) => r.is_match(text).unwrap_or(false),
        }
    }

    /// Overall-match byte start, searching from byte offset `start`. `^`/`\A`
    /// remain anchored to the true start of the text (Lucee/Java/PCRE semantics).
    fn find_at_start(&self, text: &str, start: usize) -> Option<usize> {
        match self {
            CfRegex::Std(r) => r.find_at(text, start).map(|m| m.start()),
            CfRegex::Fancy(r) => r.find_from_pos(text, start).ok().flatten().map(|m| m.start()),
        }
    }

    /// All capture groups for the first match at/after byte offset `start`.
    fn captures_at_start(&self, text: &str, start: usize) -> Option<CapList> {
        match self {
            CfRegex::Std(r) => r.captures_at(text, start).map(|caps| {
                (0..caps.len())
                    .map(|i| caps.get(i).map(|m| (m.start(), m.as_str().to_string())))
                    .collect()
            }),
            CfRegex::Fancy(r) => r.captures_from_pos(text, start).ok().flatten().map(|caps| {
                (0..caps.len())
                    .map(|i| caps.get(i).map(|m| (m.start(), m.as_str().to_string())))
                    .collect()
            }),
        }
    }

    /// All non-overlapping whole matches as owned strings (for `reMatch`).
    fn find_all(&self, text: &str) -> Vec<String> {
        match self {
            CfRegex::Std(r) => r.find_iter(text).map(|m| m.as_str().to_string()).collect(),
            CfRegex::Fancy(r) => r
                .find_iter(text)
                .filter_map(|m| m.ok().map(|m| m.as_str().to_string()))
                .collect(),
        }
    }

    /// Replace using a CFML replacement template (honors `\N` backrefs and case
    /// modifiers). `all=false` replaces only the first match.
    fn replace_cfml(&self, text: &str, replacement: &str, all: bool) -> String {
        match self {
            CfRegex::Std(r) => {
                let rep = |caps: &regex::Captures| {
                    expand_cfml_replacement(replacement, |g| caps.get(g).map(|m| m.as_str().to_string()))
                };
                if all {
                    r.replace_all(text, rep).to_string()
                } else {
                    r.replace(text, rep).to_string()
                }
            }
            CfRegex::Fancy(r) => {
                let rep = |caps: &fancy_regex::Captures| {
                    expand_cfml_replacement(replacement, |g| caps.get(g).map(|m| m.as_str().to_string()))
                };
                if all {
                    r.replace_all(text, rep).to_string()
                } else {
                    r.replace(text, rep).to_string()
                }
            }
        }
    }
}

/// Compile `pat`, returning a clone of the cached `Regex` on hit. On a compile
/// error returns `Err` (callers fall back to their no-match behavior). The
/// cache is bounded: if inserting would exceed `REGEX_CACHE_CAP` distinct
/// patterns (e.g. an adversarial workload generating unique patterns) the cache
/// is cleared first, trading a rare cold rebuild for a hard memory ceiling.
/// Translate CFML/Java regex syntax that the Rust `regex` crate rejects into an
/// equivalent it accepts, before compilation. Currently handles one construct
/// that appears in real framework code (Wheels `autoLink`'s `[^\s\b]+`): a `\b`
/// INSIDE a character class. Java/Lucee/ACF treat `\b` within `[...]` as the
/// backspace char (U+0008); the Rust crate only accepts `\b` as a word boundary
/// (outside a class) and errors on it inside one — and `cached_regex`'s callers
/// silently swallow the compile error as "no match", so the whole pattern
/// becomes a no-op. Rewrite the in-class `\b` to `\x08`; word-boundary `\b`
/// (outside a class) is left untouched.
///
/// Also handles a second real-framework construct (Preside's resource-URI
/// validator `[\w-\.]`): a literal `-` placed immediately next to a class
/// shorthand (`\w \W \d \D \s \S`) inside a character class. Java/Lucee/PCRE
/// treat that `-` as a literal hyphen, but the `regex` crate (and `fancy-regex`)
/// parse `\w-\.` as a character range with non-literal endpoints and reject it
/// with "invalid range boundary". Escape the hyphen to `\-`, which keeps Lucee
/// semantics and compiles. A `-` forming a genuine range (`a-z`, `0-9`) or one
/// that is leading/trailing in the class is left untouched.
fn translate_cfml_regex(pat: &str) -> std::borrow::Cow<'_, str> {
    // Only escapes can trigger either rewrite, so a pattern with no `\` is safe.
    if !pat.contains('\\') {
        return std::borrow::Cow::Borrowed(pat);
    }
    let is_shorthand = |c: char| matches!(c, 'w' | 'W' | 'd' | 'D' | 's' | 'S');
    let chars: Vec<char> = pat.chars().collect();
    let mut out = String::with_capacity(pat.len() + 4);
    let mut in_class = false;
    let mut class_pos = 0usize; // members seen in the current class (for leading-] / ^)
    let mut prev_was_shorthand = false; // last in-class member emitted was \w \d \s …
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            let n = chars[i + 1];
            if in_class && n == 'b' {
                out.push_str("\\x08");
                prev_was_shorthand = false;
            } else if !in_class && (n == '<' || n == '>') {
                // Java/Lucee/ACF treat `\<` and `\>` as escaped LITERAL angle
                // brackets, NOT word boundaries (Java's `Pattern` has no `\<`/`\>`
                // — word boundaries are `\b`). The Rust `regex` crate DID add
                // `\<`/`\>` as start/end-of-word boundaries, which silently changed
                // the match set: Wheels formsdateplainSpec's
                // `ReMatchNoCase("\<option", html)` matched every "option" WORD (4
                // opening + 4 closing tags = 8) instead of the 4 literal `<option`
                // opens, so `minuteStep=15` reported 8 options where 4 were
                // expected. Emit the bare bracket (`<`/`>` are literals in regex)
                // to keep Lucee semantics. Inside a character class `\<`/`\>` are
                // already literal, so this only touches the outside-class case.
                out.push(n);
                prev_was_shorthand = false;
            } else {
                out.push(c);
                out.push(n);
                prev_was_shorthand = in_class && is_shorthand(n);
            }
            if in_class {
                class_pos += 1;
            }
            i += 2;
            continue;
        }
        match c {
            '[' if !in_class => {
                in_class = true;
                class_pos = 0;
                prev_was_shorthand = false;
                out.push('[');
            }
            ']' if in_class => {
                // A `]` as the first class member (or right after `^`) is literal;
                // otherwise it closes the class.
                if class_pos == 0 {
                    out.push(']');
                    class_pos += 1;
                } else {
                    in_class = false;
                    out.push(']');
                }
                prev_was_shorthand = false;
            }
            '^' if in_class && class_pos == 0 => out.push('^'),
            '-' if in_class && class_pos > 0 => {
                // Escape only when adjacent to a class shorthand on either side
                // (an invalid range endpoint); a normal range like a-z is left
                // alone so genuine ranges keep working.
                let next_is_shorthand = chars.get(i + 1) == Some(&'\\')
                    && chars.get(i + 2).map(|&x| is_shorthand(x)).unwrap_or(false);
                if prev_was_shorthand || next_is_shorthand {
                    out.push('\\');
                }
                out.push('-');
                class_pos += 1;
                prev_was_shorthand = false;
            }
            _ => {
                if in_class {
                    class_pos += 1;
                }
                prev_was_shorthand = false;
                out.push(c);
            }
        }
        i += 1;
    }
    std::borrow::Cow::Owned(out)
}

fn cached_regex(pat: &str) -> Result<std::sync::Arc<CfRegex>, ()> {
    if let Some(re) = REGEX_CACHE.read().unwrap().get(pat) {
        // Refcount bump only — see REGEX_CACHE's note on why this must not be a
        // deep clone.
        return Ok(std::sync::Arc::clone(re));
    }
    let translated = translate_cfml_regex(pat);
    // Fast path: the `regex` crate. On a compile error, retry with `fancy-regex`,
    // which supports lookaround and backreferences (a genuinely-malformed pattern
    // fails both and the caller falls back to its no-match behavior).
    let re = match Regex::new(&translated) {
        Ok(r) => CfRegex::Std(r),
        Err(_) => match fancy_regex::Regex::new(&translated) {
            Ok(r) => CfRegex::Fancy(r),
            Err(_) => return Err(()),
        },
    };
    let re = std::sync::Arc::new(re);
    let mut cache = REGEX_CACHE.write().unwrap();
    if cache.len() >= REGEX_CACHE_CAP {
        cache.clear();
    }
    // Return the entry actually stored, not our local one: a racing thread may
    // have inserted first, and both callers should end up sharing that single
    // warm pool rather than each holding a private cold one.
    Ok(std::sync::Arc::clone(
        cache
            .entry(pat.to_string())
            .or_insert_with(|| std::sync::Arc::clone(&re)),
    ))
}

pub type BuiltinFunction = fn(Vec<CfmlValue>) -> CfmlResult;

/// Returns all builtin functions as CfmlValue::Function references for the globals table
pub fn get_builtins() -> ValueMap {
    let mut builtins = ValueMap::default();
    for (name, _) in get_builtin_functions() {
        builtins.insert(name.clone(), create_builtin_func(name.as_str()));
    }
    builtins
}

/// Returns all builtin function implementations
pub fn get_builtin_functions() -> HashMap<String, BuiltinFunction> {
    let mut f: HashMap<String, BuiltinFunction> = HashMap::new();

    // ---- Output functions ----
    f.insert("writeOutput".into(), write_output);
    // `echo()` — Lucee/ACF alias of writeOutput (writes to the page buffer).
    f.insert("echo".into(), write_output);
    f.insert("writeDump".into(), write_dump);
    f.insert("dump".into(), write_dump);
    // `cfdump(var=…)` — the cf-prefixed script-call form of <cfdump>/writeDump.
    f.insert("cfdump".into(), write_dump);

    // ---- String functions ----
    f.insert("len".into(), fn_len);
    f.insert("ucase".into(), fn_ucase);
    f.insert("lcase".into(), fn_lcase);
    f.insert("trim".into(), fn_trim);
    f.insert("ltrim".into(), fn_ltrim);
    f.insert("rtrim".into(), fn_rtrim);
    f.insert("replace".into(), fn_replace);
    f.insert("replaceNoCase".into(), fn_replace_no_case);
    f.insert("find".into(), fn_find);
    f.insert("findNoCase".into(), fn_find_no_case);
    f.insert("findOneOf".into(), fn_find_one_of);
    f.insert("mid".into(), fn_mid);
    f.insert("left".into(), fn_left);
    f.insert("right".into(), fn_right);
    f.insert("reverse".into(), fn_reverse);
    f.insert("repeatString".into(), fn_repeat_string);
    f.insert("insert".into(), fn_insert);
    f.insert("removeChars".into(), fn_remove_chars);
    f.insert("spanIncluding".into(), fn_span_including);
    f.insert("spanExcluding".into(), fn_span_excluding);
    f.insert("compare".into(), fn_compare);
    f.insert("compareNoCase".into(), fn_compare_no_case);
    f.insert("asc".into(), fn_asc);
    f.insert("chr".into(), fn_chr);
    f.insert("reFind".into(), fn_re_find);
    f.insert("reFindNoCase".into(), fn_re_find_no_case);
    f.insert("reReplace".into(), fn_re_replace);
    f.insert("reReplaceNoCase".into(), fn_re_replace_no_case);
    f.insert("reMatch".into(), fn_re_match);
    f.insert("reMatchNoCase".into(), fn_re_match_no_case);
    f.insert("wrap".into(), fn_wrap);
    f.insert("stripCr".into(), fn_strip_cr);
    f.insert("toBase64".into(), fn_to_base64);
    f.insert("toBinary".into(), fn_to_binary);
    f.insert("binaryEncode".into(), fn_binary_encode);
    f.insert("binaryDecode".into(), fn_binary_decode);
    f.insert("urlEncodedFormat".into(), fn_url_encode);
    f.insert("urlDecode".into(), fn_url_decode);
    f.insert("htmlEditFormat".into(), fn_html_edit_format);
    f.insert("htmlCodeFormat".into(), fn_html_code_format);
    f.insert("encodeForHTML".into(), fn_encode_for_html);
    f.insert("lJustify".into(), fn_ljustify);
    f.insert("rJustify".into(), fn_rjustify);
    f.insert("numberFormat".into(), fn_number_format);
    f.insert("decimalFormat".into(), fn_decimal_format);
    f.insert("formatBaseN".into(), fn_format_base_n);
    f.insert("inputBaseN".into(), fn_input_base_n);
    f.insert("replaceList".into(), fn_replace_list);
    f.insert("replaceListNoCase".into(), fn_replace_list_no_case);
    f.insert("xmlFormat".into(), fn_xml_format);
    f.insert("paragraphFormat".into(), fn_paragraph_format);
    f.insert("cJustify".into(), fn_cjustify);
    f.insert("ucFirst".into(), fn_uc_first);
    f.insert("jsStringFormat".into(), fn_js_string_format);
    f.insert("reEscape".into(), fn_re_escape);
    f.insert("getToken".into(), fn_get_token);
    f.insert("newLine".into(), fn_new_line);

    // ---- Array functions ----
    f.insert("arrayNew".into(), fn_array_new);
    f.insert("arrayLen".into(), fn_array_len);
    f.insert("arrayAppend".into(), fn_array_append);
    f.insert("arrayPrepend".into(), fn_array_prepend);
    f.insert("arrayDeleteAt".into(), fn_array_delete_at);
    f.insert("arrayInsertAt".into(), fn_array_insert_at);
    f.insert("arrayContains".into(), fn_array_contains);
    f.insert("arrayContainsNoCase".into(), fn_array_contains_no_case);
    f.insert("arrayFind".into(), fn_array_find);
    f.insert("arrayFindNoCase".into(), fn_array_find_no_case);
    f.insert("arraySort".into(), fn_array_sort);
    f.insert("arrayReverse".into(), fn_array_reverse);
    f.insert("arraySlice".into(), fn_array_slice);
    f.insert("arrayToList".into(), fn_array_to_list);
    f.insert("arrayMerge".into(), fn_array_merge);
    f.insert("arrayClear".into(), fn_array_clear);
    f.insert("arrayIsDefined".into(), fn_array_is_defined);
    f.insert("arraySet".into(), fn_array_set);
    f.insert("arraySwap".into(), fn_array_swap);
    f.insert("arrayMin".into(), fn_array_min);
    f.insert("arrayMax".into(), fn_array_max);
    f.insert("arrayAvg".into(), fn_array_avg);
    f.insert("arraySum".into(), fn_array_sum);
    f.insert("arrayMap".into(), fn_array_map);
    f.insert("arrayFilter".into(), fn_array_filter);
    f.insert("arrayReduce".into(), fn_array_reduce);
    f.insert("arrayEach".into(), fn_array_each);
    f.insert("arraySome".into(), fn_array_each);  // VM intercepts
    f.insert("arrayEvery".into(), fn_array_each);  // VM intercepts
    f.insert("isArray".into(), fn_is_array);
    f.insert("arrayIsEmpty".into(), fn_array_is_empty);
    f.insert("arrayDelete".into(), fn_array_delete);
    f.insert("arrayFindAll".into(), fn_array_find_all);
    f.insert("arrayFindAllNoCase".into(), fn_array_find_all_no_case);
    f.insert("arrayFirst".into(), fn_array_first);
    f.insert("arrayLast".into(), fn_array_last);
    f.insert("arrayPush".into(), fn_array_append);  // alias
    f.insert("arrayUnshift".into(), fn_array_prepend);  // alias
    f.insert("arrayIndexExists".into(), fn_array_index_exists);
    f.insert("arrayResize".into(), fn_array_resize);
    f.insert("arrayMedian".into(), fn_array_median);
    f.insert("arrayMid".into(), fn_array_mid);
    f.insert("arrayReduceRight".into(), fn_array_each);  // VM intercepts
    f.insert("arraySplice".into(), fn_array_splice);
    f.insert("arrayRange".into(), fn_array_range);
    f.insert("arrayToStruct".into(), fn_array_to_struct);
    f.insert("arrayDeleteNoCase".into(), fn_array_delete_no_case);

    // ---- Struct functions ----
    f.insert("structNew".into(), fn_struct_new);
    f.insert("structCount".into(), fn_struct_count);
    f.insert("structKeyExists".into(), fn_struct_key_exists);
    f.insert("structKeyList".into(), fn_struct_key_list);
    f.insert("structKeyArray".into(), fn_struct_key_array);
    f.insert("structDelete".into(), fn_struct_delete);
    f.insert("structInsert".into(), fn_struct_insert);
    f.insert("structUpdate".into(), fn_struct_update);
    f.insert("structFind".into(), fn_struct_find);
    f.insert("structFindKey".into(), fn_struct_find_key);
    f.insert("structFindValue".into(), fn_struct_find_value);
    f.insert("structClear".into(), fn_struct_clear);
    f.insert("structCopy".into(), fn_struct_copy);
    f.insert("structAppend".into(), fn_struct_append);
    f.insert("structIsEmpty".into(), fn_struct_is_empty);
    f.insert("structSort".into(), fn_struct_sort);
    f.insert("structEach".into(), fn_struct_each);
    f.insert("structMap".into(), fn_struct_map);
    f.insert("structFilter".into(), fn_struct_filter);
    f.insert("structReduce".into(), fn_struct_each);  // VM intercepts
    f.insert("structSome".into(), fn_struct_each);  // VM intercepts
    f.insert("structEvery".into(), fn_struct_each);  // VM intercepts
    f.insert("isStruct".into(), fn_is_struct);
    f.insert("structGet".into(), fn_struct_get);
    f.insert("structValueArray".into(), fn_struct_value_array);
    f.insert("structEquals".into(), fn_struct_equals);
    f.insert("structKeyTranslate".into(), fn_struct_key_translate);
    f.insert("structToSorted".into(), fn_struct_to_sorted);
    f.insert("structIsOrdered".into(), fn_struct_is_ordered);
    f.insert("structIsCaseSensitive".into(), fn_struct_is_case_sensitive);
    f.insert("structToQueryString".into(), fn_struct_to_query_string);

    // ---- General utility functions ----
    f.insert("isEmpty".into(), fn_is_empty);

    // ---- Type checking functions ----
    f.insert("isNull".into(), fn_is_null);
    f.insert("isDefined".into(), fn_is_defined);
    f.insert("isSimpleValue".into(), fn_is_simple_value);
    f.insert("isNumeric".into(), fn_is_numeric);
    f.insert("isBoolean".into(), fn_is_boolean);
    f.insert("isDate".into(), fn_is_date);
    f.insert("isQuery".into(), fn_is_query);
    f.insert("isObject".into(), fn_is_object);
    f.insert("isImageFile".into(), fn_is_image_file);
    f.insert("getReadableImageFormats".into(), fn_get_readable_image_formats);
    f.insert("getWriteableImageFormats".into(), fn_get_readable_image_formats);
    register_image_functions(&mut f);
    register_spreadsheet_functions(&mut f);
    f.insert("isBinary".into(), fn_is_binary);
    f.insert("isCustomFunction".into(), fn_is_custom_function);
    f.insert("isClosure".into(), fn_is_closure);
    f.insert("isValid".into(), fn_is_valid);
    f.insert("__cfparam_validate".into(), fn_cfparam_validate);

    // ---- Conversion functions ----
    f.insert("toString".into(), fn_to_string);
    f.insert("toNumeric".into(), fn_to_numeric);
    f.insert("toBoolean".into(), fn_to_boolean);
    f.insert("val".into(), fn_val);
    f.insert("int".into(), fn_int);
    f.insert("javacast".into(), fn_java_cast);
    f.insert("createTimeSpan".into(), fn_create_time_span);
    f.insert("yesNoFormat".into(), fn_yes_no_format);
    f.insert("booleanFormat".into(), fn_yes_no_format);  // alias
    f.insert("trueFalseFormat".into(), fn_true_false_format);
    f.insert("nullValue".into(), fn_null_value);
    f.insert("incrementValue".into(), fn_increment_value);
    f.insert("decrementValue".into(), fn_decrement_value);
    f.insert("de".into(), fn_de);
    f.insert("dollarFormat".into(), fn_dollar_format);

    // ---- Math functions ----
    f.insert("abs".into(), fn_abs);
    f.insert("ceiling".into(), fn_ceiling);
    f.insert("floor".into(), fn_floor);
    f.insert("round".into(), fn_round);
    f.insert("rand".into(), fn_rand);
    f.insert("randRange".into(), fn_rand_range);
    f.insert("randomize".into(), fn_randomize);
    f.insert("max".into(), fn_max);
    f.insert("min".into(), fn_min);
    f.insert("sqr".into(), fn_sqr);
    f.insert("sqrt".into(), fn_sqr);
    f.insert("exp".into(), fn_exp);
    f.insert("log".into(), fn_log);
    f.insert("log10".into(), fn_log10);
    f.insert("sin".into(), fn_sin);
    f.insert("cos".into(), fn_cos);
    f.insert("tan".into(), fn_tan);
    f.insert("asin".into(), fn_asin);
    f.insert("acos".into(), fn_acos);
    f.insert("atan".into(), fn_atan);
    f.insert("pi".into(), fn_pi);
    f.insert("sgn".into(), fn_sgn);
    f.insert("fix".into(), fn_fix);
    f.insert("pow".into(), fn_pow);
    f.insert("bitAnd".into(), fn_bit_and);
    f.insert("bitOr".into(), fn_bit_or);
    f.insert("bitXor".into(), fn_bit_xor);
    f.insert("bitNot".into(), fn_bit_not);
    f.insert("bitSHLN".into(), fn_bit_shln);
    f.insert("bitSHRN".into(), fn_bit_shrn);
    f.insert("bitMaskRead".into(), fn_bit_mask_read);
    f.insert("bitMaskSet".into(), fn_bit_mask_set);
    f.insert("bitMaskClear".into(), fn_bit_mask_clear);

    // ---- Date/Time functions ----
    f.insert("now".into(), fn_now);
    f.insert("createDate".into(), fn_create_date);
    f.insert("createDateTime".into(), fn_create_date_time);
    f.insert("createTime".into(), fn_create_time);
    f.insert("createODBCDate".into(), fn_create_odbc_date);
    f.insert("createODBCDateTime".into(), fn_create_odbc_date_time);
    f.insert("createODBCTime".into(), fn_create_odbc_time);
    f.insert("dateAdd".into(), fn_date_add);
    f.insert("dateDiff".into(), fn_date_diff);
    f.insert("dateFormat".into(), fn_date_format);
    f.insert("timeFormat".into(), fn_time_format);
    f.insert("dateTimeFormat".into(), fn_date_time_format);
    f.insert("parseDateTime".into(), fn_parse_date_time);
    f.insert("datePart".into(), fn_date_part);
    f.insert("dateCompare".into(), fn_date_compare);
    f.insert("year".into(), fn_year);
    f.insert("month".into(), fn_month);
    f.insert("day".into(), fn_day);
    f.insert("hour".into(), fn_hour);
    f.insert("minute".into(), fn_minute);
    f.insert("second".into(), fn_second);
    f.insert("dayOfWeek".into(), fn_day_of_week);
    f.insert("dayOfWeekAsString".into(), fn_day_of_week_as_string);
    f.insert("dayOfWeekShortAsString".into(), fn_day_of_week_short_as_string);
    f.insert("dayOfYear".into(), fn_day_of_year);
    f.insert("daysInMonth".into(), fn_days_in_month);
    f.insert("daysInYear".into(), fn_days_in_year);
    f.insert("firstDayOfMonth".into(), fn_first_day_of_month);
    f.insert("isLeapYear".into(), fn_is_leap_year);
    f.insert("monthAsString".into(), fn_month_as_string);
    f.insert("monthShortAsString".into(), fn_month_short_as_string);
    f.insert("quarter".into(), fn_quarter);
    f.insert("week".into(), fn_week);
    f.insert("millisecond".into(), fn_millisecond);
    f.insert("dateConvert".into(), fn_date_convert);
    f.insert("getNumericDate".into(), fn_get_numeric_date);
    f.insert("getHTTPTimeString".into(), fn_get_http_time_string);
    f.insert("nowServer".into(), fn_now_server);
    f.insert("getTickCount".into(), fn_get_tick_count);
    f.insert("getFunctionList".into(), fn_get_function_list);
    f.insert("getTagData".into(), fn_get_tag_data);
    f.insert("getFunctionCalledName".into(), fn_get_function_called_name);
    f.insert("getContextRoot".into(), fn_get_context_root);
    f.insert("GetContextRoot".into(), fn_get_context_root);
    f.insert("getPageContext".into(), fn_get_page_context);
    f.insert("isInThread".into(), fn_is_in_thread);

    // ---- List functions ----
    f.insert("listNew".into(), fn_list_new);
    f.insert("listLen".into(), fn_list_len);
    f.insert("listAppend".into(), fn_list_append);
    f.insert("listPrepend".into(), fn_list_prepend);
    f.insert("listGetAt".into(), fn_list_get_at);
    f.insert("listSetAt".into(), fn_list_set_at);
    f.insert("listInsertAt".into(), fn_list_insert_at);
    f.insert("listDeleteAt".into(), fn_list_delete_at);
    f.insert("listFind".into(), fn_list_find);
    f.insert("listFindNoCase".into(), fn_list_find_no_case);
    f.insert("listContains".into(), fn_list_contains);
    f.insert("listContainsNoCase".into(), fn_list_contains_no_case);
    f.insert("listSort".into(), fn_list_sort);
    f.insert("listToArray".into(), fn_list_to_array);
    f.insert("listFirst".into(), fn_list_first);
    f.insert("listLast".into(), fn_list_last);
    f.insert("listRest".into(), fn_list_rest);
    f.insert("listRemoveDuplicates".into(), fn_list_remove_duplicates);
    f.insert("listValueCount".into(), fn_list_value_count);
    f.insert("listValueCountNoCase".into(), fn_list_value_count_no_case);
    f.insert("listChangeDelims".into(), fn_list_change_delims);
    f.insert("listQualify".into(), fn_list_qualify);
    f.insert("listCompact".into(), fn_list_compact);
    f.insert("listEach".into(), fn_list_each);
    f.insert("listMap".into(), fn_list_map);
    f.insert("listFilter".into(), fn_list_filter);
    f.insert("listSome".into(), fn_list_each);  // VM intercepts
    f.insert("listEvery".into(), fn_list_each);  // VM intercepts
    f.insert("listAvg".into(), fn_list_avg);
    f.insert("listItemTrim".into(), fn_list_item_trim);
    f.insert("listIndexExists".into(), fn_list_index_exists);
    f.insert("listReduceRight".into(), fn_list_each);  // VM intercepts

    // ---- String higher-order functions (VM-intercepted stubs) ----
    f.insert("stringEach".into(), fn_list_each);     // VM intercepts
    f.insert("stringMap".into(), fn_list_each);      // VM intercepts
    f.insert("stringFilter".into(), fn_list_each);   // VM intercepts
    f.insert("stringReduce".into(), fn_list_each);   // VM intercepts
    f.insert("stringSome".into(), fn_list_each);     // VM intercepts
    f.insert("stringEvery".into(), fn_list_each);    // VM intercepts
    f.insert("stringSort".into(), fn_list_each);     // VM intercepts

    // ---- Collection higher-order functions (VM-intercepted stubs) ----
    f.insert("collectionEach".into(), fn_list_each);    // VM intercepts
    f.insert("collectionMap".into(), fn_list_each);     // VM intercepts
    f.insert("collectionFilter".into(), fn_list_each);  // VM intercepts
    f.insert("collectionReduce".into(), fn_list_each);  // VM intercepts
    f.insert("collectionSome".into(), fn_list_each);    // VM intercepts
    f.insert("collectionEvery".into(), fn_list_each);   // VM intercepts
    f.insert("each".into(), fn_list_each);              // VM intercepts (alias for collectionEach)

    // ---- WebSocket / realtime BIFs (VM-intercepted in cfml-vm/src/lib.rs) ----
    // Registered so the names resolve to callable functions; the real behaviour
    // reaches the connection registry on ServerState, so it is intercepted in
    // the VM before these stub bodies ever run.
    f.insert("io".into(), fn_ws_stub); // VM intercepts
    f.insert("wsPublish".into(), fn_ws_stub); // VM intercepts
    f.insert("wsSubscribe".into(), fn_ws_stub); // VM intercepts
    f.insert("wsUnsubscribe".into(), fn_ws_stub); // VM intercepts
    f.insert("wsPresence".into(), fn_ws_stub); // VM intercepts
    f.insert("assertBroadcast".into(), fn_ws_stub); // VM intercepts (test harness)

    // socket.io-lucee compat seam ($sio*) — the flat BIFs the imperative
    // SocketIoServer/Namespace/Socket CFCs call; all VM-intercepted in lib.rs.
    f.insert("$sioRegisterNamespace".into(), fn_ws_stub);
    f.insert("$sioRegisteredNamespaces".into(), fn_ws_stub);
    f.insert("$sioRegisterNsHandler".into(), fn_ws_stub);
    f.insert("$sioRegisterSocketHandler".into(), fn_ws_stub);
    f.insert("$sioBroadcast".into(), fn_ws_stub);
    f.insert("$sioSend".into(), fn_ws_stub);
    f.insert("$sioJoinRoom".into(), fn_ws_stub);
    f.insert("$sioLeaveRoom".into(), fn_ws_stub);
    f.insert("$sioLeaveAllRooms".into(), fn_ws_stub);
    f.insert("$sioDisconnect".into(), fn_ws_stub);
    f.insert("$sioGetData".into(), fn_ws_stub);
    f.insert("$sioSetData".into(), fn_ws_stub);
    f.insert("$sioSocketCount".into(), fn_ws_stub);

    // ---- JSON functions ----
    f.insert("serializeJSON".into(), fn_serialize_json);
    f.insert("deserializeJSON".into(), fn_deserialize_json);
    f.insert("isJSON".into(), fn_is_json);
    // CFML-literal serialisation (Lucee/ACF): produces a string that
    // `evaluate()` reads back. Distinct from JSON — strings escape `"` by
    // doubling it (`""`) CFML-literal style, not with backslashes.
    f.insert("serialize".into(), fn_serialize);
    // Binary object serialization (ACF/Lucee). RustCFML uses an INTERNAL,
    // self-describing format (magic header + JSON body) that round-trips with
    // itself — it is NOT wire-compatible with JVM object serialization, but
    // objectSave/objectLoad are only ever paired on the same engine (e.g.
    // ColdBox's cache DiskStore marshaller saves then loads).
    f.insert("objectSave".into(), fn_object_save);
    f.insert("objectLoad".into(), fn_object_load);

    // ---- Query functions ----
    f.insert("queryNew".into(), fn_query_new);
    f.insert("queryAddRow".into(), fn_query_add_row);
    f.insert("querySetCell".into(), fn_query_set_cell);
    f.insert("queryAddColumn".into(), fn_query_add_column);
    f.insert("queryGetRow".into(), fn_query_get_row as BuiltinFunction);
    f.insert("queryGetCell".into(), fn_query_get_cell as BuiltinFunction);
    f.insert("queryRecordCount".into(), fn_query_record_count as BuiltinFunction);
    f.insert("queryColumnCount".into(), fn_query_column_count as BuiltinFunction);
    f.insert("queryColumnList".into(), fn_query_column_list as BuiltinFunction);
    f.insert("queryDeleteRow".into(), fn_query_delete_row as BuiltinFunction);
    f.insert("queryDeleteColumn".into(), fn_query_delete_column as BuiltinFunction);
    f.insert("queryAppend".into(), fn_query_append as BuiltinFunction);
    f.insert("queryInsertAt".into(), fn_query_insert_at as BuiltinFunction);
    f.insert("queryPrepend".into(), fn_query_prepend as BuiltinFunction);
    f.insert("queryReverse".into(), fn_query_reverse as BuiltinFunction);
    f.insert("queryRowSwap".into(), fn_query_row_swap as BuiltinFunction);
    f.insert("querySetRow".into(), fn_query_set_row as BuiltinFunction);
    // Higher-order query functions (VM-intercepted stubs)
    f.insert("queryEach".into(), fn_query_ho_stub as BuiltinFunction);
    f.insert("queryMap".into(), fn_query_ho_stub as BuiltinFunction);
    f.insert("queryFilter".into(), fn_query_ho_stub as BuiltinFunction);
    f.insert("queryReduce".into(), fn_query_ho_stub as BuiltinFunction);
    f.insert("querySort".into(), fn_query_ho_stub as BuiltinFunction);
    f.insert("querySome".into(), fn_query_ho_stub as BuiltinFunction);
    f.insert("queryEvery".into(), fn_query_ho_stub as BuiltinFunction);
    f.insert("queryColumnExists".into(), fn_query_column_exists as BuiltinFunction);
    f.insert("queryRowData".into(), fn_query_get_row as BuiltinFunction);  // alias
    f.insert("querySlice".into(), fn_query_slice as BuiltinFunction);
    f.insert("queryGetResult".into(), fn_query_get_result as BuiltinFunction);
    f.insert("queryKeyExists".into(), fn_query_column_exists as BuiltinFunction);  // alias
    f.insert("queryColumnData".into(), fn_query_column_data as BuiltinFunction);
    f.insert("queryColumnArray".into(), fn_query_column_data as BuiltinFunction);  // alias
    f.insert("queryCurrentRow".into(), fn_query_current_row as BuiltinFunction);
    f.insert("__querySetRow".into(), fn_query_move_cursor as BuiltinFunction);
    // QoQ custom-function registration (VM-intercepted).
    f.insert("queryRegisterFunction".into(), fn_query_register_function_stub as BuiltinFunction);

    // ---- Query value list functions ----
    f.insert("valueList".into(), fn_value_list as BuiltinFunction);
    f.insert("valueArray".into(), fn_value_array as BuiltinFunction);
    f.insert("quotedValueList".into(), fn_quoted_value_list as BuiltinFunction);

    // ---- Utility functions ----
    f.insert("evaluate".into(), fn_evaluate);
    f.insert("iif".into(), fn_iif);
    f.insert("duplicate".into(), fn_duplicate);
    f.insert("sleep".into(), fn_sleep);
    f.insert("getMetadata".into(), fn_get_metadata);
    f.insert("isInstanceOf".into(), fn_is_instance_of);
    f.insert("createObject".into(), fn_create_object);
    f.insert("getDirectoryFromPath".into(), fn_get_directory_from_path);
    f.insert("getComponentMetadata".into(), fn_get_component_metadata);
    f.insert("getComponentStaticScope".into(), fn_get_component_static_scope);
    f.insert("createUUID".into(), fn_create_uuid);
    f.insert("createUniqueID".into(), fn_create_unique_id);
    f.insert("preserveSingleQuotes".into(), fn_preserve_single_quotes);
    f.insert("createGUID".into(), fn_create_guid);
    f.insert("hash".into(), fn_hash);
    f.insert("lsParseNumber".into(), fn_ls_parse_number);

    // ---- System functions ----
    f.insert("getTickCount".into(), fn_get_tick_count);
    f.insert("getFunctionList".into(), fn_get_function_list);
    f.insert("getCurrentTemplatePath".into(), fn_get_current_template_path);
    f.insert("getBaseTemplatePath".into(), fn_get_base_template_path);
    f.insert("getTimeZone".into(), fn_get_time_zone);
    f.insert("getTimeZoneInfo".into(), fn_get_time_zone_info);
    f.insert("getContextRoot".into(), fn_get_context_root);
    f.insert("GetContextRoot".into(), fn_get_context_root);
    f.insert("getPageContext".into(), fn_get_page_context);
    f.insert("isInThread".into(), fn_is_in_thread);
    f.insert("getFileFromPath".into(), fn_get_file_from_path);
    f.insert("getCanonicalPath".into(), fn_get_canonical_path);
    f.insert("systemOutput".into(), fn_system_output);
    f.insert("systemCacheClear".into(), fn_system_cache_clear);
    f.insert("getEnvironmentVariable".into(), fn_get_environment_variable);
    f.insert("readLine".into(), fn_read_line);
    f.insert("getTemplatePath".into(), fn_get_current_template_path);  // alias
    f.insert("writeLog".into(), fn_write_log);
    f.insert("setLocale".into(), fn_set_locale);
    f.insert("getLocale".into(), fn_get_locale);
    f.insert("setTimeZone".into(), fn_set_time_zone);
    f.insert("setEncoding".into(), fn_set_encoding);

    // ---- Locale (ls*) functions ----
    f.insert("lsDateFormat".into(), fn_ls_date_format);
    f.insert("lsTimeFormat".into(), fn_ls_time_format);
    f.insert("lsDateTimeFormat".into(), fn_ls_date_time_format);
    f.insert("lsCurrencyFormat".into(), fn_ls_currency_format);
    f.insert("lsEuroCurrencyFormat".into(), fn_ls_euro_currency_format);
    f.insert("lsIsDate".into(), fn_ls_is_date);
    f.insert("lsIsNumeric".into(), fn_ls_is_numeric);
    f.insert("lsIsCurrency".into(), fn_ls_is_currency);
    f.insert("lsParseCurrency".into(), fn_ls_parse_currency);
    f.insert("lsParseDateTime".into(), fn_ls_parse_date_time);
    f.insert("lsNumberFormat".into(), fn_ls_number_format);
    f.insert("lsWeek".into(), fn_ls_week);
    f.insert("lsDayOfWeek".into(), fn_ls_day_of_week);
    f.insert("applicationStop".into(), fn_application_stop);
    f.insert("getApplicationMetadata".into(), fn_get_application_metadata);
    f.insert("getApplicationSettings".into(), fn_get_application_metadata);  // alias
    // `location` (script alias for `cflocation`) is intentionally NOT registered
    // as a builtin: it must flow through the LoadGlobal script-tag-call mapping to
    // the `__cflocation` VM intercept (see `lib.rs`). Registering it as a builtin
    // shadowed that mapping and hit the stub — a 500 "requires VM intercept".
    f.insert("trace".into(), fn_trace);
    f.insert("exceptionKeyExists".into(), fn_exception_key_exists);
    // Classic CF debug-footer BIFs. These are VM-intercepted in lib.rs when the
    // `observability` feature is on (they need the per-request collector + live
    // scopes); the stubs below are the fallback for feature-off builds (e.g. the
    // wasm worker) so a page calling them never errors.
    f.insert("getDebugData".into(), fn_get_debug_data_stub);
    f.insert("isDebugMode".into(), fn_is_debug_mode_stub);
    f.insert("debugAdd".into(), fn_debug_add_stub);
    // Sampling-profiler BIFs (Phase 2) — VM-intercepted when observability is on;
    // these stubs keep pages portable on feature-off / wasm builds.
    f.insert("getRequestProfile".into(), fn_get_request_profile_stub);
    f.insert("profileNow".into(), fn_profile_now_stub);

    // ---- File I/O functions ----
    f.insert("fileRead".into(), fn_file_read);
    f.insert("fileWrite".into(), fn_file_write);
    f.insert("fileAppend".into(), fn_file_append);
    f.insert("fileExists".into(), fn_file_exists);
    f.insert("fileDelete".into(), fn_file_delete);
    f.insert("fileMove".into(), fn_file_move);
    f.insert("fileCopy".into(), fn_file_copy);
    f.insert("directoryCreate".into(), fn_directory_create);
    f.insert("directoryExists".into(), fn_directory_exists);
    f.insert("directoryDelete".into(), fn_directory_delete);
    f.insert("directoryList".into(), fn_directory_list);
    f.insert("getTempDirectory".into(), fn_get_temp_directory);
    f.insert("getTempFile".into(), fn_get_temp_file);
    f.insert("getFileInfo".into(), fn_get_file_info);
    f.insert("expandPath".into(), fn_expand_path);
    f.insert("fileReadBinary".into(), fn_file_read_binary);
    f.insert("fileGetMimeType".into(), fn_file_get_mime_type);
    f.insert("directoryRename".into(), fn_directory_rename);
    f.insert("directoryCopy".into(), fn_directory_copy);
    f.insert("fileOpen".into(), fn_file_open);
    f.insert("fileClose".into(), fn_file_close);
    f.insert("fileReadLine".into(), fn_file_read_line);
    f.insert("fileWriteLine".into(), fn_file_write_line);
    f.insert("fileIsEOF".into(), fn_file_is_eof);
    f.insert("fileUpload".into(), fn_file_upload);
    f.insert("fileUploadAll".into(), fn_file_upload_all);
    f.insert("__cffile_upload".into(), fn_cffile_upload);
    f.insert("getProfileString".into(), fn_get_profile_string);
    f.insert("setProfileString".into(), fn_set_profile_string);
    f.insert("getProfileSections".into(), fn_get_profile_sections);

    // ---- Additional builtins ----
    f.insert("encodeForURL".into(), fn_encode_for_url);
    f.insert("encodeForCSS".into(), fn_encode_for_css);
    f.insert("encodeForJavaScript".into(), fn_encode_for_javascript);
    f.insert("charsetDecode".into(), fn_charset_decode);
    f.insert("charsetEncode".into(), fn_charset_encode);
    f.insert("encodeForHTMLAttribute".into(), fn_encode_for_html_attribute);
    f.insert("encodeForXML".into(), fn_encode_for_xml);
    f.insert("encodeForXMLAttribute".into(), fn_encode_for_xml_attribute);
    f.insert("encodeFor".into(), fn_encode_for);
    f.insert("decodeForHTML".into(), fn_decode_for_html);
    f.insert("decodeFromURL".into(), fn_decode_from_url);
    f.insert("urlEncode".into(), fn_url_encode_alias);
    f.insert("canonicalize".into(), fn_canonicalize);
    f.insert("listReduce".into(), fn_list_reduce);
    f.insert("arrayPop".into(), fn_array_pop);
    f.insert("arrayShift".into(), fn_array_shift);

    // ---- HTTP/Tag infrastructure (VM-intercepted) ----
    f.insert("__cfheader".into(), fn_cfheader_stub);
    f.insert("__cfcontent".into(), fn_cfcontent_stub);
    f.insert("__cflocation".into(), fn_cflocation_stub);
    f.insert("getHTTPRequestData".into(), fn_get_http_request_data_stub);
    f.insert("__cfinvoke".into(), fn_cfinvoke_stub);
    f.insert("__cfsavecontent_start".into(), fn_cfsavecontent_start_stub);
    f.insert("__cfsavecontent_end".into(), fn_cfsavecontent_end_stub);
    f.insert("__cfabort".into(), fn_cfabort_stub);
    f.insert("__cfexit".into(), fn_cfexit_stub);
    f.insert("__cfhtmlhead".into(), fn_cfhtmlhead_stub);
    f.insert("__cfhtmlbody".into(), fn_cfhtmlbody_stub);
    f.insert("invoke".into(), fn_invoke_stub);
    f.insert("__cftransaction_start".into(), fn_cftransaction_start_stub);
    f.insert("__cftransaction_commit".into(), fn_cftransaction_commit_stub);
    f.insert("__cftransaction_rollback".into(), fn_cftransaction_rollback_stub);
    f.insert("cfdirectory".into(), fn_cfdirectory);
    f.insert("cffile".into(), fn_cffile);
    f.insert("cfdbinfo".into(), fn_cfdbinfo_stub);
    f.insert("dbinfo".into(), fn_cfdbinfo_stub);
    #[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
    f.insert("__dbinfo_impl".into(), crate::dbinfo::fn_dbinfo_impl);
    #[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
    f.insert("__register_ds_timeout".into(), fn_register_ds_timeout);
    f.insert("__cflog".into(), fn_cflog_stub);
    f.insert("__cfparam".into(), fn_cfparam_stub);
    f.insert("__cfsetting".into(), fn_cfsetting_stub);
    f.insert("__cfapplication".into(), fn_cfapplication_stub);
    f.insert("__cflock_start".into(), fn_cflock_start_stub);
    f.insert("__cflock_end".into(), fn_cflock_end_stub);
    f.insert("__cfcookie".into(), fn_cfcookie_stub);
    f.insert("__cfcache".into(), fn_cfcache_stub);
    f.insert("__cfloop_file_lines".into(), fn_cfloop_file_lines_stub);
    f.insert("__cfexecute".into(), fn_cfexecute_stub);
    f.insert("__cfmail".into(), fn_cfmail);

    // ---- Whitespace/output control functions (VM-intercepted) ----
    f.insert("__writeText".into(), fn_write_text_stub);
    f.insert("__cfprocessingdirective_collapse".into(), fn_cfprocessingdirective_collapse);

    // ---- cfthread functions (VM-intercepted) ----
    f.insert("__cfthread_run".into(), fn_cfthread_stub);
    f.insert("__cfthread_join".into(), fn_cfthread_stub);
    f.insert("__cfthread_terminate".into(), fn_cfthread_stub);
    // Script BIFs threadJoin()/threadTerminate() route to the same handlers.
    f.insert("threadjoin".into(), fn_cfthread_stub);
    f.insert("threadterminate".into(), fn_cfthread_stub);

    // ---- async kernel: runAsync + _schedule (VM-intercepted) ----
    f.insert("runAsync".into(), fn_async_stub);
    f.insert("_schedule".into(), fn_async_stub);
    // createDynamicProxy: wraps a CFC as a Java SAM (Callable/Runnable/…) so the
    // java.util.concurrent shim can invoke it. VM-intercepted. See lib.rs.
    f.insert("createDynamicProxy".into(), fn_async_stub);

    // ---- Cache functions (VM-intercepted) ----
    f.insert("cachePut".into(), fn_cache_stub);
    f.insert("cacheGet".into(), fn_cache_stub);
    f.insert("cacheDelete".into(), fn_cache_stub);
    f.insert("cacheClear".into(), fn_cache_stub);
    f.insert("cacheKeyExists".into(), fn_cache_stub);
    f.insert("cacheCount".into(), fn_cache_stub);
    f.insert("cacheGetAll".into(), fn_cache_stub);
    f.insert("cacheGetAllIds".into(), fn_cache_stub);
    f.insert("cacheGetProperties".into(), fn_cache_stub);

    // ---- Session & Auth functions (VM-intercepted) ----
    f.insert("sessionInvalidate".into(), fn_session_stub);
    f.insert("sessionRotate".into(), fn_session_stub);
    f.insert("sessionCommit".into(), fn_session_stub);
    f.insert("sessionGetMetaData".into(), fn_session_stub);
    f.insert("getAuthUser".into(), fn_session_stub);
    f.insert("isUserInRole".into(), fn_session_stub);
    f.insert("isUserLoggedIn".into(), fn_session_stub);
    f.insert("__cfloginuser".into(), fn_session_stub);
    f.insert("__cflogout".into(), fn_session_stub);

    // ---- Variable scope functions (VM-intercepted) ----
    f.insert("setVariable".into(), fn_session_stub);
    f.insert("getVariable".into(), fn_session_stub);

    // ---- throw() function form (VM-intercepted) ----
    f.insert("throw".into(), fn_session_stub);

    // ---- Struct metadata functions ----
    f.insert("structGetMetadata".into(), fn_struct_get_metadata);
    f.insert("structSetMetadata".into(), fn_struct_set_metadata);

    // ---- File attribute functions ----
    f.insert("fileSetAccessMode".into(), fn_file_set_access_mode);
    f.insert("fileSetAttribute".into(), fn_file_set_attribute);
    f.insert("fileSetLastModified".into(), fn_file_set_last_modified);

    // ---- HTTP functions ----
    #[cfg(feature = "http")]
    f.insert("cfhttp".into(), fn_cfhttp);

    // ---- Database functions ----
    #[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
    f.insert("queryExecute".into(), fn_query_execute);
    // No per-engine DB feature compiled in (e.g. the Cloudflare Workers
    // build). Fall back to the dynamic-driver-only path so cfquery /
    // queryExecute against an externally-registered driver (D1, etc.)
    // still works.
    #[cfg(not(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db")))]
    f.insert("queryExecute".into(), fn_query_execute_dynamic);

    // ---- Security functions ----
    f.insert("hmac".into(), fn_hmac);
    // JWT (Lucee crypto-extension names) — HMAC algorithms (HS256/384/512).
    f.insert("jwtSign".into(), fn_jwt_sign);
    f.insert("jwtVerify".into(), fn_jwt_verify);
    f.insert("jwtDecode".into(), fn_jwt_decode);
    #[cfg(feature = "security")]
    f.insert("generateSecretKey".into(), fn_generate_secret_key);
    f.insert("encrypt".into(), fn_encrypt);
    f.insert("decrypt".into(), fn_decrypt);

    // ---- Password hashing / CSRF functions ----
    #[cfg(feature = "security")]
    {
        f.insert("generatePBKDFKey".into(), fn_generate_pbkdf_key);
        f.insert("generateBCryptHash".into(), fn_generate_bcrypt_hash);
        f.insert("verifyBCryptHash".into(), fn_verify_bcrypt_hash);
        // Lucee crypto-extension modern names (GenerateBCryptHash/VerifyBCryptHash
        // are the deprecated predecessors).
        f.insert("bcryptHash".into(), fn_bcrypt_hash);
        f.insert("bcryptVerify".into(), fn_bcrypt_verify);
        f.insert("generateSCryptHash".into(), fn_generate_scrypt_hash);
        f.insert("verifySCryptHash".into(), fn_verify_scrypt_hash);
        f.insert("generateArgon2Hash".into(), fn_generate_argon2_hash);
        f.insert("argon2CheckHash".into(), fn_argon2_check_hash);
        f.insert("csrfGenerateToken".into(), fn_csrf_generate_token);
        f.insert("csrfVerifyToken".into(), fn_csrf_verify_token);
    }

    // ---- YAML functions (BoxLang-compatible names) ----
    #[cfg(feature = "yaml")]
    {
        f.insert("yamlDeserialize".into(), fn_yaml_deserialize);
        f.insert("yamlSerialize".into(), fn_yaml_serialize);
        f.insert("yamlDeserializeFile".into(), fn_yaml_deserialize_file);
    }

    // ---- JSON Schema validation (Lucee `validateJSON`) ----
    #[cfg(feature = "jsonschema")]
    {
        f.insert("validateJSON".into(), fn_validate_json);
        // Internal: returns Preside's {valid,error} JSON string for the
        // ca.vanmulligen.json.schema.Validator shim's isValid().
        f.insert("__jsonSchemaValidateResult".into(), fn_json_schema_validate_result);
    }

    // ---- XML functions ----
    #[cfg(feature = "xml")]
    {
        f.insert("xmlParse".into(), fn_xml_parse);
        // XMP metadata (RDF/XML) flattener — replaces Preside's xmpcore.jar.
        f.insert("xmpParse".into(), crate::xmp::fn_xmp_parse);
        f.insert("xmlSearch".into(), fn_xml_search);
        f.insert("isXML".into(), fn_is_xml);
        f.insert("xmlTransform".into(), fn_xml_transform_stub);
        f.insert("xmlValidate".into(), fn_xml_validate_stub);
        f.insert("xmlNew".into(), fn_xml_new);
        f.insert("xmlElemNew".into(), fn_xml_elem_new);
        f.insert("xmlChildPos".into(), fn_xml_child_pos);
        f.insert("xmlGetNodeType".into(), fn_xml_get_node_type);
        f.insert("xmlHasChild".into(), fn_xml_has_child);
        f.insert("isXMLDoc".into(), fn_is_xml_doc);
        f.insert("isXMLElem".into(), fn_is_xml_elem);
        f.insert("isXMLNode".into(), fn_is_xml_node);
        f.insert("isXMLRoot".into(), fn_is_xml_root);
        f.insert("isXMLAttribute".into(), fn_is_xml_attribute);
    }

    // ---- HTML functions ----
    #[cfg(feature = "html")]
    {
        f.insert("htmlParse".into(), fn_html_parse);
    }

    // ---- Zip functions ----
    #[cfg(feature = "zip_support")]
    {
        f.insert("cfzip".into(), fn_cfzip);
    }

    // ---- Misc functions ----
    f.insert("soundex".into(), fn_soundex);
    f.insert("metaphone".into(), fn_metaphone);
    f.insert("toScript".into(), fn_to_script);

    // ---- S3 functions (gated on `s3` feature) ----
    #[cfg(feature = "s3")]
    {
        f.insert("s3Read".into(), crate::s3_builtins::fn_s3_read);
        f.insert("s3ReadBinary".into(), crate::s3_builtins::fn_s3_read_binary);
        f.insert("s3Write".into(), crate::s3_builtins::fn_s3_write);
        f.insert("s3Upload".into(), crate::s3_builtins::fn_s3_upload);
        f.insert("s3Download".into(), crate::s3_builtins::fn_s3_download);
        f.insert("s3ListBuckets".into(), crate::s3_builtins::fn_s3_list_buckets);
        f.insert("s3ListBucket".into(), crate::s3_builtins::fn_s3_list_bucket);
        f.insert("s3CreateBucket".into(), crate::s3_builtins::fn_s3_create_bucket);
        f.insert("s3Delete".into(), crate::s3_builtins::fn_s3_delete);
        f.insert("s3ClearBucket".into(), crate::s3_builtins::fn_s3_clear_bucket);
        f.insert("s3Exists".into(), crate::s3_builtins::fn_s3_exists);
        f.insert("s3Copy".into(), crate::s3_builtins::fn_s3_copy);
        f.insert("s3Move".into(), crate::s3_builtins::fn_s3_move);
        f.insert("s3GetMetaData".into(), crate::s3_builtins::fn_s3_get_metadata);
        f.insert(
            "s3GeneratePresignedURL".into(),
            crate::s3_builtins::fn_s3_generate_presigned_url,
        );
        f.insert("s3GenerateURI".into(), crate::s3_builtins::fn_s3_generate_uri);
        f.insert("storeGetMetadata".into(), crate::s3_builtins::fn_store_get_metadata);
    }
    #[cfg(not(feature = "s3"))]
    {
        for name in &[
            "s3Read",
            "s3ReadBinary",
            "s3Write",
            "s3Upload",
            "s3Download",
            "s3ListBuckets",
            "s3ListBucket",
            "s3CreateBucket",
            "s3Delete",
            "s3ClearBucket",
            "s3Exists",
            "s3Copy",
            "s3Move",
            "s3GetMetaData",
            "s3GeneratePresignedURL",
            "s3GenerateURI",
            "storeGetMetadata",
        ] {
            f.insert((*name).into(), fn_s3_unavailable_stub);
        }
    }

    f
}

#[cfg(not(feature = "s3"))]
fn fn_s3_unavailable_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime(
        "S3 support not compiled in. Rebuild with `--features s3` on cfml-stdlib.".to_string(),
    ))
}

fn create_builtin_func(name: &str) -> CfmlValue {
    CfmlValue::Function(std::sync::Arc::new(CfmlFunction {
        name: name.to_string(),
        params: Vec::new(),
        body: CfmlClosureBody::Expression(Box::new(CfmlValue::Null)),
        return_type: None,
        access: CfmlAccess::Public,
        captured_scope: None,
    }))
}

// ---- Helper functions ----

#[allow(dead_code)]
fn get_arg(args: &[CfmlValue], idx: usize) -> &CfmlValue {
    args.get(idx).unwrap_or(&CfmlValue::Null)
}

fn get_str(args: &[CfmlValue], idx: usize) -> String {
    args.get(idx).map(|v| v.as_string()).unwrap_or_default()
}

fn get_int(args: &[CfmlValue], idx: usize) -> i64 {
    // A QueryColumn proxy coerces to its first-row value in scalar contexts.
    match args.get(idx).map(|v| v.query_column_scalar()) {
        Some(CfmlValue::Int(i)) => *i,
        Some(CfmlValue::Double(d)) => *d as i64,
        Some(CfmlValue::String(s)) => s.parse().unwrap_or(0),
        Some(CfmlValue::Bool(b)) => if *b { 1 } else { 0 },
        _ => 0,
    }
}

fn get_float(args: &[CfmlValue], idx: usize) -> f64 {
    // A QueryColumn proxy coerces to its first-row value in scalar contexts.
    match args.get(idx).map(|v| v.query_column_scalar()) {
        Some(CfmlValue::Int(i)) => *i as f64,
        Some(CfmlValue::Double(d)) => *d,
        Some(CfmlValue::String(s)) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn get_delimiter(args: &[CfmlValue], idx: usize) -> String {
    args.get(idx)
        .map(|v| v.as_string())
        .unwrap_or_else(|| ",".to_string())
}

/// Case-insensitive key lookup for CFML structs. Returns the actual key in the IndexMap.
/// Find the actual (case-preserving) key in a struct matching `key`
/// case-insensitively. Returns an owned `String` because the backing map is
/// behind a lock and can't be borrowed out.
fn struct_find_key_ci(s: &CfmlStruct, key: &str) -> Option<String> {
    // v0.442 (issue #262) — O(1) resolution via the struct's ci index, instead
    // of the old O(n) `keys().find(eq_ignore_ascii_case)` scan that made
    // `StructKeyExists` on a large struct O(n) per call.
    if let Some(found) = s.key_ci(key) {
        return Some(found);
    }
    // Live `variables.this` alias (Lucee/ACF): `StructKeyExists(variables,
    // "this")` must be true on a CFC private scope so framework code can gate
    // a public mixin append on it (Wheels Plugins.cfc). Checked outside the
    // `with_read` guard above — parking_lot is not reentrant.
    if key.eq_ignore_ascii_case("this") && s.this_alias_struct().is_some() {
        return Some("this".to_string());
    }
    None
}

/// CFML list splitting: each character in `delimiters` is a separate delimiter.
/// Empty elements are excluded (CFML default behavior).
fn cfml_list_split<'a>(list: &'a str, delimiters: &str) -> Vec<&'a str> {
    if list.is_empty() {
        return Vec::new();
    }
    list.split(|c: char| delimiters.contains(c))
        .filter(|s| !s.is_empty())
        .collect()
}

/// CFML list splitting that keeps empty elements (for includeEmptyValues=true).
fn cfml_list_split_keep_empty<'a>(list: &'a str, delimiters: &str) -> Vec<&'a str> {
    if list.is_empty() {
        return Vec::new();
    }
    list.split(|c: char| delimiters.contains(c)).collect()
}

/// Map a 1-based CFML list element index (which counts only NON-EMPTY fields, like
/// ListLen) to a position in the empty-preserving field vector. ListSetAt/InsertAt/
/// DeleteAt index by non-empty element but keep all empty fields in the output.
fn nth_nonempty_field_pos(fields: &[&str], index_1based: usize) -> Option<usize> {
    if index_1based == 0 {
        return None;
    }
    let mut seen = 0usize;
    for (pos, f) in fields.iter().enumerate() {
        if !f.is_empty() {
            seen += 1;
            if seen == index_1based {
                return Some(pos);
            }
        }
    }
    None
}

// Thread-local xorshift64 PRNG state for deterministic randomize()/rand() support
thread_local! {
    static PRNG_STATE: std::cell::Cell<u64> = std::cell::Cell::new(0);
    static PRNG_SEEDED: std::cell::Cell<bool> = std::cell::Cell::new(false);
}

fn xorshift64(state: u64) -> u64 {
    let mut x = state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn cfml_random() -> f64 {
    PRNG_SEEDED.with(|_seeded| {
        PRNG_STATE.with(|state| {
            let current = state.get();
            let next = if current == 0 {
                // Lazy initialization: seed from nanosecond clock on first use
                cfml_common::clock::now_unix_nanos() as u64
            } else {
                xorshift64(current)
            };
            state.set(next);
            (next >> 11) as f64 / (1u64 << 53) as f64
        })
    })
}

// ===============================================
// OUTPUT FUNCTIONS
// ===============================================

fn write_output(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(val) = args.first() {
        print!("{}", val.as_string());
    }
    Ok(CfmlValue::Null)
}

fn write_dump(args: Vec<CfmlValue>) -> CfmlResult {
    for arg in &args {
        println!("{:?}", arg);
    }
    Ok(CfmlValue::Null)
}

// ===============================================
// STRING FUNCTIONS
// ===============================================

fn fn_len(args: Vec<CfmlValue>) -> CfmlResult {
    match args.first() {
        // CFML len() is a CHARACTER count, not a byte count. `mid`/`left`/`right`
        // and the java.util.regex Matcher shim (start/end/group) are all
        // char-indexed, so a byte-based len() here desyncs any code that mixes
        // len() with those (e.g. Preside's DynamicFindAndReplaceService slices
        // `Right(source, Len(source) - charPos)`, which over-read by the UTF-8
        // byte gap of every multibyte char — the `▾` twisties in a writeDump —
        // and re-appended the tail of the delayed-Sticker `<!--ds:…:ds-->` marker
        // into the response).
        Some(CfmlValue::String(s)) => Ok(CfmlValue::Int(s.chars().count() as i64)),
        Some(CfmlValue::Bool(_)) | Some(CfmlValue::Int(_)) | Some(CfmlValue::Double(_)) => Ok(
            CfmlValue::Int(args.first().unwrap().as_string().chars().count() as i64),
        ),
        Some(CfmlValue::Array(a)) => Ok(CfmlValue::Int(a.len() as i64)),
        Some(CfmlValue::Struct(s)) => Ok(CfmlValue::Int(s.len() as i64)),
        Some(CfmlValue::Binary(b)) => Ok(CfmlValue::Int(b.len() as i64)),
        // Lucee@7 parity: len(q.col) treats the column as a string and returns
        // the first row's stringified length. This deliberately disagrees with
        // arrayLen() — which errors instead, matching Lucee's stricter rules.
        Some(v @ CfmlValue::QueryColumn(..)) => Ok(CfmlValue::Int(v.as_string().chars().count() as i64)),
        _ => Ok(CfmlValue::Int(0)),
    }
}

fn fn_ucase(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::string(get_str(&args, 0).to_uppercase()))
}

fn fn_lcase(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::string(get_str(&args, 0).to_lowercase()))
}

fn fn_trim(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::string(get_str(&args, 0).trim().to_string()))
}

fn fn_ltrim(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::string(get_str(&args, 0).trim_start().to_string()))
}

fn fn_rtrim(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::string(get_str(&args, 0).trim_end().to_string()))
}

fn fn_replace(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 3 {
        let string = get_str(&args, 0);
        let find = get_str(&args, 1);
        let replace_with = get_str(&args, 2);
        let scope = if args.len() >= 4 { get_str(&args, 3).to_lowercase() } else { "one".to_string() };
        if scope == "all" {
            Ok(CfmlValue::string(string.replace(&find, &replace_with)))
        } else {
            Ok(CfmlValue::string(string.replacen(&find, &replace_with, 1)))
        }
    } else {
        Ok(CfmlValue::string(get_str(&args, 0)))
    }
}

fn fn_replace_no_case(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 3 {
        let string = get_str(&args, 0);
        let find = get_str(&args, 1);
        let replace_with = get_str(&args, 2);
        let scope = if args.len() >= 4 { get_str(&args, 3).to_lowercase() } else { "one".to_string() };
        let find_lower = find.to_lowercase();

        if scope == "all" {
            let mut result = String::new();
            let lower = string.to_lowercase();
            let mut start = 0;
            while let Some(pos) = lower[start..].find(&find_lower) {
                result.push_str(&string[start..start + pos]);
                result.push_str(&replace_with);
                start += pos + find.len();
            }
            result.push_str(&string[start..]);
            Ok(CfmlValue::string(result))
        } else {
            let lower = string.to_lowercase();
            if let Some(pos) = lower.find(&find_lower) {
                let mut result = String::new();
                result.push_str(&string[..pos]);
                result.push_str(&replace_with);
                result.push_str(&string[pos + find.len()..]);
                Ok(CfmlValue::string(result))
            } else {
                Ok(CfmlValue::string(string))
            }
        }
    } else {
        Ok(CfmlValue::string(get_str(&args, 0)))
    }
}

/// Byte offset in `s` for the given 0-based CHARACTER index. Returns `s.len()`
/// when the index is at or past the end (usable directly as a search start).
fn char_index_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map(|(b, _)| b).unwrap_or_else(|| s.len())
}

/// 0-based CHARACTER count preceding the given byte offset in `s`.
fn byte_to_char_index(s: &str, byte_idx: usize) -> usize {
    s[..byte_idx.min(s.len())].chars().count()
}

/// Char-based substring search. `start_char` is a 0-based character index.
/// Returns a 1-based CHARACTER position, or 0 if not found. CFML positions are
/// all character-based (Len/Mid/Left/Right are already char-based here), so
/// Find must be too — a byte-based position desynced with Mid on any non-ASCII
/// string (GitHub #248).
fn find_substr_char(haystack: &str, needle: &str, start_char: usize) -> i64 {
    if start_char > haystack.chars().count() {
        return 0;
    }
    let byte_start = char_index_to_byte(haystack, start_char);
    match haystack[byte_start..].find(needle) {
        Some(bpos) => (byte_to_char_index(haystack, byte_start + bpos) + 1) as i64,
        None => 0,
    }
}

fn fn_find(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        let substring = get_str(&args, 0);
        let string = get_str(&args, 1);
        let start = if args.len() >= 3 { get_int(&args, 2).max(1) as usize - 1 } else { 0 };
        Ok(CfmlValue::Int(find_substr_char(&string, &substring, start)))
    } else {
        Ok(CfmlValue::Int(0))
    }
}

fn fn_find_no_case(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        let substring = get_str(&args, 0).to_lowercase();
        let string = get_str(&args, 1).to_lowercase();
        let start = if args.len() >= 3 { get_int(&args, 2).max(1) as usize - 1 } else { 0 };
        Ok(CfmlValue::Int(find_substr_char(&string, &substring, start)))
    } else {
        Ok(CfmlValue::Int(0))
    }
}

fn fn_find_one_of(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        let chars = get_str(&args, 0);
        let string = get_str(&args, 1);
        let start = if args.len() >= 3 { (get_int(&args, 2) as usize).saturating_sub(1) } else { 0 };
        for (i, c) in string.chars().enumerate().skip(start) {
            if chars.contains(c) {
                return Ok(CfmlValue::Int((i + 1) as i64));
            }
        }
        Ok(CfmlValue::Int(0))
    } else {
        Ok(CfmlValue::Int(0))
    }
}

fn fn_mid(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 3 {
        let string = get_str(&args, 0);
        let start = (get_int(&args, 1).max(1) as usize).saturating_sub(1);
        let length = get_int(&args, 2).max(0) as usize;
        let chars: Vec<char> = string.chars().collect();
        if start >= chars.len() {
            return Ok(CfmlValue::string(String::new()));
        }
        let end = (start + length).min(chars.len());
        Ok(CfmlValue::string(chars[start..end].iter().collect::<String>()))
    } else if args.len() >= 2 {
        let string = get_str(&args, 0);
        let start = (get_int(&args, 1).max(1) as usize).saturating_sub(1);
        let chars: Vec<char> = string.chars().collect();
        if start >= chars.len() {
            return Ok(CfmlValue::string(String::new()));
        }
        Ok(CfmlValue::string(chars[start..].iter().collect::<String>()))
    } else {
        Ok(CfmlValue::string(String::new()))
    }
}

fn fn_left(args: Vec<CfmlValue>) -> CfmlResult {
    let string = get_str(&args, 0);
    let count = get_int(&args, 1).max(0) as usize;
    let chars: Vec<char> = string.chars().collect();
    Ok(CfmlValue::string(chars[..count.min(chars.len())].iter().collect::<String>()))
}

fn fn_right(args: Vec<CfmlValue>) -> CfmlResult {
    let string = get_str(&args, 0);
    let count = get_int(&args, 1).max(0) as usize;
    let chars: Vec<char> = string.chars().collect();
    let start = chars.len().saturating_sub(count);
    Ok(CfmlValue::string(chars[start..].iter().collect::<String>()))
}

fn fn_reverse(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::string(get_str(&args, 0).chars().rev().collect::<String>()))
}

fn fn_repeat_string(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let count = get_int(&args, 1).max(0) as usize;
    Ok(CfmlValue::string(s.repeat(count)))
}

fn fn_insert(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 3 {
        let substring = get_str(&args, 0);
        let string = get_str(&args, 1);
        // `pos` is a CHARACTER offset (insert AFTER this many chars). Byte-based
        // insert_str desynced with the char-based Find/Mid family and could panic
        // on a non-char-boundary offset for non-ASCII input (GitHub #248).
        let pos_char = get_int(&args, 2).max(0) as usize;
        let char_count = string.chars().count();
        let mut result = string.clone();
        if pos_char <= char_count {
            let byte_pos = char_index_to_byte(&string, pos_char);
            result.insert_str(byte_pos, &substring);
        }
        Ok(CfmlValue::string(result))
    } else {
        Ok(CfmlValue::string(get_str(&args, 0)))
    }
}

fn fn_remove_chars(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 3 {
        let string = get_str(&args, 0);
        let start = (get_int(&args, 1).max(1) as usize).saturating_sub(1);
        let count = get_int(&args, 2).max(0) as usize;
        let mut chars: Vec<char> = string.chars().collect();
        let end = (start + count).min(chars.len());
        chars.drain(start..end);
        Ok(CfmlValue::string(chars.into_iter().collect::<String>()))
    } else {
        Ok(CfmlValue::string(get_str(&args, 0)))
    }
}

fn fn_span_including(args: Vec<CfmlValue>) -> CfmlResult {
    let string = get_str(&args, 0);
    let chars_set = get_str(&args, 1);
    let result: String = string.chars().take_while(|c| chars_set.contains(*c)).collect();
    Ok(CfmlValue::string(result))
}

fn fn_span_excluding(args: Vec<CfmlValue>) -> CfmlResult {
    let string = get_str(&args, 0);
    let chars_set = get_str(&args, 1);
    let result: String = string.chars().take_while(|c| !chars_set.contains(*c)).collect();
    Ok(CfmlValue::string(result))
}

fn fn_compare(args: Vec<CfmlValue>) -> CfmlResult {
    let a = get_str(&args, 0);
    let b = get_str(&args, 1);
    Ok(CfmlValue::Int(match a.cmp(&b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }))
}

fn fn_compare_no_case(args: Vec<CfmlValue>) -> CfmlResult {
    let a = get_str(&args, 0).to_lowercase();
    let b = get_str(&args, 1).to_lowercase();
    Ok(CfmlValue::Int(match a.cmp(&b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }))
}

fn fn_asc(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    Ok(CfmlValue::Int(s.chars().next().map_or(0, |c| c as i64)))
}

fn fn_chr(args: Vec<CfmlValue>) -> CfmlResult {
    let code = get_int(&args, 0) as u32;
    Ok(CfmlValue::string(
        char::from_u32(code).map_or(String::new(), |c| c.to_string()),
    ))
}

fn fn_re_find(args: Vec<CfmlValue>) -> CfmlResult {
    re_find_impl(args, false)
}

fn fn_re_find_no_case(args: Vec<CfmlValue>) -> CfmlResult {
    re_find_impl(args, true)
}

fn re_find_impl(args: Vec<CfmlValue>, case_insensitive: bool) -> CfmlResult {
    if args.len() < 2 {
        return Ok(CfmlValue::Int(0));
    }
    let pattern = get_str(&args, 0);
    let string = get_str(&args, 1);
    // The `start` argument is a 1-based CHARACTER offset; convert to a byte
    // offset for the regex engine. Returned match positions/lengths are mapped
    // back to characters below so reFind stays consistent with Mid/Len/Find
    // on non-ASCII input (GitHub #248).
    let start_char = if args.len() >= 3 { (get_int(&args, 2).max(1) as usize).saturating_sub(1) } else { 0 };
    let return_sub = if args.len() >= 4 { args[3].is_true() } else { false };

    let pat = if case_insensitive { format!("(?i){}", pattern) } else { pattern };
    let re = match cached_regex(&pat) {
        Ok(r) => r,
        Err(_) => return Ok(CfmlValue::Int(0)),
    };

    // Start the search at byte offset `start` WITHOUT re-anchoring `^`/`\b` to
    // that offset: `find_at`/`captures_at` advance the search position but keep
    // the anchors relative to the TRUE start of the string (Lucee/Java/PCRE
    // semantics). Slicing `&string[start..]` and matching that — the old
    // approach — made `^` match at the slice start, so e.g.
    // `reFind("^a","xax",2)` wrongly returned 2 instead of 0. Returned match
    // offsets from these APIs are already absolute, so they are NOT re-adjusted.
    // Clamp to len so an out-of-range start can't panic.
    let start = char_index_to_byte(&string, start_char).min(string.len());

    if return_sub {
        if let Some(caps) = re.captures_at_start(&string, start) {
            let mut pos_arr = Vec::new();
            let mut match_arr = Vec::new();
            let mut len_arr = Vec::new();
            for cap in &caps {
                if let Some((m_start, m_str)) = cap {
                    pos_arr.push(CfmlValue::Int((byte_to_char_index(&string, *m_start) + 1) as i64));
                    len_arr.push(CfmlValue::Int(m_str.chars().count() as i64));
                    match_arr.push(CfmlValue::string(m_str.clone()));
                } else {
                    pos_arr.push(CfmlValue::Int(0));
                    match_arr.push(CfmlValue::string(String::new()));
                    len_arr.push(CfmlValue::Int(0));
                }
            }
            let mut result = ValueMap::default();
            result.insert("POS".to_string(), CfmlValue::array(pos_arr));
            result.insert("MATCH".to_string(), CfmlValue::array(match_arr));
            result.insert("LEN".to_string(), CfmlValue::array(len_arr));
            Ok(CfmlValue::strukt(result))
        } else {
            let mut result = ValueMap::default();
            result.insert("POS".to_string(), CfmlValue::array(vec![CfmlValue::Int(0)]));
            result.insert("MATCH".to_string(), CfmlValue::array(vec![CfmlValue::string(String::new())]));
            result.insert("LEN".to_string(), CfmlValue::array(vec![CfmlValue::Int(0)]));
            Ok(CfmlValue::strukt(result))
        }
    } else {
        match re.find_at_start(&string, start) {
            Some(m_start) => Ok(CfmlValue::Int((byte_to_char_index(&string, m_start) + 1) as i64)),
            None => Ok(CfmlValue::Int(0)),
        }
    }
}

fn fn_re_replace(args: Vec<CfmlValue>) -> CfmlResult {
    re_replace_impl(args, false)
}

fn fn_re_replace_no_case(args: Vec<CfmlValue>) -> CfmlResult {
    re_replace_impl(args, true)
}

/// Expand a CFML reReplace replacement template against one match.
///
/// CFML/Lucee/ACF/BoxLang use Perl-style backslash syntax (NOT the regex crate's
/// `$1`): `\0`..`\9` substitute captured groups (`\0` = whole match), and the
/// case modifiers `\u`/`\l` upper/lower the next character while `\U`/`\L` apply
/// to the rest until `\E`. Any other `\X` emits `X` literally, and a bare `$`
/// is literal (the regex crate would otherwise treat `$name` as a group ref).
fn expand_cfml_replacement<F: Fn(usize) -> Option<String>>(template: &str, group: F) -> String {
    let chars: Vec<char> = template.chars().collect();
    let n = chars.len();
    let mut out = String::new();
    let mut ranged_upper: Option<bool> = None; // \U / \L .. \E
    let mut oneshot_upper: Option<bool> = None; // \u / \l (next char only)

    // Append `text`, honoring the active case state. A one-shot \u/\l applies to
    // the first character only; a ranged \U/\L applies until \E.
    fn emit(out: &mut String, text: &str, ranged: Option<bool>, oneshot: &mut Option<bool>) {
        for ch in text.chars() {
            if let Some(up) = oneshot.take() {
                if up { out.extend(ch.to_uppercase()); } else { out.extend(ch.to_lowercase()); }
            } else if let Some(up) = ranged {
                if up { out.extend(ch.to_uppercase()); } else { out.extend(ch.to_lowercase()); }
            } else {
                out.push(ch);
            }
        }
    }

    let mut i = 0;
    while i < n {
        let c = chars[i];
        if c == '\\' && i + 1 < n {
            let next = chars[i + 1];
            match next {
                '0'..='9' => {
                    let g = next as usize - '0' as usize;
                    let text = group(g).unwrap_or_default();
                    emit(&mut out, &text, ranged_upper, &mut oneshot_upper);
                    i += 2;
                }
                'u' => { oneshot_upper = Some(true); i += 2; }
                'l' => { oneshot_upper = Some(false); i += 2; }
                'U' => { ranged_upper = Some(true); i += 2; }
                'L' => { ranged_upper = Some(false); i += 2; }
                'E' => { ranged_upper = None; i += 2; }
                // A backslash before ANY other char is a LITERAL backslash, and
                // we RE-SCAN the following char (advance by 1, not 2) — Lucee
                // parses the replacement one char at a time. So `\\1` is a literal
                // `\` followed by the backref `\1` (→ `\<group1>`), NOT the verbatim
                // `\\1`; `\\`→`\\`, `\n`→`\n`, `\d`→`\d` are unchanged (advancing 1
                // vs 2 yields the same output unless the trailing char is a
                // backref/modifier digit-or-letter). Verified against Lucee.
                _ => {
                    emit(&mut out, "\\", ranged_upper, &mut oneshot_upper);
                    i += 1;
                }
            }
        } else {
            let mut buf = [0u8; 4];
            emit(&mut out, c.encode_utf8(&mut buf), ranged_upper, &mut oneshot_upper);
            i += 1;
        }
    }
    out
}

fn re_replace_impl(args: Vec<CfmlValue>, case_insensitive: bool) -> CfmlResult {
    if args.len() < 3 {
        return Ok(CfmlValue::string(get_str(&args, 0)));
    }
    let string = get_str(&args, 0);
    let pattern = get_str(&args, 1);
    let replacement = get_str(&args, 2);
    let scope = get_str(&args, 3).to_lowercase();

    let pat = if case_insensitive { format!("(?i){}", pattern) } else { pattern };
    let re = match cached_regex(&pat) {
        Ok(r) => r,
        Err(_) => return Ok(CfmlValue::string(string)),
    };

    // Use a custom replacer so CFML's `\N` backreferences and `\u`/`\l`/`\U`/`\L`
    // case modifiers are honored (and `$` stays literal).
    Ok(CfmlValue::string(re.replace_cfml(&string, &replacement, scope == "all")))
}

fn fn_re_match(args: Vec<CfmlValue>) -> CfmlResult {
    re_match_impl(args, false)
}

fn fn_re_match_no_case(args: Vec<CfmlValue>) -> CfmlResult {
    re_match_impl(args, true)
}

fn re_match_impl(args: Vec<CfmlValue>, case_insensitive: bool) -> CfmlResult {
    if args.len() < 2 {
        return Ok(CfmlValue::array(Vec::new()));
    }
    // reMatch(regex, string) - regex is first arg
    let pattern = get_str(&args, 0);
    let string = get_str(&args, 1);

    let pat = if case_insensitive { format!("(?i){}", pattern) } else { pattern };
    let re = match cached_regex(&pat) {
        Ok(r) => r,
        Err(_) => return Ok(CfmlValue::array(Vec::new())),
    };

    let matches: Vec<CfmlValue> = re
        .find_all(&string)
        .into_iter()
        .map(CfmlValue::string)
        .collect();
    Ok(CfmlValue::array(matches))
}

fn fn_wrap(args: Vec<CfmlValue>) -> CfmlResult {
    let string = get_str(&args, 0);
    let limit = get_int(&args, 1).max(1) as usize;
    let strip = args.get(2).map(|v| v.is_true()).unwrap_or(false);
    let input = if strip { string.replace('\n', " ").replace('\r', " ") } else { string };
    let mut result = String::new();
    let mut col = 0;
    for word in input.split_whitespace() {
        if col + word.len() > limit && col > 0 {
            result.push('\n');
            col = 0;
        }
        if col > 0 {
            result.push(' ');
            col += 1;
        }
        result.push_str(word);
        col += word.len();
    }
    Ok(CfmlValue::string(result))
}

fn fn_strip_cr(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::string(get_str(&args, 0).replace('\r', "")))
}

fn fn_to_base64(args: Vec<CfmlValue>) -> CfmlResult {
    // Simple base64 encoding. A Binary value must be encoded from its RAW bytes,
    // not its display string — `get_str` renders Binary as the literal "<Binary>",
    // so `toBase64(FileReadBinary(x))` was producing base64("<Binary>") for EVERY
    // binary input (identical output regardless of content). That silently broke
    // any binary round-trip and made two different files compare equal — Wheels
    // crudSpec's `hasChanged` binary test (SQLite adapter base64-encodes blobs)
    // saw no change between a PNG and a TXT because both encoded to the same
    // "<Binary>" string.
    let input_string;
    let bytes: &[u8] = match args.first() {
        Some(CfmlValue::Binary(b)) => b.as_slice(),
        _ => {
            input_string = get_str(&args, 0);
            input_string.as_bytes()
        }
    };
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(alphabet[((n >> 18) & 63) as usize] as char);
        result.push(alphabet[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 { result.push(alphabet[((n >> 6) & 63) as usize] as char); } else { result.push('='); }
        if chunk.len() > 2 { result.push(alphabet[(n & 63) as usize] as char); } else { result.push('='); }
    }
    Ok(CfmlValue::string(result))
}

fn fn_to_binary(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bytes = Vec::new();
    let chars: Vec<u8> = s.bytes().filter(|&b| b != b'\n' && b != b'\r' && b != b' ').collect();
    let mut i = 0;
    while i < chars.len() {
        if i + 1 >= chars.len() { break; }
        let b0 = table.iter().position(|&c| c == chars[i]).unwrap_or(0) as u32;
        let b1 = table.iter().position(|&c| c == chars[i + 1]).unwrap_or(0) as u32;
        let b2 = if i + 2 < chars.len() && chars[i + 2] != b'=' { table.iter().position(|&c| c == chars[i + 2]).unwrap_or(0) as u32 } else { 0 };
        let b3 = if i + 3 < chars.len() && chars[i + 3] != b'=' { table.iter().position(|&c| c == chars[i + 3]).unwrap_or(0) as u32 } else { 0 };
        let triple = (b0 << 18) | (b1 << 12) | (b2 << 6) | b3;
        bytes.push(((triple >> 16) & 0xFF) as u8);
        if i + 2 < chars.len() && chars[i + 2] != b'=' { bytes.push(((triple >> 8) & 0xFF) as u8); }
        if i + 3 < chars.len() && chars[i + 3] != b'=' { bytes.push((triple & 0xFF) as u8); }
        i += 4;
    }
    Ok(CfmlValue::Binary(bytes))
}

/// Magic header prefixing an `objectSave()` blob. Lets `objectLoad()` recognise
/// its own output and produce a clear error (rather than a cryptic JSON parse
/// failure) if handed something that isn't RustCFML-serialized.
const OBJECT_SAVE_MAGIC: &[u8] = b"RCFMLOBJ\x01";

/// `objectSave(value)` — serialize any CFML value to a binary blob.
///
/// ACF/Lucee implement this via Java object serialization; RustCFML has no JVM,
/// so we use our own self-describing format: the `OBJECT_SAVE_MAGIC` header
/// followed by the value serialized as JSON via `CfmlValue`'s serde impl (which
/// tags Binary/Query with `_cftype` markers so `objectLoad` can reconstruct
/// them). The blob only needs to round-trip with `objectLoad` on the same
/// engine, which is exactly how ColdBox's cache DiskStore marshaller uses it.
///
/// Limitation: Components/Closures/Functions serialize to null (they cannot be
/// reconstituted without their defining program); documented in known-issues.
fn fn_object_save(args: Vec<CfmlValue>) -> CfmlResult {
    let value = args.first().unwrap_or(&CfmlValue::Null);
    let body = serde_json::to_vec(value)
        .map_err(|e| CfmlError::runtime(format!("objectSave: failed to serialize value: {e}")))?;
    let mut out = Vec::with_capacity(OBJECT_SAVE_MAGIC.len() + body.len());
    out.extend_from_slice(OBJECT_SAVE_MAGIC);
    out.extend_from_slice(&body);
    Ok(CfmlValue::Binary(out))
}

/// `objectLoad(binary)` — inflate a blob produced by `objectSave()`.
///
/// Accepts a Binary value (the normal case; ColdBox calls `toBinary()` on a
/// base64 string first) or a String (treated as raw UTF-8 bytes) for leniency.
fn fn_object_load(args: Vec<CfmlValue>) -> CfmlResult {
    let bytes: Vec<u8> = match args.first() {
        Some(CfmlValue::Binary(b)) => b.clone(),
        Some(CfmlValue::String(s)) => s.as_bytes().to_vec(),
        Some(other) => other.as_string().into_bytes(),
        None => {
            return Err(CfmlError::runtime(
                "objectLoad: requires a binary argument".to_string(),
            ))
        }
    };
    let body = if bytes.starts_with(OBJECT_SAVE_MAGIC) {
        &bytes[OBJECT_SAVE_MAGIC.len()..]
    } else {
        // Not our header — could be a JVM-serialized blob from another engine.
        return Err(CfmlError::runtime(
            "objectLoad: input was not produced by RustCFML's objectSave \
             (JVM object serialization is not supported)"
                .to_string(),
        ));
    };
    serde_json::from_slice::<CfmlValue>(body)
        .map_err(|e| CfmlError::runtime(format!("objectLoad: failed to deserialize value: {e}")))
}

fn fn_binary_encode(args: Vec<CfmlValue>) -> CfmlResult {
    let bytes = match args.first() {
        Some(CfmlValue::Binary(b)) => b.clone(),
        Some(other) => other.as_string().into_bytes(),
        None => Vec::new(),
    };
    let encoding = get_str(&args, 1).to_lowercase();
    match encoding.as_str() {
        "hex" => {
            let hex: String = bytes.iter().map(|b| format!("{:02X}", b)).collect();
            Ok(CfmlValue::string(hex))
        }
        "base64" => {
            let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut result = String::new();
            for chunk in bytes.chunks(3) {
                let b0 = chunk[0] as u32;
                let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
                let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
                let n = (b0 << 16) | (b1 << 8) | b2;
                result.push(alphabet[((n >> 18) & 63) as usize] as char);
                result.push(alphabet[((n >> 12) & 63) as usize] as char);
                if chunk.len() > 1 { result.push(alphabet[((n >> 6) & 63) as usize] as char); } else { result.push('='); }
                if chunk.len() > 2 { result.push(alphabet[(n & 63) as usize] as char); } else { result.push('='); }
            }
            Ok(CfmlValue::string(result))
        }
        _ => Err(CfmlError::runtime(format!("Unsupported encoding: {}", encoding))),
    }
}

fn fn_binary_decode(args: Vec<CfmlValue>) -> CfmlResult {
    let input = get_str(&args, 0);
    let encoding = get_str(&args, 1).to_lowercase();
    match encoding.as_str() {
        "hex" => {
            let hex = input.trim();
            let mut bytes = Vec::new();
            let chars: Vec<char> = hex.chars().collect();
            let mut i = 0;
            while i + 1 < chars.len() {
                let high = chars[i].to_digit(16).unwrap_or(0) as u8;
                let low = chars[i + 1].to_digit(16).unwrap_or(0) as u8;
                bytes.push((high << 4) | low);
                i += 2;
            }
            Ok(CfmlValue::Binary(bytes))
        }
        "base64" => {
            let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut bytes = Vec::new();
            let chars: Vec<u8> = input.bytes().filter(|&b| b != b'\n' && b != b'\r' && b != b' ').collect();
            let mut i = 0;
            while i < chars.len() {
                if i + 1 >= chars.len() { break; }
                let b0 = table.iter().position(|&c| c == chars[i]).unwrap_or(0) as u32;
                let b1 = table.iter().position(|&c| c == chars[i + 1]).unwrap_or(0) as u32;
                let b2 = if i + 2 < chars.len() && chars[i + 2] != b'=' { table.iter().position(|&c| c == chars[i + 2]).unwrap_or(0) as u32 } else { 0 };
                let b3 = if i + 3 < chars.len() && chars[i + 3] != b'=' { table.iter().position(|&c| c == chars[i + 3]).unwrap_or(0) as u32 } else { 0 };
                let triple = (b0 << 18) | (b1 << 12) | (b2 << 6) | b3;
                bytes.push(((triple >> 16) & 0xFF) as u8);
                if i + 2 < chars.len() && chars[i + 2] != b'=' { bytes.push(((triple >> 8) & 0xFF) as u8); }
                if i + 3 < chars.len() && chars[i + 3] != b'=' { bytes.push((triple & 0xFF) as u8); }
                i += 4;
            }
            Ok(CfmlValue::Binary(bytes))
        }
        "utf-8" | "us-ascii" => {
            // Convert string directly to bytes
            Ok(CfmlValue::Binary(input.as_bytes().to_vec()))
        }
        _ => Err(CfmlError::runtime(format!("Unsupported encoding: {}", encoding))),
    }
}

// Shared percent-encoder. `space_as_plus` selects between the two CFML
// reference behaviours (GH #270 / GH #283):
//   * urlEncodedFormat / urlEncode → space is `%20` (path-style; matches
//     Lucee/ACF `URLEncodedFormat`).
//   * encodeForURL → space is `+` (ESAPI routes through `java.net.URLEncoder`,
//     i.e. application/x-www-form-urlencoded semantics; every JVM engine —
//     Lucee 5/6/7, Adobe CF, BoxLang — encodes a space as `+`).
fn url_encode_impl(s: &str, space_as_plus: bool) -> String {
    let mut result = String::new();
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '*' => result.push(c),
            ' ' if space_as_plus => result.push('+'),
            _ => {
                for b in c.to_string().as_bytes() {
                    result.push_str(&format!("%{:02X}", b));
                }
            }
        }
    }
    result
}

fn fn_url_encode(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::string(url_encode_impl(&get_str(&args, 0), false)))
}

fn fn_url_decode(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let mut result = String::new();
    let mut bytes = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '%' => {
                let hex: String = chars.by_ref().take(2).collect();
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    bytes.push(byte);
                }
                if chars.peek() != Some(&'%') {
                    if let Ok(decoded) = String::from_utf8(bytes.clone()) {
                        result.push_str(&decoded);
                    } else {
                        for b in &bytes { result.push(*b as char); }
                    }
                    bytes.clear();
                }
            }
            '+' => {
                if !bytes.is_empty() {
                    if let Ok(decoded) = String::from_utf8(bytes.clone()) {
                        result.push_str(&decoded);
                    }
                    bytes.clear();
                }
                result.push(' ');
            }
            _ => {
                if !bytes.is_empty() {
                    if let Ok(decoded) = String::from_utf8(bytes.clone()) {
                        result.push_str(&decoded);
                    }
                    bytes.clear();
                }
                result.push(c);
            }
        }
    }
    if !bytes.is_empty() {
        if let Ok(decoded) = String::from_utf8(bytes.clone()) {
            result.push_str(&decoded);
        }
    }
    Ok(CfmlValue::string(result))
}

fn fn_html_edit_format(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    Ok(CfmlValue::string(
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;"),
    ))
}

fn fn_html_code_format(args: Vec<CfmlValue>) -> CfmlResult {
    let inner = fn_html_edit_format(args)?;
    Ok(CfmlValue::string(format!("<pre>{}</pre>", inner.as_string())))
}

fn fn_encode_for_html(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let mut result = String::new();
    // OWASP/ESAPI HTMLEntityCodec for an HTML *content* context. The immune set
    // is ESAPI's IMMUNE_HTML = `, . - _` PLUS space — that single extra immune
    // char (space) is the only thing distinguishing it from the attribute codec
    // (IMMUNE_HTMLATTR). Every other char below U+0100 is encoded: a named
    // entity where one exists, else a lowercase hex numeric entity. Codepoints
    // >= U+0100 pass through. Adobe CF and BoxLang match this; Lucee 7 is the
    // outlier (encodes almost nothing). The old 6-char replace() chain left `(`,
    // `)`, `=`, `[`, `]`, etc. raw — wrong by OWASP/Adobe/BoxLang, and it broke
    // Wheels form helpers asserting e.g. `alert&#x28;&quot;XSS&quot;&#x29;`.
    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' => result.push(c),
            ',' | '.' | '-' | '_' | ' ' => result.push(c),
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            c if (c as u32) >= 0x100 => result.push(c),
            c => result.push_str(&format!("&#x{:x};", c as u32)),
        }
    }
    Ok(CfmlValue::string(result))
}

fn fn_ljustify(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let length = get_int(&args, 1).max(0) as usize;
    Ok(CfmlValue::string(format!("{:<width$}", s, width = length)))
}

fn fn_rjustify(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let length = get_int(&args, 1).max(0) as usize;
    Ok(CfmlValue::string(format!("{:>width$}", s, width = length)))
}

fn add_thousands_separator(s: &str) -> String {
    let bytes = s.as_bytes();
    let len = bytes.len();
    if len <= 3 { return s.to_string(); }
    let mut result = String::new();
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push(',');
        }
        result.push(b as char);
    }
    result
}

fn fn_number_format(args: Vec<CfmlValue>) -> CfmlResult {
    let n = get_float(&args, 0);
    let mask = get_str(&args, 1);
    if mask.is_empty() {
        let rounded = n.round() as i64;
        let s = rounded.to_string();
        let negative = rounded < 0;
        let digits = if negative { &s[1..] } else { &s };
        let formatted = add_thousands_separator(digits);
        if negative {
            return Ok(CfmlValue::string(format!("-{}", formatted)));
        }
        return Ok(CfmlValue::string(formatted));
    }

    let has_dollar = mask.contains('$');
    let has_parens = mask.contains('(') && mask.contains(')');
    let has_plus = mask.contains('+');
    let has_comma = mask.contains(',');

    let decimals = if let Some(dot_pos) = mask.find('.') {
        mask[dot_pos + 1..].chars().filter(|c| *c == '9' || *c == '0' || *c == '_').count()
    } else {
        0
    };

    let formatted_num = format!("{:.prec$}", n.abs(), prec = decimals);
    let parts: Vec<&str> = formatted_num.split('.').collect();
    let int_part = parts[0];
    let dec_part = if parts.len() > 1 { parts[1] } else { "" };

    // Integer-part mask padding (Lucee/ACF): each digit position in the mask
    // that the number doesn't fill becomes '0' for a `0` placeholder or a space
    // for a `9`/`_` placeholder. NumberFormat(1,"09") -> "01",
    // NumberFormat(5,"99") -> " 5". Only widens when the number is shorter than
    // the mask, so masks the number already fills are untouched.
    let int_mask: String = mask
        .split('.')
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| matches!(c, '0' | '9' | '_'))
        .collect();
    let padded_int = if int_part.len() < int_mask.len() {
        let pad = int_mask.len() - int_part.len();
        let mut s = String::with_capacity(int_mask.len());
        for i in 0..pad {
            s.push(if int_mask.as_bytes()[i] == b'0' { '0' } else { ' ' });
        }
        s.push_str(int_part);
        s
    } else {
        int_part.to_string()
    };

    let int_formatted = if has_comma {
        add_thousands_separator(int_part)
    } else {
        padded_int
    };

    let mut result = if decimals > 0 {
        format!("{}.{}", int_formatted, dec_part)
    } else {
        int_formatted
    };

    if n < 0.0 {
        if has_parens {
            result = format!("({})", result);
        } else {
            result = format!("-{}", result);
        }
    } else if has_plus {
        result = format!("+{}", result);
    }

    if has_dollar {
        if result.starts_with('-') || result.starts_with('(') {
            let sign = result.chars().next().unwrap();
            result = format!("{}${}", sign, &result[1..]);
        } else {
            result = format!("${}", result);
        }
    }

    Ok(CfmlValue::string(result))
}

fn fn_decimal_format(args: Vec<CfmlValue>) -> CfmlResult {
    let n = get_float(&args, 0);
    let formatted = format!("{:.2}", n.abs());
    let parts: Vec<&str> = formatted.split('.').collect();
    let int_with_commas = add_thousands_separator(parts[0]);
    let result = format!("{}.{}", int_with_commas, parts.get(1).unwrap_or(&"00"));
    if n < 0.0 {
        Ok(CfmlValue::string(format!("-{}", result)))
    } else {
        Ok(CfmlValue::string(result))
    }
}

fn fn_format_base_n(args: Vec<CfmlValue>) -> CfmlResult {
    let n = get_int(&args, 0) as i32;
    let radix = get_int(&args, 1) as u32;
    if radix < 2 || radix > 36 {
        return Err(CfmlError::runtime("formatBaseN: radix must be between 2 and 36".to_string()));
    }
    let is_negative = n < 0;
    let abs_n = if is_negative { (n as i64).unsigned_abs() } else { n as u64 };
    if abs_n == 0 { return Ok(CfmlValue::string("0".to_string())); }
    let digits = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut result = String::new();
    let mut val = abs_n;
    while val > 0 {
        let d = (val % radix as u64) as usize;
        result.push(digits.as_bytes()[d] as char);
        val /= radix as u64;
    }
    if is_negative { result.push('-'); }
    Ok(CfmlValue::string(result.chars().rev().collect::<String>()))
}

fn fn_input_base_n(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let radix = get_int(&args, 1) as u32;
    Ok(CfmlValue::Int(
        i64::from_str_radix(&s, radix).unwrap_or(0),
    ))
}

fn fn_replace_list(args: Vec<CfmlValue>) -> CfmlResult {
    let mut string = get_str(&args, 0);
    let list1 = get_str(&args, 1);
    let list2 = get_str(&args, 2);
    let delimiter = get_delimiter(&args, 3);
    let items1: Vec<&str> = list1.split(|c: char| delimiter.contains(c)).filter(|s| !s.is_empty()).collect();
    let items2: Vec<&str> = list2.split(|c: char| delimiter.contains(c)).filter(|s| !s.is_empty()).collect();
    for (i, find) in items1.iter().enumerate() {
        let replace_with = items2.get(i).unwrap_or(&"");
        string = string.replace(find, replace_with);
    }
    Ok(CfmlValue::string(string))
}

fn fn_replace_list_no_case(args: Vec<CfmlValue>) -> CfmlResult {
    let mut string = get_str(&args, 0);
    let list1 = get_str(&args, 1);
    let list2 = get_str(&args, 2);
    let delimiter = get_delimiter(&args, 3);
    let items1: Vec<&str> = list1.split(|c: char| delimiter.contains(c)).filter(|s| !s.is_empty()).collect();
    let items2: Vec<&str> = list2.split(|c: char| delimiter.contains(c)).filter(|s| !s.is_empty()).collect();
    for (i, find) in items1.iter().enumerate() {
        let replace_with = items2.get(i).unwrap_or(&"");
        let lower = string.to_lowercase();
        let find_lower = find.to_lowercase();
        let mut result = String::new();
        let mut start = 0;
        while let Some(pos) = lower[start..].find(&find_lower) {
            result.push_str(&string[start..start + pos]);
            result.push_str(replace_with);
            start += pos + find.len();
        }
        result.push_str(&string[start..]);
        string = result;
    }
    Ok(CfmlValue::string(string))
}

fn fn_xml_format(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    Ok(CfmlValue::string(
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;"),
    ))
}

fn fn_paragraph_format(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let result = s.replace("\r\n", "\n")
        .split('\n')
        .map(|line| if line.trim().is_empty() { "<p>".to_string() } else { format!("{}<br>", line) })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(CfmlValue::string(result))
}

fn fn_cjustify(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let length = get_int(&args, 1) as usize;
    if s.len() >= length {
        return Ok(CfmlValue::string(s));
    }
    let padding = length - s.len();
    let left_pad = padding / 2;
    let right_pad = padding - left_pad;
    Ok(CfmlValue::string(format!("{}{}{}", " ".repeat(left_pad), s, " ".repeat(right_pad))))
}

// ===============================================
// ARRAY FUNCTIONS
// ===============================================

fn fn_array_new(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::array(Vec::new()))
}

fn fn_array_len(args: Vec<CfmlValue>) -> CfmlResult {
    match args.first() {
        Some(CfmlValue::Array(a)) => Ok(CfmlValue::Int(a.len() as i64)),
        // Lucee@7 parity: arrayLen(q.col) errors — column proxies are NOT arrays.
        Some(v @ CfmlValue::QueryColumn(..)) => Err(CfmlError::runtime(format!(
            "Can't cast String [{}] to a value of type [Array]",
            v.as_string()
        ))),
        // Arguments scope is a struct but arrayLen should return count of positional entries
        Some(CfmlValue::Struct(s)) => {
            // The arguments scope is a hybrid array/struct on Lucee/ACF: for both
            // positional AND named calls, `arrayLen(arguments)` counts the bound
            // args. Marker keys (`__arguments_scope`, `__arguments_params`) are
            // excluded so they don't inflate the count.
            if s.contains_key("__arguments_scope") {
                let count = s
                    .keys()
                    .into_iter()
                    .filter(|k| k.as_str() != "__arguments_scope" && k.as_str() != "__arguments_params")
                    .count();
                return Ok(CfmlValue::Int(count as i64));
            }
            // Plain struct: count entries with numeric keys (1-based positional args).
            let count = s.keys().into_iter().filter(|k| k.parse::<usize>().is_ok()).count();
            Ok(CfmlValue::Int(count as i64))
        }
        _ => Ok(CfmlValue::Int(0)),
    }
}

// Reference semantics: the genuinely-mutating array builtins update args[0]'s
// shared backing in place (so aliases — `b = a` — observe the change) and
// return that same handle. Pure builtins below snapshot and build a new array.
fn fn_array_append(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        // Optional 3rd arg `merge`: when true and the value is an array, its
        // elements are appended individually instead of as a single nested
        // element (Lucee/ACF semantics).
        let merge = args.get(2).map(|v| v.is_true()).unwrap_or(false);
        if let CfmlValue::Array(a) = &args[0] {
            match (&args[1], merge) {
                (CfmlValue::Array(elems), true) => {
                    let elems = elems.snapshot();
                    a.with_write(|v| v.extend(elems));
                }
                _ => a.push(args[1].clone()),
            }
            return Ok(CfmlValue::Array(a.clone()));
        }
        // Non-array first arg: build a fresh array (legacy coercion).
        let mut v = Vec::new();
        match (&args[1], merge) {
            (CfmlValue::Array(elems), true) => v.extend(elems.snapshot()),
            _ => v.push(args[1].clone()),
        }
        Ok(CfmlValue::array(v))
    } else {
        Ok(args.into_iter().next().unwrap_or(CfmlValue::array(Vec::new())))
    }
}

fn fn_array_prepend(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let CfmlValue::Array(a) = &args[0] {
            a.with_write(|v| v.insert(0, args[1].clone()));
            return Ok(CfmlValue::Array(a.clone()));
        }
        Ok(CfmlValue::array(vec![args[1].clone()]))
    } else {
        Ok(args.into_iter().next().unwrap_or(CfmlValue::array(Vec::new())))
    }
}

fn fn_array_delete_at(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let CfmlValue::Array(a) = &args[0] {
            let idx = (get_int(&args, 1) as usize).saturating_sub(1);
            a.with_write(|v| {
                if idx < v.len() {
                    v.remove(idx);
                }
            });
            Ok(CfmlValue::Array(a.clone()))
        } else {
            Ok(CfmlValue::Bool(false))
        }
    } else {
        Ok(CfmlValue::Bool(false))
    }
}

fn fn_array_insert_at(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 3 {
        if let CfmlValue::Array(a) = &args[0] {
            let idx = (get_int(&args, 1) as usize).saturating_sub(1);
            a.with_write(|v| {
                if idx <= v.len() {
                    v.insert(idx, args[2].clone());
                }
            });
            Ok(CfmlValue::Array(a.clone()))
        } else {
            Ok(CfmlValue::Bool(false))
        }
    } else {
        Ok(CfmlValue::Bool(false))
    }
}

/// Deep, order-insensitive value equality for the array find/contains family.
/// Lucee's `arrayFind`/`arrayContains` match COMPLEX needles (structs/arrays)
/// by deep equality — struct keys compare case-insensitively and regardless of
/// insertion order, arrays compare element-wise. Scalars fall back to the same
/// string comparison the callers used before (so numeric `20` still matches the
/// string `"20"`); `nocase` lowercases scalar comparisons.
fn cfml_deep_equal(a: &CfmlValue, b: &CfmlValue, nocase: bool) -> bool {
    match (a, b) {
        (CfmlValue::Struct(sa), CfmlValue::Struct(sb)) => {
            // Identity short-circuit: two references to the SAME backing handle
            // are equal without walking their contents. This is both correct (a
            // value equals itself) and essential for cycle safety — a
            // self-referential struct graph (e.g. a Wheels model with a circular
            // association: `profile.author = author; author.profile = profile`)
            // would otherwise recurse forever here. Lucee compares CFC instances
            // by reference, so `arrayContains(visited, obj)` detects an
            // already-seen object by identity, which is exactly how Wheels'
            // `allErrors(includeAssociations=true)` breaks the cycle. (Wheels
            // model.errorsSpec "handles circular reference" stack-overflowed the
            // whole TestBox suite without this.)
            if sa.backing_ptr() == sb.backing_ptr() {
                return true;
            }
            if sa.len() != sb.len() {
                return false;
            }
            for (k, va) in sa.iter() {
                match sb.get_ci(&k) {
                    Some(vb) => {
                        if !cfml_deep_equal(&va, &vb, nocase) {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
            true
        }
        (CfmlValue::Array(aa), CfmlValue::Array(ab)) => {
            // Same identity short-circuit as Struct (above) — same backing
            // handle is equal without recursing, so a cyclic array graph is safe.
            if aa.backing_ptr() == ab.backing_ptr() {
                return true;
            }
            let sa = aa.snapshot();
            let sb = ab.snapshot();
            sa.len() == sb.len()
                && sa
                    .iter()
                    .zip(sb.iter())
                    .all(|(x, y)| cfml_deep_equal(x, y, nocase))
        }
        // A complex value never equals a scalar (or a struct-vs-array mismatch).
        (CfmlValue::Struct(_), _) | (_, CfmlValue::Struct(_)) => false,
        (CfmlValue::Array(_), _) | (_, CfmlValue::Array(_)) => false,
        // Flyweight component instances compare by REFERENCE (Lucee/ACF parity —
        // same as the marker backing-ptr identity above). Without this an Instance
        // fell to the `_ => as_string()` arm below, where EVERY component
        // stringifies to "<Component>" → any two components compared EQUAL, so
        // `arrayContains(seen, obj)` matched the first component of any class
        // (breaking Wheels' circular-association cycle detection).
        _ if a.as_component().is_some_and(|c| c.is_instance_backed())
            || b.as_component().is_some_and(|c| c.is_instance_backed()) =>
        {
            cfml_common::component::same_component_instance(a, b)
        }
        _ => {
            if nocase {
                a.as_string().eq_ignore_ascii_case(&b.as_string())
            } else {
                a.as_string() == b.as_string()
            }
        }
    }
}

fn fn_array_contains(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let Some(arr) = args[0].as_array() {
            return Ok(CfmlValue::Bool(
                arr.iter().any(|v| cfml_deep_equal(&v, &args[1], false)),
            ));
        }
    }
    Ok(CfmlValue::Bool(false))
}

fn fn_array_contains_no_case(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let Some(arr) = args[0].as_array() {
            return Ok(CfmlValue::Bool(
                arr.iter().any(|v| cfml_deep_equal(&v, &args[1], true)),
            ));
        }
    }
    Ok(CfmlValue::Bool(false))
}

fn fn_array_find(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let CfmlValue::Array(arr) = &args[0] {
            for (i, v) in arr.iter().enumerate() {
                if cfml_deep_equal(&v, &args[1], false) {
                    return Ok(CfmlValue::Int((i + 1) as i64));
                }
            }
        }
    }
    Ok(CfmlValue::Int(0))
}

fn fn_array_find_no_case(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let CfmlValue::Array(arr) = &args[0] {
            for (i, v) in arr.iter().enumerate() {
                if cfml_deep_equal(&v, &args[1], true) {
                    return Ok(CfmlValue::Int((i + 1) as i64));
                }
            }
        }
    }
    Ok(CfmlValue::Int(0))
}

fn fn_array_sort(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(CfmlValue::Array(arr)) = args.first() {
        let sort_type = if args.len() > 1 { get_str(&args, 1).to_lowercase() } else { "text".to_string() };
        let sort_order = if args.len() > 2 { get_str(&args, 2).to_lowercase() } else { "asc".to_string() };
        // In-place sort on the shared handle (Lucee sorts the original array).
        arr.with_write(|v| {
            match sort_type.as_str() {
                "numeric" => {
                    v.sort_by(|a, b| {
                        let fa = a.as_string().parse::<f64>().unwrap_or(0.0);
                        let fb = b.as_string().parse::<f64>().unwrap_or(0.0);
                        fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
                "textnocase" => {
                    v.sort_by(|a, b| a.as_string().to_lowercase().cmp(&b.as_string().to_lowercase()));
                }
                _ => {
                    v.sort_by(|a, b| a.as_string().cmp(&b.as_string()));
                }
            }
            if sort_order == "desc" {
                v.reverse();
            }
        });
        Ok(CfmlValue::Array(arr.clone()))
    } else {
        Ok(CfmlValue::array(Vec::new()))
    }
}

fn fn_array_reverse(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(CfmlValue::Array(arr)) = args.first() {
        // In-place reverse on the shared handle.
        arr.with_write(|v| v.reverse());
        Ok(CfmlValue::Array(arr.clone()))
    } else {
        Ok(CfmlValue::array(Vec::new()))
    }
}

fn fn_array_slice(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(CfmlValue::Array(arr)) = args.first() {
        // Pure: produces a new array.
        let snap = arr.snapshot();
        let offset = get_int(&args, 1);
        let length = if args.len() >= 3 { Some(get_int(&args, 2) as usize) } else { None };

        let start = if offset >= 0 {
            (offset as usize).saturating_sub(1) // 1-based to 0-based
        } else {
            // Negative: count from end
            let from_end = (-offset) as usize;
            if from_end > snap.len() { 0 } else { snap.len() - from_end }
        };

        if start >= snap.len() {
            return Ok(CfmlValue::array(Vec::new()));
        }

        let end = match length {
            Some(len) => (start + len).min(snap.len()),
            None => snap.len(),
        };

        Ok(CfmlValue::array(snap[start..end].to_vec()))
    } else {
        Ok(CfmlValue::array(Vec::new()))
    }
}

fn fn_array_to_list(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(CfmlValue::Array(arr)) = args.first() {
        let delimiter = get_delimiter(&args, 1);
        let items: Vec<String> = arr.iter().map(|v| v.as_string()).collect();
        Ok(CfmlValue::string(items.join(&delimiter)))
    } else {
        Ok(CfmlValue::string(String::new()))
    }
}

fn fn_array_merge(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let (CfmlValue::Array(a), CfmlValue::Array(b)) = (&args[0], &args[1]) {
            // Pure: produces a new array (does not mutate either operand).
            let leave_index = args.get(2).map(|v| v.is_true()).unwrap_or(false);
            let mut result = a.snapshot();
            if leave_index {
                for (i, item) in b.iter().enumerate() {
                    if i < result.len() {
                        result[i] = item;
                    } else {
                        result.push(item);
                    }
                }
            } else {
                result.extend(b.iter());
            }
            return Ok(CfmlValue::array(result));
        }
    }
    Ok(CfmlValue::array(Vec::new()))
}

fn fn_array_clear(args: Vec<CfmlValue>) -> CfmlResult {
    // In-place clear on the shared handle (aliases see the emptied array).
    if let Some(CfmlValue::Array(a)) = args.first() {
        a.with_write(|v| v.clear());
        return Ok(CfmlValue::Array(a.clone()));
    }
    Ok(CfmlValue::array(Vec::new()))
}

fn fn_array_is_defined(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        let idx = get_int(&args, 1) as usize;
        match &args[0] {
            CfmlValue::Array(arr) => {
                return Ok(CfmlValue::Bool(idx >= 1 && idx <= arr.len()));
            }
            // The arguments scope is array-like (see fn_is_array/fn_array_len):
            // arrayIsDefined(arguments, i) tests the i-th bound arg.
            CfmlValue::Struct(s) if s.contains_key("__arguments_scope") => {
                let count = s
                    .keys()
                    .into_iter()
                    .filter(|k| {
                        k.as_str() != "__arguments_scope" && k.as_str() != "__arguments_params"
                    })
                    .count();
                return Ok(CfmlValue::Bool(idx >= 1 && idx <= count));
            }
            _ => {}
        }
    }
    Ok(CfmlValue::Bool(false))
}

fn fn_array_set(args: Vec<CfmlValue>) -> CfmlResult {
    // arraySet(array, start, end, value) — in-place on the shared handle.
    if args.len() >= 4 {
        if let CfmlValue::Array(arr) = &args[0] {
            let start = (get_int(&args, 1) as usize).saturating_sub(1);
            let end = get_int(&args, 2) as usize;
            arr.with_write(|v| {
                while v.len() < end {
                    v.push(CfmlValue::Null);
                }
                for i in start..end.min(v.len()) {
                    v[i] = args[3].clone();
                }
            });
            return Ok(CfmlValue::Array(arr.clone()));
        }
    }
    // Was `Ok(Bool(false))`: a too-short or non-array call mutated nothing and
    // reported no failure, so `a.set(1,"z")` (3 args) silently did nothing. Lucee
    // errors on the same call. (GH #307 no-op audit.)
    Err(CfmlError::runtime(
        "arraySet requires (array, startIndex, endIndex, value)".to_string(),
    ))
}

fn fn_array_swap(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 3 {
        if let CfmlValue::Array(arr) = &args[0] {
            let i = (get_int(&args, 1) as usize).saturating_sub(1);
            let j = (get_int(&args, 2) as usize).saturating_sub(1);
            arr.with_write(|v| {
                if i < v.len() && j < v.len() {
                    v.swap(i, j);
                }
            });
            return Ok(CfmlValue::Array(arr.clone()));
        }
    }
    // Was `Ok(Bool(false))` — a silent no-op on a bad call (GH #307 no-op audit).
    Err(CfmlError::runtime(
        "arraySwap requires (array, index1, index2)".to_string(),
    ))
}

fn fn_array_min(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(CfmlValue::Array(arr)) = args.first() {
        let mut min = f64::INFINITY;
        for v in arr.iter() {
            let n = get_float(&[v.clone()], 0);
            if n < min { min = n; }
        }
        Ok(CfmlValue::Double(if min.is_infinite() { 0.0 } else { min }))
    } else {
        Ok(CfmlValue::Int(0))
    }
}

fn fn_array_max(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(CfmlValue::Array(arr)) = args.first() {
        let mut max = f64::NEG_INFINITY;
        for v in arr.iter() {
            let n = get_float(&[v.clone()], 0);
            if n > max { max = n; }
        }
        Ok(CfmlValue::Double(if max.is_infinite() { 0.0 } else { max }))
    } else {
        Ok(CfmlValue::Int(0))
    }
}

fn fn_array_avg(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(CfmlValue::Array(arr)) = args.first() {
        if arr.is_empty() { return Ok(CfmlValue::Int(0)); }
        let sum: f64 = arr.iter().map(|v| get_float(&[v.clone()], 0)).sum();
        Ok(CfmlValue::Double(sum / arr.len() as f64))
    } else {
        Ok(CfmlValue::Int(0))
    }
}

fn fn_array_sum(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(CfmlValue::Array(arr)) = args.first() {
        let sum: f64 = arr.iter().map(|v| get_float(&[v.clone()], 0)).sum();
        Ok(CfmlValue::Double(sum))
    } else {
        Ok(CfmlValue::Int(0))
    }
}

// Higher-order array functions (stubs - would need closure support in builtins)
fn fn_array_map(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(args.into_iter().next().unwrap_or(CfmlValue::array(Vec::new())))
}
fn fn_array_filter(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(args.into_iter().next().unwrap_or(CfmlValue::array(Vec::new())))
}
fn fn_array_reduce(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Null)
}
fn fn_array_each(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Null)
}

fn fn_is_array(args: Vec<CfmlValue>) -> CfmlResult {
    // Lucee@7 parity: QueryColumn is NOT an array. The `arguments` scope IS an
    // array (it is a hybrid array/struct): isArray(arguments) is true for
    // positional, named, and empty arg lists alike. Gated on the private
    // `__arguments_scope` marker so a plain numeric-keyed struct stays a struct.
    let is = match args.first() {
        Some(CfmlValue::Array(_)) => true,
        Some(CfmlValue::Struct(s)) => s.contains_key("__arguments_scope"),
        _ => false,
    };
    Ok(CfmlValue::Bool(is))
}

fn fn_array_is_empty(args: Vec<CfmlValue>) -> CfmlResult {
    match args.first() {
        Some(CfmlValue::Array(arr)) => Ok(CfmlValue::Bool(arr.is_empty())),
        _ => Ok(CfmlValue::Bool(true)),
    }
}

fn fn_array_delete(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let CfmlValue::Array(arr) = &args[0] {
            let value_str = args[1].as_string().to_lowercase();
            arr.with_write(|v| {
                if let Some(pos) =
                    v.iter().position(|x| x.as_string().to_lowercase() == value_str)
                {
                    v.remove(pos);
                }
            });
            return Ok(CfmlValue::Array(arr.clone()));
        }
    }
    Ok(CfmlValue::array(Vec::new()))
}

fn fn_array_find_all(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let CfmlValue::Array(arr) = &args[0] {
            let indices: Vec<CfmlValue> = arr.iter().enumerate()
                .filter(|(_, v)| cfml_deep_equal(v, &args[1], false))
                .map(|(i, _)| CfmlValue::Int((i + 1) as i64))
                .collect();
            return Ok(CfmlValue::array(indices));
        }
    }
    Ok(CfmlValue::array(Vec::new()))
}

fn fn_array_find_all_no_case(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let CfmlValue::Array(arr) = &args[0] {
            let indices: Vec<CfmlValue> = arr.iter().enumerate()
                .filter(|(_, v)| cfml_deep_equal(v, &args[1], true))
                .map(|(i, _)| CfmlValue::Int((i + 1) as i64))
                .collect();
            return Ok(CfmlValue::array(indices));
        }
    }
    Ok(CfmlValue::array(Vec::new()))
}

fn fn_array_first(args: Vec<CfmlValue>) -> CfmlResult {
    match args.first() {
        Some(CfmlValue::Array(arr)) => Ok(arr.first().unwrap_or(CfmlValue::Null)),
        _ => Err(CfmlError::runtime("arrayFirst: argument must be an array".to_string())),
    }
}

fn fn_array_last(args: Vec<CfmlValue>) -> CfmlResult {
    match args.first() {
        Some(CfmlValue::Array(arr)) => Ok(arr.last().unwrap_or(CfmlValue::Null)),
        _ => Err(CfmlError::runtime("arrayLast: argument must be an array".to_string())),
    }
}

fn fn_is_empty(args: Vec<CfmlValue>) -> CfmlResult {
    match args.first() {
        Some(CfmlValue::String(s)) => Ok(CfmlValue::Bool(s.is_empty())),
        Some(CfmlValue::Array(arr)) => Ok(CfmlValue::Bool(arr.is_empty())),
        Some(CfmlValue::Struct(s)) => Ok(CfmlValue::Bool(s.is_empty())),
        Some(CfmlValue::Null) => Ok(CfmlValue::Bool(true)),
        _ => Ok(CfmlValue::Bool(false)),
    }
}

// ===============================================
// STRUCT FUNCTIONS
// ===============================================

fn fn_struct_new(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::strukt(ValueMap::default()))
}

/// getTagData( library, tag ) — returns metadata about a CFML tag, notably a
/// `.attributes` struct keyed by the attribute names the tag actually supports.
/// Used for runtime feature-detection (e.g. Preside's `DbInfoService` checks
/// `structKeyExists( getTagData("CF","DBINFO").attributes, "filter" )` to pick
/// the modern dbinfo path). The metadata describes RustCFML's *real* tag
/// capabilities rather than mirroring a particular Lucee version, so the
/// feature-detect resolves correctly on this engine. Unknown libraries/tags
/// return null, matching Lucee for tags it doesn't know.
fn fn_get_tag_data(args: Vec<CfmlValue>) -> CfmlResult {
    let library = args.first().map(|v| v.as_string()).unwrap_or_default();
    let tag = args.get(1).map(|v| v.as_string()).unwrap_or_default();

    // Only the standard CFML tag library ("CF") is described.
    if !library.eq_ignore_ascii_case("cf") {
        return Ok(CfmlValue::Null);
    }

    // (attribute name, type, required) for each supported attribute.
    let attrs: &[(&str, &str, bool)] = match tag.to_lowercase().as_str() {
        // Mirrors the attributes honoured by crate::dbinfo::fn_dbinfo_impl and
        // the cfdbinfo VM intercept. `filter` and the `columns_minimal` type
        // are both supported, so Preside's modern dbinfo path activates.
        "dbinfo" => &[
            ("type", "string", true),
            ("name", "variableName", true),
            ("datasource", "string", false),
            ("table", "string", false),
            ("pattern", "string", false),
            ("filter", "string", false),
            ("procedure", "string", false),
        ],
        _ => return Ok(CfmlValue::Null),
    };

    let mut attributes = ValueMap::default();
    for (name, ty, required) in attrs {
        let mut entry = ValueMap::default();
        entry.insert("name".into(), CfmlValue::string((*name).to_string()));
        entry.insert("type".into(), CfmlValue::string((*ty).to_string()));
        entry.insert("required".into(), CfmlValue::Bool(*required));
        attributes.insert((*name).to_string(), CfmlValue::strukt(entry));
    }

    let mut result = ValueMap::default();
    result.insert("name".into(), CfmlValue::string(tag.to_lowercase()));
    result.insert("attributes".into(), CfmlValue::strukt(attributes));
    Ok(CfmlValue::strukt(result))
}

/// The `arguments` scope carries two private markers (`__arguments_scope` +
/// `__arguments_params`) used for positional-index fallback in `arguments[i]`.
/// Neither must appear in user-visible struct introspection (`structKeyList`,
/// `structCount`, `structKeyExists`, key arrays, for-in). Real numeric keys —
/// produced when overflow positional args have no matching declared param
/// name (e.g. paramless fn called positionally) — DO remain visible: that's
/// how Lucee surfaces them.
fn visible_struct_keys(s: &cfml_common::dynamic::CfmlStruct) -> Vec<String> {
    // Lucee ENUMERATES a null-valued key (StructKeyList/StructKeyArray/StructCount
    // and struct for-in all include it — verified: `{a=1,x=nullValue(),b=2}` →
    // "B,X,A"), even though `structKeyExists` reports it absent ("a NULL value is
    // the same as not existing"). structKeyExists enforces its own null-absence
    // check (fn_struct_key_exists), so listing null keys here does NOT make the two
    // disagree. (Query rows store NULL columns as "" — a real value — so they are
    // unaffected either way.)
    // `all_keys()` unions the shared method table (component flyweight) so a
    // component's public methods still enumerate even though they now live once
    // per class rather than per-instance. Plain structs have no table → == keys().
    let keys: Vec<String> = s.all_keys();
    // A Java-collection shim (e.g. createObject("java","java.util.LinkedHashMap"))
    // is a transparent map facade — its `__java_class`/`__java_shim` markers are
    // engine-internal and must never surface as struct keys (ColdBox's
    // ModuleService iterates `structKeyArray( moduleRegistry )` over exactly such
    // a LinkedHashMap, and a leaked `__java_class` was being treated as a module).
    let is_java_shim = keys.iter().any(|k| k == "__java_shim");
    if is_java_shim {
        return keys.into_iter().filter(|k| !k.starts_with("__")).collect();
    }
    // A CFC instance is materialised as a marker-bearing struct; its engine
    // internals (__name/__variables/__properties/__metadata/__source_file/...)
    // are NOT struct keys in Lucee/ACF — StructKeyList/StructKeyArray expose only
    // public members. Without this filter, toXML/$structToXML descends into
    // `__variables` and recurses without bound (Wheels renderWith(data=model)
    // tripped the depth-256 guard). Mirrors the for-in CFC filter in cfml-vm:
    // drop `__`-prefixed keys + `this`, and keep only public/remote methods.
    let is_component = keys.iter().any(|k| k.eq_ignore_ascii_case("__variables"))
        && keys
            .iter()
            .any(|k| k.eq_ignore_ascii_case("__name") || k.eq_ignore_ascii_case("this"));
    if is_component {
        // Accessor-private property names (values written by the implicit accessor
        // ctor or a generated setX) — Lucee keeps these in the private `variables`
        // scope, so structKeyList/Count/Exists/for-in must not surface them (only
        // getX()/serializeJSON do). See ACCESSOR_PRIVATE_MARKER.
        let accessor_private = match s.get(cfml_common::dynamic::ACCESSOR_PRIVATE_MARKER) {
            Some(CfmlValue::Struct(m)) => Some(m),
            _ => None,
        };
        return keys
            .into_iter()
            .filter(|k| {
                // Hide ONLY the exact engine-reserved bookkeeping keys, not every
                // `__`-prefixed key: `__`/`___` are legal identifiers frameworks use
                // for real public data (FW/1 AOP's `this["___doReverse"]`/`___orig`),
                // which Lucee/ACF surface. `is_reserved_component_key` is the exact
                // set (C.4 blanket-`__`-filter deletion, applied to the marker path).
                if cfml_common::component::is_reserved_component_key(k)
                    || k.eq_ignore_ascii_case("this")
                {
                    return false;
                }
                match s.get(k) {
                    Some(CfmlValue::Function(f)) => matches!(
                        f.access,
                        cfml_common::dynamic::CfmlAccess::Public
                            | cfml_common::dynamic::CfmlAccess::Remote
                    ),
                    _ => !accessor_private.as_ref().is_some_and(|m| m.contains_key_ci(k)),
                }
            })
            .collect();
    }
    // The magic-scope marker (cgi) is engine-internal — never surface it.
    let has_magic = keys
        .iter()
        .any(|k| k == cfml_common::dynamic::EMPTY_DEFAULT_SCOPE_MARKER);
    if !keys.iter().any(|k| k == "__arguments_scope") {
        if has_magic {
            return keys
                .into_iter()
                .filter(|k| k != cfml_common::dynamic::EMPTY_DEFAULT_SCOPE_MARKER)
                .collect();
        }
        return keys;
    }
    keys.into_iter()
        .filter(|k| {
            k != "__arguments_scope"
                && k != "__arguments_params"
                && k != cfml_common::dynamic::EMPTY_DEFAULT_SCOPE_MARKER
        })
        .collect()
}

fn fn_struct_count(args: Vec<CfmlValue>) -> CfmlResult {
    // Phase C.3 — Slice 4: a flyweight instance enumerates its public members from
    // the data maps directly (no `__` filter). `is_instance_backed()` is const
    // `false` in a default build, so the marker path below is untouched.
    if let Some(comp) = args.first().and_then(|v| v.as_component()) {
        if comp.is_instance_backed() {
            return Ok(CfmlValue::Int(comp.instance_public_keys().len() as i64));
        }
    }
    match args.first() {
        Some(CfmlValue::Struct(s)) => Ok(CfmlValue::Int(visible_struct_keys(s).len() as i64)),
        // A query's columns are its keys (Lucee parity, issue #146).
        Some(CfmlValue::Query(q)) => Ok(CfmlValue::Int(q.column_count() as i64)),
        _ => Ok(CfmlValue::Int(0)),
    }
}

fn fn_struct_key_exists(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        let key = args[1].as_string();
        // Phase C.3 — Slice 4: flyweight instance — check public members directly.
        if let Some(comp) = args[0].as_component() {
            if comp.is_instance_backed() {
                let exists = comp
                    .instance_public_keys()
                    .iter()
                    .any(|k| k.eq_ignore_ascii_case(&key));
                return Ok(CfmlValue::Bool(exists));
            }
        }
        match &args[0] {
            CfmlValue::Struct(s) => {
                let found = struct_find_key_ci(s, &key).is_some();
                // Lucee parity: "a NULL value is the same as not existing in CFML" — a key
                // that is present but holds null reports as absent. Verified on Lucee 7:
                // `s.foo = nullValue(); structKeyExists(s,"foo")` → false. Without this,
                // Preside's `if ( StructKeyExists( page, prop ) ) { value = page[prop]; ... }`
                // enters the block for a null property then throws reading the null `value`.
                if found && matches!(s.get_ci(&key), Some(CfmlValue::Null)) {
                    return Ok(CfmlValue::Bool(false));
                }
                // Hide the private arguments-scope markers from introspection.
                if found
                    && s.contains_key("__arguments_scope")
                    && (key == "__arguments_scope" || key == "__arguments_params")
                {
                    return Ok(CfmlValue::Bool(false));
                }
                // The magic-scope marker is never a user-visible key, and a
                // magic scope (cgi) reports every UNSET key as absent (Lucee
                // parity) even though reading it yields "".
                if key == cfml_common::dynamic::EMPTY_DEFAULT_SCOPE_MARKER {
                    return Ok(CfmlValue::Bool(false));
                }
                // A CFC instance exposes only public members; its engine
                // internals (__name/__variables/...) and private methods are
                // not keys (Lucee/ACF parity). Defer to visible_struct_keys so
                // StructKeyExists never disagrees with StructKeyList/for-in.
                if found {
                    // Engine-internal marker keys are always stored lowercase,
                    // so probe them with the O(1) exact `contains_key` rather
                    // than a ci lookup (issue #262). ("this" can be user-cased,
                    // but the live-alias check needs contains_key_ci anyway.)
                    let is_component = s.contains_key("__variables")
                        && (s.contains_key("__name") || s.contains_key_ci("this"));
                    if is_component
                        && !visible_struct_keys(s)
                            .iter()
                            .any(|k| k.eq_ignore_ascii_case(&key))
                    {
                        return Ok(CfmlValue::Bool(false));
                    }
                }
                return Ok(CfmlValue::Bool(found));
            }
            // Lucee treats a query's columns as its keys, so the struct
            // introspection BIFs work on a query (e.g. cfdbinfo type="version"
            // results, issue #146). Row count is irrelevant.
            CfmlValue::Query(q) => {
                let found = q.columns().iter().any(|c| c.eq_ignore_ascii_case(&key));
                return Ok(CfmlValue::Bool(found));
            }
            _ => {}
        }
    }
    Ok(CfmlValue::Bool(false))
}

fn fn_struct_key_list(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(comp) = args.first().and_then(|v| v.as_component()) {
        if comp.is_instance_backed() {
            let delimiter = get_delimiter(&args, 1);
            return Ok(CfmlValue::string(comp.instance_public_keys().join(&delimiter)));
        }
    }
    match args.first() {
        Some(CfmlValue::Struct(s)) => {
            let delimiter = get_delimiter(&args, 1);
            Ok(CfmlValue::string(visible_struct_keys(s).join(&delimiter)))
        }
        Some(CfmlValue::Query(q)) => {
            let delimiter = get_delimiter(&args, 1);
            Ok(CfmlValue::string(q.columns().join(&delimiter)))
        }
        _ => Ok(CfmlValue::string(String::new())),
    }
}

fn fn_struct_key_array(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(comp) = args.first().and_then(|v| v.as_component()) {
        if comp.is_instance_backed() {
            let keys: Vec<CfmlValue> = comp
                .instance_public_keys()
                .into_iter()
                .map(CfmlValue::string)
                .collect();
            return Ok(CfmlValue::array(keys));
        }
    }
    let keys: Vec<CfmlValue> = match args.first() {
        Some(CfmlValue::Struct(s)) => {
            visible_struct_keys(s).into_iter().map(CfmlValue::string).collect()
        }
        Some(CfmlValue::Query(q)) => {
            q.columns().into_iter().map(CfmlValue::string).collect()
        }
        _ => Vec::new(),
    };
    Ok(CfmlValue::array(keys))
}

/// Component flyweight: if `v` is an instance-backed component, project its PUBLIC
/// scope (data + public methods, accessor-private hidden) into a plain struct so the
/// struct-family BIFs (read/search/copy) can reuse their existing `CfmlValue::Struct`
/// logic on it. Returns None for a marker component, a plain struct, or any
/// non-component — those already flow through the existing `Struct` arms. NOTE: the
/// projection is a snapshot copy, so this is ONLY correct for read-only BIFs; mutating
/// BIFs must go through the live `instance_set_public`/`instance_delete_public` helpers.
#[inline]
fn instance_public_as_struct(v: &CfmlValue) -> Option<CfmlValue> {
    v.as_component()
        .filter(|c| c.is_instance_backed())
        .map(|c| CfmlValue::strukt(c.instance_public_members()))
}

fn fn_struct_delete(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        // Flyweight component: delete the public member in place via the live
        // instance (a Struct-only match no-oped, silently reporting deletion of a
        // key it never removed).
        if let Some(comp) = args[0].as_component().filter(|c| c.is_instance_backed()) {
            let key = args[1].as_string();
            let existed = comp.instance_has_public(&key);
            comp.instance_delete_public(&key);
            let indicate = args.get(2).map(|v| v.is_true()).unwrap_or(false);
            return Ok(CfmlValue::Bool(if indicate { existed } else { true }));
        }
        if let CfmlValue::Struct(s) = &args[0] {
            // Mutate the shared handle in place (Lucee reference semantics):
            // aliases of the struct observe the deletion.
            let key = args[1].as_string();
            let existed = struct_find_key_ci(s, &key).is_some();
            s.remove_ci(&key);
            // StructDelete returns a BOOLEAN, not the struct (Lucee/ACF). With
            // indicateNotExisting=true it reports whether the key was present;
            // otherwise it is always true. (Wheels' flashDelete returns this
            // value directly and asserts toBeTrue.)
            let indicate = args.get(2).map(|v| v.is_true()).unwrap_or(false);
            return Ok(CfmlValue::Bool(if indicate { existed } else { true }));
        }
    }
    Ok(CfmlValue::Bool(false))
}

fn fn_struct_insert(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 3 {
        // Flyweight component: set the public member in place (a Struct-only match
        // no-oped, so structInsert/structUpdate on a component were silently lost).
        if let Some(comp) = args[0].as_component().filter(|c| c.is_instance_backed()) {
            let key = args[1].as_string();
            let allow_overwrite = if args.len() >= 4 { args[3].is_true() } else { true };
            if comp.instance_has_public(&key) && !allow_overwrite {
                return Err(CfmlError::runtime(format!("Key '{}' already exists in struct. Use allowOverwrite=true to overwrite.", key)));
            }
            comp.instance_set_public(key, args[2].clone());
            return Ok(args[0].clone());
        }
        if let CfmlValue::Struct(s) = &args[0] {
            // Mutate the shared handle in place (Lucee reference semantics).
            let key = args[1].as_string();
            let allow_overwrite = if args.len() >= 4 { args[3].is_true() } else { true };
            if let Some(actual_key) = struct_find_key_ci(s, &key) {
                if !allow_overwrite {
                    return Err(CfmlError::runtime(format!("Key '{}' already exists in struct. Use allowOverwrite=true to overwrite.", key)));
                }
                // Replace an existing case-variant key so the new casing wins.
                if actual_key != key {
                    s.remove(&actual_key);
                }
            }
            s.insert(key, args[2].clone());
            return Ok(CfmlValue::Struct(s.clone()));
        }
    }
    Ok(CfmlValue::Bool(false))
}

fn fn_struct_update(args: Vec<CfmlValue>) -> CfmlResult {
    fn_struct_insert(args)
}

fn fn_struct_find(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(s) = args.first().and_then(instance_public_as_struct) { let mut a = args; a[0] = s; return fn_struct_find(a); }
    if args.len() >= 2 {
        if let CfmlValue::Struct(s) = &args[0] {
            let key = args[1].as_string();
            if let Some(actual_key) = struct_find_key_ci(s, &key) {
                return Ok(s.get(&actual_key).unwrap_or(CfmlValue::Null));
            }
            return Ok(CfmlValue::Null);
        }
    }
    Ok(CfmlValue::Null)
}

fn fn_struct_find_key(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(s) = args.first().and_then(instance_public_as_struct) { let mut a = args; a[0] = s; return fn_struct_find_key(a); }
    if args.len() >= 2 {
        if let CfmlValue::Struct(s) = &args[0] {
            let key = get_str(&args, 1);
            let scope = if args.len() >= 3 { get_str(&args, 2).to_lowercase() } else { "one".to_string() };
            let mut results = Vec::new();
            struct_find_key_recursive(s, &key, "", &scope, &mut results);
            return Ok(CfmlValue::array(results));
        }
    }
    Ok(CfmlValue::array(Vec::new()))
}

fn struct_find_key_recursive(
    s: &CfmlStruct,
    search_key: &str,
    path: &str,
    scope: &str,
    results: &mut Vec<CfmlValue>,
) {
    let search_lower = search_key.to_lowercase();
    for (k, v) in s.iter() {
        let current_path = if path.is_empty() { k.clone() } else { format!("{}.{}", path, k) };
        if k.eq_ignore_ascii_case(&search_lower) {
            let mut result_struct = ValueMap::default();
            result_struct.insert("owner".to_string(), CfmlValue::Struct(s.clone()));
            result_struct.insert("path".to_string(), CfmlValue::string(current_path.clone()));
            result_struct.insert("value".to_string(), v.clone());
            results.push(CfmlValue::strukt(result_struct));
            if scope == "one" { return; }
        }
        if let CfmlValue::Struct(nested) = &v {
            struct_find_key_recursive(nested, search_key, &current_path, scope, results);
            if scope == "one" && !results.is_empty() { return; }
        }
        if let CfmlValue::Array(arr) = &v {
            for (i, item) in arr.iter().enumerate() {
                if let CfmlValue::Struct(nested) = &item {
                    let arr_path = format!("{}[{}]", current_path, i + 1);
                    struct_find_key_recursive(nested, search_key, &arr_path, scope, results);
                    if scope == "one" && !results.is_empty() { return; }
                }
            }
        }
    }
}

fn fn_struct_find_value(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(s) = args.first().and_then(instance_public_as_struct) { let mut a = args; a[0] = s; return fn_struct_find_value(a); }
    if args.len() >= 2 {
        if let CfmlValue::Struct(s) = &args[0] {
            let search_value = get_str(&args, 1);
            let scope = if args.len() >= 3 { get_str(&args, 2).to_lowercase() } else { "one".to_string() };
            let mut results = Vec::new();
            struct_find_value_recursive(s, &search_value, "", &scope, &mut results);
            return Ok(CfmlValue::array(results));
        }
    }
    Ok(CfmlValue::array(Vec::new()))
}

fn struct_find_value_recursive(
    s: &CfmlStruct,
    search_value: &str,
    path: &str,
    scope: &str,
    results: &mut Vec<CfmlValue>,
) {
    let search_lower = search_value.to_lowercase();
    for (k, v) in s.iter() {
        let current_path = if path.is_empty() { k.clone() } else { format!("{}.{}", path, k) };
        if v.as_string().to_lowercase() == search_lower {
            let mut result_struct = ValueMap::default();
            result_struct.insert("owner".to_string(), CfmlValue::Struct(s.clone()));
            result_struct.insert("path".to_string(), CfmlValue::string(current_path.clone()));
            result_struct.insert("key".to_string(), CfmlValue::string(k.clone()));
            results.push(CfmlValue::strukt(result_struct));
            if scope == "one" { return; }
        }
        if let CfmlValue::Struct(nested) = &v {
            struct_find_value_recursive(nested, search_value, &current_path, scope, results);
            if scope == "one" && !results.is_empty() { return; }
        }
        if let CfmlValue::Array(arr) = &v {
            for (i, item) in arr.iter().enumerate() {
                if let CfmlValue::Struct(nested) = &item {
                    let arr_path = format!("{}[{}]", current_path, i + 1);
                    struct_find_value_recursive(nested, search_value, &arr_path, scope, results);
                    if scope == "one" && !results.is_empty() { return; }
                }
            }
        }
    }
}

fn fn_struct_clear(args: Vec<CfmlValue>) -> CfmlResult {
    // Phase C.3 — Slice 4/5: a flyweight instance clears its public scope (data +
    // method table) in place, keeping identity + private data (MockBox pattern).
    if let Some(comp) = args.first().and_then(|v| v.as_component()) {
        if comp.is_instance_backed() {
            comp.instance_clear_public();
            return Ok(args.into_iter().next().unwrap_or(CfmlValue::Null));
        }
    }
    // Empty the shared handle in place (Lucee reference semantics).
    if let Some(CfmlValue::Struct(s)) = args.first() {
        // A CFC instance is represented as a Struct carrying engine-internal
        // sentinel keys (`__variables`, `__name`, `__source_file`, `__super`,
        // `__metadata`, `__properties`). Clearing those destroys the object's
        // identity and private scope, so a method invoked afterwards can no
        // longer bind `this` ("Variable 'this' is undefined"). Lucee clears the
        // public THIS scope of a component but keeps it a usable object — so
        // when the target is a component, preserve the `__`-prefixed sentinels
        // and drop only the user-facing public members (which is exactly what
        // MockBox's clearMethods=true relies on). (GitHub #177)
        let is_component =
            s.contains_key_ci("__variables") && (s.contains_key_ci("__name") || s.contains_key_ci("this"));
        if is_component {
            // Preserve ONLY the exact engine sentinels — a user/framework `__`/`___`
            // public data member (FW/1 AOP `___orig`) is user data and MUST be
            // cleared like any other public member (C.4 marker-path narrowing).
            let preserved: Vec<(String, CfmlValue)> = s
                .iter()
                .filter(|(k, _)| cfml_common::component::is_reserved_component_key(k))
                .collect();
            s.clear();
            // Methods live in the shared per-class table (component flyweight);
            // clearing the map alone would leave them resolvable via delegation.
            // Drop the table too so `structKeyExists(cleared, "init")` is false
            // and the object has no methods until re-mixed (MockBox clearMethods).
            s.clear_method_table();
            for (k, v) in preserved {
                s.insert(k, v);
            }
            if let Some(CfmlValue::Struct(vs)) = s.get("__variables") {
                vs.clear_method_table();
            }
            return Ok(CfmlValue::Struct(s.clone()));
        }
        s.clear();
        return Ok(CfmlValue::Struct(s.clone()));
    }
    Ok(CfmlValue::strukt(ValueMap::default()))
}

fn fn_struct_copy(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(s) = args.first().and_then(instance_public_as_struct) { let mut a = args; a[0] = s; return fn_struct_copy(a); }
    // Shallow copy: a fresh top-level struct over the same (shared) values.
    // A component's `variables` SCOPE is a plain Struct with the shared method
    // table attached (GH #285) — union it in so the copy keeps its function
    // members, matching Lucee (functions are ordinary struct values here).
    match args.first() {
        Some(CfmlValue::Struct(s)) if s.method_table().is_some() => {
            Ok(CfmlValue::strukt(s.snapshot_with_methods()))
        }
        Some(CfmlValue::Struct(s)) => Ok(CfmlValue::strukt(s.snapshot())),
        _ => Ok(CfmlValue::strukt(ValueMap::default())),
    }
}

fn fn_struct_append(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        // Phase C.3 — Slice 4: destination is a flyweight instance (the mixin
        // injection path `structAppend(target, mixins, true)` — the merged Function
        // values become injected data-methods on the instance). `is_instance_backed`
        // is const false in a default build, so the marker path below is untouched.
        if let Some(dst) = args[0].as_component() {
            if dst.is_instance_backed() {
                let overwrite = if args.len() >= 3 { args[2].is_true() } else { true };
                // Source members: a plain struct's entries, or a component's public
                // members (data + public methods). Skip engine `__` keys from a
                // marker source; user `__`/`___` DATA keys from a flyweight source
                // are already the only `__` keys its public view yields.
                let src: ValueMap = match &args[1] {
                    CfmlValue::Struct(b) => b.snapshot(),
                    other => other
                        .as_component()
                        .map(|c| c.instance_public_members())
                        .unwrap_or_default(),
                };
                let src_is_marker = matches!(&args[1], CfmlValue::Struct(b)
                    if b.contains_key("__variables") || b.contains_key("__name"));
                for (k, v) in src {
                    if src_is_marker
                        && (cfml_common::component::is_reserved_component_key(&k)
                            || k.eq_ignore_ascii_case("this")
                            || k.eq_ignore_ascii_case("super"))
                    {
                        continue;
                    }
                    if !overwrite && dst.instance_has_public(&k) {
                        continue;
                    }
                    dst.instance_set_public(k, v);
                }
                return Ok(args[0].clone());
            }
        }
        if let (CfmlValue::Struct(a), CfmlValue::Struct(b)) = (&args[0], &args[1]) {
            // Mutate the first struct's shared handle in place (Lucee semantics).
            let overwrite = if args.len() >= 3 { args[2].is_true() } else { true };
            // When the SOURCE is a component (ColdBox does
            // `structAppend(settings, new Settings(), true)` to fold a config
            // CFC's public settings into a plain struct), copy only its public
            // data keys — NOT the engine-internal markers (`__variables`,
            // `__name`, `__is_component`, …) or the `this`/`super` self-refs.
            // Copying those turned the destination into a pseudo-component
            // (`isObject()` true), so a later `settings.keyExists(...)` dispatched
            // as a component method and threw "has no function with name".
            let src_is_component = b.contains_key("__variables")
                || b.contains_key("__name")
                || b.contains_key("__is_component");
            // For a component source, its methods live in the shared method table
            // (component-model flyweight), NOT the instance map, so `b.iter()`
            // (map-only) would miss them entirely — e.g. TestBox's
            // `addMatchers( new CustomMatcher() )` folds a matcher CFC's public
            // methods into a plain struct. A component's `variables` SCOPE
            // (`StructAppend(c, variables)` — Wheels' plugin snapshot, GH #285)
            // is a plain Struct with the method table attached but WITHOUT the
            // `__` marker keys, so detect the table directly. Enumerate map ∪
            // table whenever a table is present.
            let src_has_methods = b.method_table().is_some();
            let entries: Vec<(String, CfmlValue)> = if src_is_component || src_has_methods {
                b.all_entries()
            } else {
                b.iter().collect()
            };
            for (k, v) in entries {
                if (src_is_component || src_has_methods)
                    && (cfml_common::component::is_reserved_component_key(&k)
                        || k.eq_ignore_ascii_case("this")
                        || k.eq_ignore_ascii_case("super"))
                {
                    continue;
                }
                if overwrite || struct_find_key_ci(a, &k).is_none() {
                    a.insert(k, v);
                }
            }
            return Ok(CfmlValue::Struct(a.clone()));
        }
        // Phase C.3 — Slice 4: destination is a plain struct, SOURCE is a flyweight
        // instance (ColdBox `structAppend(settings, new Settings())`, TestBox
        // `addMatchers(new CustomMatcher())` — fold the CFC's public data + methods
        // into the struct).
        if let CfmlValue::Struct(a) = &args[0] {
            if let Some(src) = args[1].as_component() {
                if src.is_instance_backed() {
                    let overwrite = if args.len() >= 3 { args[2].is_true() } else { true };
                    for (k, v) in src.instance_public_members() {
                        if overwrite || struct_find_key_ci(a, &k).is_none() {
                            a.insert(k, v);
                        }
                    }
                    return Ok(CfmlValue::Struct(a.clone()));
                }
            }
        }
    }
    Ok(args.into_iter().next().unwrap_or(CfmlValue::strukt(ValueMap::default())))
}

fn fn_struct_is_empty(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(s) = args.first().and_then(instance_public_as_struct) { let mut a = args; a[0] = s; return fn_struct_is_empty(a); }
    match args.first() {
        Some(CfmlValue::Struct(s)) => Ok(CfmlValue::Bool(s.is_empty())),
        // A query's columns are its keys, so a query with columns is never
        // "empty" — even with zero rows (Lucee parity, issue #146).
        Some(CfmlValue::Query(q)) => Ok(CfmlValue::Bool(q.column_count() == 0)),
        _ => Ok(CfmlValue::Bool(true)),
    }
}

fn fn_struct_sort(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(s) = args.first().and_then(instance_public_as_struct) { let mut a = args; a[0] = s; return fn_struct_sort(a); }
    if let Some(CfmlValue::Struct(s)) = args.first() {
        let sort_type = if args.len() > 1 { get_str(&args, 1).to_lowercase() } else { "text".to_string() };
        let sort_order = if args.len() > 2 { get_str(&args, 2).to_lowercase() } else { "asc".to_string() };
        let mut keys: Vec<String> = s.keys();
        match sort_type.as_str() {
            "numeric" => {
                keys.sort_by(|a, b| {
                    let va = s.get(a).map(|v| v.as_string().parse::<f64>().unwrap_or(0.0)).unwrap_or(0.0);
                    let vb = s.get(b).map(|v| v.as_string().parse::<f64>().unwrap_or(0.0)).unwrap_or(0.0);
                    va.partial_cmp(&vb).unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            "textnocase" => {
                keys.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
            }
            _ => keys.sort(),
        }
        if sort_order == "desc" { keys.reverse(); }
        Ok(CfmlValue::array(keys.into_iter().map(CfmlValue::string).collect()))
    } else {
        Ok(CfmlValue::array(Vec::new()))
    }
}

fn fn_struct_each(_args: Vec<CfmlValue>) -> CfmlResult { Ok(CfmlValue::Null) }
fn fn_struct_map(args: Vec<CfmlValue>) -> CfmlResult { Ok(args.into_iter().next().unwrap_or(CfmlValue::Null)) }
fn fn_struct_filter(args: Vec<CfmlValue>) -> CfmlResult { Ok(args.into_iter().next().unwrap_or(CfmlValue::Null)) }

fn fn_is_struct(args: Vec<CfmlValue>) -> CfmlResult {
    let is = match args.first() {
        Some(CfmlValue::Struct(s)) => {
            // A java shim is only a struct to CFML code if it's a Map
            // implementation (java.util.Map family) — Lucee treats those as
            // struct-compatible (structKeyList/each/etc. all work). Every other
            // shim (java.net.URL, StringBuilder, File, ...) is an object, not a
            // struct, so isStruct() is false. This lets TestBox's equalize(),
            // which short-circuits its generic struct-key walk on isStruct(),
            // fall through to the shim's own .equals() instead of treating all
            // instances as interchangeable empty structs (GH #238).
            if s.contains_key("__java_shim") {
                s.get("__java_class")
                    .map(|v| {
                        let c = v.as_string().to_lowercase();
                        (c.contains("map") && !c.ends_with("map.entry"))
                            || c.contains("hashtable")
                            || c.contains("dictionary")
                            || c == "java.util.properties"
                    })
                    .unwrap_or(false)
            } else {
                true
            }
        }
        // A flyweight component instance is struct-like, exactly as the marker
        // representation was (RustCFML treats components as structs —
        // structKeyList/structEach/etc. all work). Detected via the facade rather
        // than a `#[cfg]` arm (cfml-stdlib has no `component-instance` feature).
        Some(other) if other.is_component() => true,
        _ => false,
    };
    Ok(CfmlValue::Bool(is))
}

fn fn_struct_get(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = CfmlValue::strukt(ValueMap::default());
    for part in parts.iter().rev() {
        let mut s = ValueMap::default();
        s.insert(part.to_string(), current);
        current = CfmlValue::strukt(s);
    }
    let mut result = current;
    for part in &parts {
        let next = if let CfmlValue::Struct(s) = &result {
            s.get(*part)
        } else {
            None
        };
        if let Some(v) = next {
            result = v;
        }
    }
    Ok(result)
}

fn fn_struct_value_array(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(s) = args.first().and_then(instance_public_as_struct) { let mut a = args; a[0] = s; return fn_struct_value_array(a); }
    if let Some(CfmlValue::Struct(s)) = args.first() {
        let values: Vec<CfmlValue> = s.iter().map(|(_, v)| v).collect();
        Ok(CfmlValue::array(values))
    } else {
        Ok(CfmlValue::array(Vec::new()))
    }
}

fn fn_struct_equals(args: Vec<CfmlValue>) -> CfmlResult {
    // Either side may be a flyweight component — project each to its public scope.
    if args.len() >= 2
        && (instance_public_as_struct(&args[0]).is_some() || instance_public_as_struct(&args[1]).is_some())
    {
        let mut a = args;
        if let Some(s) = instance_public_as_struct(&a[0]) { a[0] = s; }
        if let Some(s) = instance_public_as_struct(&a[1]) { a[1] = s; }
        return fn_struct_equals(a);
    }
    if args.len() >= 2 {
        if let (CfmlValue::Struct(a), CfmlValue::Struct(b)) = (&args[0], &args[1]) {
            if a.len() != b.len() { return Ok(CfmlValue::Bool(false)); }
            for (k, v) in a.iter() {
                match b.get(&k) {
                    Some(bv) => {
                        if v.as_string() != bv.as_string() {
                            return Ok(CfmlValue::Bool(false));
                        }
                    }
                    None => {
                        match struct_find_key_ci(b, &k) {
                            Some(actual) => {
                                if v.as_string() != b.get(&actual).unwrap().as_string() {
                                    return Ok(CfmlValue::Bool(false));
                                }
                            }
                            None => return Ok(CfmlValue::Bool(false)),
                        }
                    }
                }
            }
            return Ok(CfmlValue::Bool(true));
        }
    }
    Ok(CfmlValue::Bool(false))
}

fn fn_struct_key_translate(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(s) = args.first().and_then(instance_public_as_struct) { let mut a = args; a[0] = s; return fn_struct_key_translate(a); }
    if let Some(CfmlValue::Struct(s)) = args.first() {
        let retain = args.get(1).map(|v| v.is_true()).unwrap_or(false);
        let mut result = ValueMap::default();
        for (k, v) in s.iter() {
            let new_key = if retain { k.clone() } else { k.to_lowercase() };
            result.insert(new_key, v.clone());
        }
        return Ok(CfmlValue::strukt(result));
    }
    Err(CfmlError::runtime("structKeyTranslate requires a struct argument".into()))
}

// ===============================================
// TYPE CHECKING FUNCTIONS
// ===============================================

fn fn_is_null(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Bool(matches!(args.first(), None | Some(CfmlValue::Null))))
}

fn fn_is_defined(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Bool(!matches!(args.first(), None | Some(CfmlValue::Null))))
}

fn fn_is_simple_value(args: Vec<CfmlValue>) -> CfmlResult {
    // A bare query-column access (q.col) yields a QueryColumn proxy that stands
    // in for its first-row scalar in scalar contexts (arithmetic, coercion,
    // isNumeric — see to_number/fn_is_numeric). A single query cell IS a simple
    // value on Lucee/ACF/BoxLang, so unwrap the proxy before the type test.
    Ok(CfmlValue::Bool(matches!(
        args.first().map(|v| v.query_column_scalar()),
        Some(
            CfmlValue::Bool(_)
                | CfmlValue::Int(_)
                | CfmlValue::Double(_)
                | CfmlValue::TimeSpan(_)
                | CfmlValue::String(_)
        )
    )))
}

fn fn_is_numeric(args: Vec<CfmlValue>) -> CfmlResult {
    // A QueryColumn proxy (bare q.col access) behaves as its first-row scalar
    // value in numeric type tests — mirrors to_number/cfml_equal/cfml_compare.
    match args.first().map(|v| v.query_column_scalar()) {
        Some(CfmlValue::Int(_)) | Some(CfmlValue::Double(_)) => Ok(CfmlValue::Bool(true)),
        Some(CfmlValue::String(s)) => Ok(CfmlValue::Bool(s.trim().parse::<f64>().is_ok())),
        // A CFML boolean is NOT numeric — isNumeric(true)/isNumeric(false) are
        // false on Lucee, Adobe CF and BoxLang. (Wheels guards its finder
        // `parameterize` flag with isNumeric(), which defaults to boolean true.)
        _ => Ok(CfmlValue::Bool(false)),
    }
}

fn fn_is_boolean(args: Vec<CfmlValue>) -> CfmlResult {
    // A bare query-column access (q.col) yields a QueryColumn proxy standing in
    // for its first-row scalar; unwrap it before the type test, matching
    // isNumeric/isDate/isSimpleValue. (Preside FormBuilderService.isFormActive
    // guards on IsBoolean(formRecord.active) where `active` is a boolean query
    // column.)
    match args.first().map(|v| v.query_column_scalar()) {
        Some(CfmlValue::Bool(_)) => Ok(CfmlValue::Bool(true)),
        Some(CfmlValue::Int(_)) | Some(CfmlValue::Double(_)) => Ok(CfmlValue::Bool(true)),
        Some(CfmlValue::String(s)) => {
            let lower = s.trim().to_lowercase();
            Ok(CfmlValue::Bool(
                matches!(lower.as_str(), "true" | "false" | "yes" | "no")
                || s.trim().parse::<f64>().is_ok()
            ))
        }
        _ => Ok(CfmlValue::Bool(false)),
    }
}

fn fn_is_date(args: Vec<CfmlValue>) -> CfmlResult {
    // Lucee: a bare number (or numeric string) is NOT a date — only date/datetime
    // STRINGS parse. parse_cfml_date intentionally accepts an OLE date serial for
    // ParseDateTime/CreateDate, so IsDate must reject numerics up front.
    // A timespan IS a date in Lucee (isDate(createTimeSpan(...)) is true) — it is
    // a duration on the date/time axis. Report true before the numeric guard.
    if matches!(args.first().map(|v| v.query_column_scalar()), Some(CfmlValue::TimeSpan(_))) {
        return Ok(CfmlValue::Bool(true));
    }
    match args.first().map(|v| v.query_column_scalar()) {
        Some(CfmlValue::Int(_)) | Some(CfmlValue::Double(_)) | Some(CfmlValue::Bool(_)) => {
            return Ok(CfmlValue::Bool(false));
        }
        _ => {}
    }
    let s = get_str(&args, 0);
    let t = s.trim();
    if !t.is_empty() && t.parse::<f64>().is_ok() {
        return Ok(CfmlValue::Bool(false));
    }
    // Lucee treats a bare time-of-day ("6:15 PM", "18:15") as a valid date (it
    // resolves to today at that time). isValid("date"/"datetime") routes here, and
    // Wheels' automatic SQLite datetime validation does validatesFormatOf type="date".
    let is_time = ["%H:%M", "%H:%M:%S", "%I:%M %p", "%I:%M:%S %p"]
        .iter()
        .any(|fmt| NaiveTime::parse_from_str(t, fmt).is_ok());
    Ok(CfmlValue::Bool(is_time || parse_cfml_date(&s).is_some()))
}

fn fn_is_query(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Bool(matches!(args.first(), Some(CfmlValue::Query(_)))))
}

fn fn_is_object(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Bool(match args.first() {
        Some(CfmlValue::Component(_)) | Some(CfmlValue::NativeObject(_)) => true,
        Some(CfmlValue::Struct(s)) => {
            s.contains_key("__name") || s.contains_key("__java_shim")
        }
        // Flyweight component instance (Phase C.3). Detected via the facade rather
        // than a `#[cfg]` arm: cfml-stdlib has no `component-instance` feature of
        // its own (feature unification supplies the variant), so a cfg gate would
        // compile this arm out. `is_component()` is `false` for every scalar.
        Some(other) => other.is_component(),
        None => false,
    }))
}

/// IsImageFile(path) — true only if the file exists AND its content is a
/// recognized raster/vector image (Lucee checks content, not extension: a
/// text file renamed `.png` returns false). Magic-byte sniff of the common
/// formats ImageIO/Lucee can read.
fn fn_is_image_file(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    if path.is_empty() {
        return Ok(CfmlValue::Bool(false));
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return Ok(CfmlValue::Bool(false)),
    };
    let b: &[u8] = bytes.as_slice();
    let is_img = b.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) // PNG
        || b.starts_with(&[0xFF, 0xD8, 0xFF])           // JPEG
        || b.starts_with(b"GIF87a")
        || b.starts_with(b"GIF89a")
        || b.starts_with(b"BM")                          // BMP
        || b.starts_with(&[0x49, 0x49, 0x2A, 0x00])     // TIFF little-endian
        || b.starts_with(&[0x4D, 0x4D, 0x00, 0x2A])     // TIFF big-endian
        || b.starts_with(&[0x00, 0x00, 0x01, 0x00])     // ICO
        || b.starts_with(b"8BPS")                        // PSD
        || (b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP")
        // SVG: an XML/text vector image — sniff for an <svg root within the head
        || {
            let head = String::from_utf8_lossy(&b[..b.len().min(512)]).to_lowercase();
            head.contains("<svg")
        };
    Ok(CfmlValue::Bool(is_img))
}

/// GetReadableImageFormats() / GetWriteableImageFormats() — comma list of the
/// image formats the engine can read. Wheels only requires a non-empty simple
/// value; the set mirrors Lucee/ImageIO.
fn fn_get_readable_image_formats(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::string(
        "BMP,GIF,JPEG,JPG,PNG,PSD,TIF,TIFF,WBMP,WEBP".to_string(),
    ))
}

/// Tier 2 (drawing) and Tier 3 (filters/transforms/metadata) image function
/// names. When `image_support` is OFF these are registered as disabled stubs so
/// calls fail with a clear message rather than "undefined function"; when it is
/// ON they are wired to real implementations in `register_image_functions`.
#[cfg(not(feature = "image_support"))]
const IMAGE_TIER23_STUBS: &[&str] = &[
    // drawing state
    "imageSetDrawingColor", "imageSetBackgroundColor", "imageSetDrawingStroke",
    "imageSetAntialiasing", "imageSetDrawingTransparency", "imageXORDrawingMode",
    // drawing primitives
    "imageDrawLine", "imageDrawLines", "imageDrawPoint", "imageDrawRect",
    "imageDrawRoundRect", "imageDrawBeveledRect", "imageDrawOval", "imageDrawArc",
    "imageDrawCubicCurve", "imageDrawQuadraticCurve", "imageDrawText",
    "imageClearRect", "imageDrawImage", "imageAddBorder",
    // compositing
    "imagePaste", "imageOverlay", "imageCopy",
    // filters / effects
    "imageBlur", "imageSharpen", "imageNegative", "imageGrayscale",
    "imageMakeColorTransparent", "imageMakeTranslucent",
    // coordinate transforms
    "imageTranslate", "imageTranslateDrawingAxis", "imageShear",
    "imageShearDrawingAxis", "imageRotateDrawingAxis",
    // metadata / interop
    "imageGetEXIFMetadata", "imageGetEXIFTag", "imageGetIPTCMetadata",
    "imageGetIPTCTag", "imageGetBufferedImage",
];

/// Tier 1 (implemented when `image_support` is on) function names — used only to
/// register disabled-build stubs when the feature is off.
#[cfg(not(feature = "image_support"))]
const IMAGE_TIER1_NAMES: &[&str] = &[
    "imageNew", "imageRead", "imageReadBase64", "imageWrite", "imageWriteBase64",
    "imageGetBlob", "imageResize", "imageScaleToFit", "imageGetWidth",
    "imageGetHeight", "imageInfo", "imageCrop", "imageRotate", "imageFlip",
];

#[cfg(feature = "image_support")]
fn register_image_functions(f: &mut HashMap<String, BuiltinFunction>) {
    use crate::image as img;
    f.insert("imageNew".into(), img::fn_image_new);
    f.insert("imageRead".into(), img::fn_image_read);
    f.insert("imageReadBase64".into(), img::fn_image_read_base64);
    f.insert("imageWrite".into(), img::fn_image_write);
    f.insert("imageWriteBase64".into(), img::fn_image_write_base64);
    f.insert("imageGetBlob".into(), img::fn_image_get_blob);
    f.insert("imageResize".into(), img::fn_image_resize);
    f.insert("imageScaleToFit".into(), img::fn_image_scale_to_fit);
    f.insert("imageGetWidth".into(), img::fn_image_get_width);
    f.insert("imageGetHeight".into(), img::fn_image_get_height);
    f.insert("imageInfo".into(), img::fn_image_info);
    f.insert("imageCrop".into(), img::fn_image_crop);
    f.insert("imageRotate".into(), img::fn_image_rotate);
    f.insert("imageFlip".into(), img::fn_image_flip);
    f.insert("isImage".into(), img::fn_is_image);
    f.insert("cfimage".into(), img::fn_cfimage);

    // Tier 2 — drawing state
    f.insert("imageSetDrawingColor".into(), img::fn_image_set_drawing_color);
    f.insert("imageSetBackgroundColor".into(), img::fn_image_set_background_color);
    f.insert("imageSetDrawingStroke".into(), img::fn_image_set_drawing_stroke);
    f.insert("imageSetAntialiasing".into(), img::fn_image_set_antialiasing);
    f.insert("imageSetDrawingTransparency".into(), img::fn_image_set_drawing_transparency);
    f.insert("imageXORDrawingMode".into(), img::fn_image_xor_drawing_mode);
    // Tier 2 — drawing primitives
    f.insert("imageDrawLine".into(), img::fn_image_draw_line);
    f.insert("imageDrawLines".into(), img::fn_image_draw_lines);
    f.insert("imageDrawPoint".into(), img::fn_image_draw_point);
    f.insert("imageDrawRect".into(), img::fn_image_draw_rect);
    f.insert("imageDrawRoundRect".into(), img::fn_image_draw_round_rect);
    f.insert("imageDrawBeveledRect".into(), img::fn_image_draw_beveled_rect);
    f.insert("imageDrawOval".into(), img::fn_image_draw_oval);
    f.insert("imageDrawArc".into(), img::fn_image_draw_arc);
    f.insert("imageDrawCubicCurve".into(), img::fn_image_draw_cubic_curve);
    f.insert("imageDrawQuadraticCurve".into(), img::fn_image_draw_quadratic_curve);
    f.insert("imageDrawText".into(), img::fn_image_draw_text);
    f.insert("imageClearRect".into(), img::fn_image_clear_rect);
    // Tier 2 — compositing
    f.insert("imageDrawImage".into(), img::fn_image_draw_image);
    f.insert("imagePaste".into(), img::fn_image_paste);
    f.insert("imageOverlay".into(), img::fn_image_overlay);
    f.insert("imageCopy".into(), img::fn_image_copy);
    f.insert("imageAddBorder".into(), img::fn_image_add_border);
    // Tier 3 — filters / effects
    f.insert("imageBlur".into(), img::fn_image_blur);
    f.insert("imageSharpen".into(), img::fn_image_sharpen);
    f.insert("imageNegative".into(), img::fn_image_negative);
    f.insert("imageGrayscale".into(), img::fn_image_grayscale);
    f.insert("imageMakeColorTransparent".into(), img::fn_image_make_color_transparent);
    f.insert("imageMakeTranslucent".into(), img::fn_image_make_translucent);
    // Tier 3 — coordinate transforms
    f.insert("imageTranslate".into(), img::fn_image_translate);
    f.insert("imageTranslateDrawingAxis".into(), img::fn_image_translate_drawing_axis);
    f.insert("imageShear".into(), img::fn_image_shear);
    f.insert("imageShearDrawingAxis".into(), img::fn_image_shear_drawing_axis);
    f.insert("imageRotateDrawingAxis".into(), img::fn_image_rotate_drawing_axis);
    // Tier 3 — metadata / interop
    f.insert("imageGetEXIFMetadata".into(), img::fn_image_get_exif_metadata);
    f.insert("imageGetEXIFTag".into(), img::fn_image_get_exif_tag);
    f.insert("imageGetIPTCMetadata".into(), img::fn_image_get_iptc_metadata);
    f.insert("imageGetIPTCTag".into(), img::fn_image_get_iptc_tag);
    f.insert("imageGetBufferedImage".into(), img::fn_image_get_buffered_image);
}

#[cfg(not(feature = "image_support"))]
fn register_image_functions(f: &mut HashMap<String, BuiltinFunction>) {
    // isImage is a type predicate — always safe to answer "false".
    f.insert("isImage".into(), |_args| Ok(CfmlValue::Bool(false)));
    f.insert("cfimage".into(), fn_image_disabled);
    for name in IMAGE_TIER1_NAMES.iter().chain(IMAGE_TIER23_STUBS) {
        f.insert((*name).into(), fn_image_disabled);
    }
}

/// Every Spreadsheet* BIF name — used to register disabled-build stubs when the
/// `spreadsheet` feature is off (native/server-only capability; kept out of wasm).
#[cfg(not(feature = "spreadsheet"))]
const SPREADSHEET_FN_NAMES: &[&str] = &[
    "spreadsheetNew", "spreadsheet", "spreadsheetRead", "spreadsheetReadBinary",
    "spreadsheetSetCellValue", "spreadsheetGetCellValue", "spreadsheetCreateSheet",
    "spreadsheetRenameSheet", "spreadsheetWrite", "spreadsheetInfo", "spreadsheetGetColumnCount",
    "spreadsheetSetActiveSheet", "spreadsheetSetActiveSheetNumber", "spreadsheetAddRow",
    "spreadsheetAddRows", "spreadsheetAddColumn", "spreadsheetFormatCell", "spreadsheetFormatRow",
    "spreadsheetFormatColumn", "spreadsheetFormatCellRange", "spreadsheetMergeCells",
    "spreadsheetAddFreezePane", "spreadsheetSetColumnWidth", "spreadsheetSetRowHeight",
    "spreadsheetDeleteRow", "spreadsheetDeleteRows", "spreadsheetDeleteColumn",
    "spreadsheetDeleteColumns", "spreadsheetShiftRows", "spreadsheetShiftColumns",
    "spreadsheetSetCellFormula", "spreadsheetGetCellFormula", "spreadsheetGetCellType",
    "spreadsheetClearCell", "spreadsheetClearCellRange", "spreadsheetSetCellRangeValue",
    "spreadsheetSetCellComment", "spreadsheetSetCellHyperlink", "spreadsheetAddAutofilter",
    "spreadsheetAddInfo", "spreadsheetAddImage", "spreadsheetAddChart", "spreadsheetToQuery",
    "spreadsheetToArray", "spreadsheetToCsv", "spreadsheetWriteToCsv", "spreadsheetReadCsv",
    "isSpreadsheetFile", "spreadsheetGetCellComment", "spreadsheetGetCellHyperlink",
    "spreadsheetAddSplitPane", "spreadsheetSetPrintOrientation", "spreadsheetSetFitToPage",
    "spreadsheetSetHeader", "spreadsheetSetFooter", "spreadsheetSetColumnHidden",
    "spreadsheetSetRowHidden", "spreadsheetAddDataValidation", "spreadsheetAddConditionalFormatting",
    "spreadsheetGetColumnWidth", "spreadsheetGetCellFormat", "spreadsheetSetActiveCell",
    "spreadsheetAddPageBreaks", "spreadsheetSetRepeatingRows", "spreadsheetSetRepeatingColumns",
    "spreadsheetToJson", "spreadsheetFromJson",
];

#[cfg(feature = "spreadsheet")]
fn register_spreadsheet_functions(f: &mut HashMap<String, BuiltinFunction>) {
    use crate::spreadsheet as ss;
    f.insert("spreadsheetNew".into(), ss::fn_spreadsheet_new);
    f.insert("spreadsheet".into(), ss::fn_spreadsheet);
    f.insert("spreadsheetRead".into(), ss::fn_spreadsheet_read);
    f.insert("spreadsheetReadBinary".into(), ss::fn_spreadsheet_read_binary);
    f.insert("isSpreadsheetObject".into(), ss::fn_is_spreadsheet_object);
    f.insert("spreadsheetSetCellValue".into(), ss::fn_spreadsheet_set_cell_value);
    f.insert("spreadsheetGetCellValue".into(), ss::fn_spreadsheet_get_cell_value);
    f.insert("spreadsheetCreateSheet".into(), ss::fn_spreadsheet_create_sheet);
    f.insert("spreadsheetRenameSheet".into(), ss::fn_spreadsheet_rename_sheet);
    f.insert("spreadsheetWrite".into(), ss::fn_spreadsheet_write);
    f.insert("spreadsheetInfo".into(), ss::fn_spreadsheet_info);
    f.insert("spreadsheetGetColumnCount".into(), ss::fn_spreadsheet_get_column_count);
    f.insert("spreadsheetSetActiveSheet".into(), ss::fn_spreadsheet_set_active_sheet);
    f.insert("spreadsheetSetActiveSheetNumber".into(), ss::fn_spreadsheet_set_active_sheet_number);
    f.insert("spreadsheetAddRow".into(), ss::fn_spreadsheet_add_row);
    f.insert("spreadsheetAddRows".into(), ss::fn_spreadsheet_add_rows);
    f.insert("spreadsheetAddColumn".into(), ss::fn_spreadsheet_add_column);
    f.insert("spreadsheetFormatCell".into(), ss::fn_spreadsheet_format_cell);
    f.insert("spreadsheetFormatRow".into(), ss::fn_spreadsheet_format_row);
    f.insert("spreadsheetFormatColumn".into(), ss::fn_spreadsheet_format_column);
    f.insert("spreadsheetFormatCellRange".into(), ss::fn_spreadsheet_format_cell_range);
    f.insert("spreadsheetMergeCells".into(), ss::fn_spreadsheet_merge_cells);
    f.insert("spreadsheetAddFreezePane".into(), ss::fn_spreadsheet_add_freeze_pane);
    f.insert("spreadsheetSetColumnWidth".into(), ss::fn_spreadsheet_set_column_width);
    f.insert("spreadsheetSetRowHeight".into(), ss::fn_spreadsheet_set_row_height);
    f.insert("spreadsheetDeleteRow".into(), ss::fn_spreadsheet_delete_row);
    f.insert("spreadsheetDeleteRows".into(), ss::fn_spreadsheet_delete_rows);
    f.insert("spreadsheetDeleteColumn".into(), ss::fn_spreadsheet_delete_column);
    f.insert("spreadsheetDeleteColumns".into(), ss::fn_spreadsheet_delete_columns);
    f.insert("spreadsheetShiftRows".into(), ss::fn_spreadsheet_shift_rows);
    f.insert("spreadsheetShiftColumns".into(), ss::fn_spreadsheet_shift_columns);
    f.insert("spreadsheetSetCellFormula".into(), ss::fn_spreadsheet_set_cell_formula);
    f.insert("spreadsheetGetCellFormula".into(), ss::fn_spreadsheet_get_cell_formula);
    f.insert("spreadsheetGetCellType".into(), ss::fn_spreadsheet_get_cell_type);
    f.insert("spreadsheetClearCell".into(), ss::fn_spreadsheet_clear_cell);
    f.insert("spreadsheetClearCellRange".into(), ss::fn_spreadsheet_clear_cell_range);
    f.insert("spreadsheetSetCellRangeValue".into(), ss::fn_spreadsheet_set_cell_range_value);
    f.insert("spreadsheetSetCellComment".into(), ss::fn_spreadsheet_set_cell_comment);
    f.insert("spreadsheetSetCellHyperlink".into(), ss::fn_spreadsheet_set_cell_hyperlink);
    f.insert("spreadsheetAddAutofilter".into(), ss::fn_spreadsheet_add_autofilter);
    f.insert("spreadsheetAddInfo".into(), ss::fn_spreadsheet_add_info);
    f.insert("spreadsheetAddImage".into(), ss::fn_spreadsheet_add_image);
    f.insert("spreadsheetAddChart".into(), ss::fn_spreadsheet_add_chart);
    f.insert("spreadsheetToQuery".into(), ss::fn_spreadsheet_to_query);
    f.insert("spreadsheetToArray".into(), ss::fn_spreadsheet_to_array);
    f.insert("spreadsheetToCsv".into(), ss::fn_spreadsheet_to_csv);
    f.insert("spreadsheetWriteToCsv".into(), ss::fn_spreadsheet_write_to_csv);
    f.insert("spreadsheetReadCsv".into(), ss::fn_spreadsheet_read_csv);
    f.insert("isSpreadsheetFile".into(), ss::fn_is_spreadsheet_file);
    f.insert("spreadsheetGetCellComment".into(), ss::fn_spreadsheet_get_cell_comment);
    f.insert("spreadsheetGetCellHyperlink".into(), ss::fn_spreadsheet_get_cell_hyperlink);
    f.insert("spreadsheetAddSplitPane".into(), ss::fn_spreadsheet_add_split_pane);
    f.insert("spreadsheetSetPrintOrientation".into(), ss::fn_spreadsheet_set_print_orientation);
    f.insert("spreadsheetSetFitToPage".into(), ss::fn_spreadsheet_set_fit_to_page);
    f.insert("spreadsheetSetHeader".into(), ss::fn_spreadsheet_set_header);
    f.insert("spreadsheetSetFooter".into(), ss::fn_spreadsheet_set_footer);
    f.insert("spreadsheetSetColumnHidden".into(), ss::fn_spreadsheet_set_column_hidden);
    f.insert("spreadsheetSetRowHidden".into(), ss::fn_spreadsheet_set_row_hidden);
    f.insert("spreadsheetAddDataValidation".into(), ss::fn_spreadsheet_add_data_validation);
    f.insert("spreadsheetAddConditionalFormatting".into(), ss::fn_spreadsheet_add_conditional_formatting);
    f.insert("spreadsheetGetColumnWidth".into(), ss::fn_spreadsheet_get_column_width);
    f.insert("spreadsheetGetCellFormat".into(), ss::fn_spreadsheet_get_cell_format);
    f.insert("spreadsheetSetActiveCell".into(), ss::fn_spreadsheet_set_active_cell);
    f.insert("spreadsheetAddPageBreaks".into(), ss::fn_spreadsheet_add_page_breaks);
    f.insert("spreadsheetSetRepeatingRows".into(), ss::fn_spreadsheet_set_repeating_rows);
    f.insert("spreadsheetSetRepeatingColumns".into(), ss::fn_spreadsheet_set_repeating_columns);
    f.insert("spreadsheetToJson".into(), ss::fn_spreadsheet_to_json);
    f.insert("spreadsheetFromJson".into(), ss::fn_spreadsheet_from_json);
}

#[cfg(not(feature = "spreadsheet"))]
fn register_spreadsheet_functions(f: &mut HashMap<String, BuiltinFunction>) {
    // isSpreadsheetObject is a type predicate — always safe to answer "false".
    f.insert("isSpreadsheetObject".into(), |_args| Ok(CfmlValue::Bool(false)));
    for name in SPREADSHEET_FN_NAMES {
        f.insert((*name).into(), fn_spreadsheet_disabled);
    }
}

/// Any spreadsheet function called in a build compiled WITHOUT `spreadsheet`
/// (e.g. the wasm targets).
#[cfg(not(feature = "spreadsheet"))]
fn fn_spreadsheet_disabled(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime(
        "Spreadsheet support is not available in this build (native/server only)".to_string(),
    ))
}

/// Any image function called in a build compiled WITHOUT `image_support`.
#[cfg(not(feature = "image_support"))]
fn fn_image_disabled(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime(
        "Image support was not compiled into this build (enable the 'image_support' feature)"
            .to_string(),
    ))
}

fn fn_is_binary(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Bool(matches!(args.first(), Some(CfmlValue::Binary(_)))))
}

fn fn_is_custom_function(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Bool(matches!(args.first(), Some(CfmlValue::Function(_)))))
}

fn fn_is_closure(args: Vec<CfmlValue>) -> CfmlResult {
    // Lucee parity: `isClosure()` is true for a closure/arrow-function
    // expression (`function(){}` / `()=>{}`) but false for a plain named UDF
    // or component method. At runtime an anonymous function expression is a
    // `CfmlValue::Function` (like every UDF) whose `captured_scope` is `Some`
    // — but so is every named UDF's, so that flag can't distinguish them. The
    // reliable signal is the compiler-synthesized name: closures compile to
    // `__closure_N` and arrow functions to `__arrow_N` (see cfml-codegen
    // `Expression::Closure`/`ArrowFunction`); named declarations keep their
    // real name. `DefineFunction` already keys behavior off the same `__`
    // prefix convention. (The legacy `CfmlValue::Closure` variant is also
    // honored for completeness.)
    //
    // This being wrong (always false) silently broke Preside/Sticker's
    // `Bundle.addAssets()`: its `if ( !isClosure(match) || match(path) )` guard
    // short-circuited on `!false`, so the `match` closure was never applied and
    // every file (and directory) got registered as an asset — clobbering the
    // core admin CSS/JS bundles and leaving the admin unstyled.
    let is = match args.first() {
        Some(CfmlValue::Closure(_)) => true,
        Some(CfmlValue::Function(f)) => {
            f.name.starts_with("__closure_") || f.name.starts_with("__arrow_")
        }
        _ => false,
    };
    Ok(CfmlValue::Bool(is))
}

fn fn_is_valid(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        let type_name = get_str(&args, 0).to_lowercase();
        let value = &args[1];
        match type_name.as_str() {
            // Lucee/ACF: isValid("string", x) is true only for SIMPLE values —
            // a struct/array/query/component is not a valid string (TestBox
            // notTypeOf("string", this)). Was unconditionally true.
            "string" => fn_is_simple_value(vec![value.clone()]),
            "numeric" | "float" | "double" => fn_is_numeric(vec![value.clone()]),
            "integer" => {
                let s = value.as_string();
                Ok(CfmlValue::Bool(s.trim().parse::<i64>().is_ok()))
            }
            "boolean" => fn_is_boolean(vec![value.clone()]),
            "date" | "datetime" => fn_is_date(vec![value.clone()]),
            "time" => {
                // Lucee: a bare time-of-day (6:15 PM, 18:15, 06:15:30) OR a
                // parseable date/datetime *string* ("2020-01-01" -> true) is a
                // valid time; "25:99"/"hello" are not. (Wheels validatesFormat
                // type="time".) A bare number ("1", "123", 0) is a valid *date*
                // serial but NOT a valid time in Lucee, so exclude numerics from
                // the date path — isValid("time","1") must be false.
                let s = value.as_string();
                let s = s.trim();
                let is_numeric = !s.is_empty() && s.parse::<f64>().is_ok();
                let ok = (!is_numeric && parse_cfml_date(s).is_some())
                    || ["%H:%M", "%H:%M:%S", "%I:%M %p", "%I:%M:%S %p"]
                        .iter()
                        .any(|fmt| NaiveTime::parse_from_str(s, fmt).is_ok());
                Ok(CfmlValue::Bool(ok))
            }
            "array" => fn_is_array(vec![value.clone()]),
            "struct" => fn_is_struct(vec![value.clone()]),
            "binary" => fn_is_binary(vec![value.clone()]),
            "component" | "object" | "class" => fn_is_object(vec![value.clone()]),
            "email" => {
                let s = value.as_string();
                Ok(CfmlValue::Bool(EMAIL_REGEX.is_match(&s)))
            }
            "url" => {
                let s = value.as_string().to_lowercase();
                Ok(CfmlValue::Bool(
                    s.starts_with("http://") || s.starts_with("https://") || s.starts_with("ftp://")
                ))
            }
            "query" => fn_is_query(vec![value.clone()]),
            "uuid" => {
                // CFML UUID format: 8-4-4-16 (35 chars total)
                let s = value.as_string();
                Ok(CfmlValue::Bool(UUID_REGEX.is_match(&s)))
            }
            "guid" => {
                // Standard GUID format: 8-4-4-4-12
                let s = value.as_string();
                Ok(CfmlValue::Bool(GUID_REGEX.is_match(&s)))
            }
            "variablename" => {
                // A legal CFML variable identifier: a letter/underscore start
                // followed by letters/digits/underscores. Mura/Masa's
                // onApplicationStart setup-detection gates on
                // `isValid('variableName', application.setupSubmitButton)`; without
                // this the type fell through to `false`, flipping setupComplete to
                // true on a fresh DB and skipping the setup wizard (which Lucee
                // shows) — booting straight into an unbuilt schema instead.
                let s = value.as_string();
                let mut chars = s.chars();
                let ok = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
                    && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
                Ok(CfmlValue::Bool(ok))
            }
            "range" => {
                // isValid("range", value, min, max)
                if args.len() >= 4 {
                    let n = value.as_string().parse::<f64>().unwrap_or(f64::NAN);
                    if n.is_nan() { return Ok(CfmlValue::Bool(false)); }
                    let min_val = get_float(&args, 2);
                    let max_val = get_float(&args, 3);
                    Ok(CfmlValue::Bool(n >= min_val && n <= max_val))
                } else {
                    Ok(CfmlValue::Bool(false))
                }
            }
            "regex" | "regular_expression" => {
                // isValid("regex", value, pattern) - check if value matches pattern.
                // Requires 3 arguments (matches Lucee: pattern is mandatory).
                if args.len() < 3 {
                    return Err(CfmlError::runtime(
                        "Invalid call of the function [isValid], first Argument [type] is invalid, for [regex] you have to define a pattern".to_string()
                    ));
                }
                let s = value.as_string();
                let pattern = get_str(&args, 2);
                match cached_regex(&pattern) {
                    Ok(re) => Ok(CfmlValue::Bool(re.is_match(&s))),
                    Err(_) => Ok(CfmlValue::Bool(false)),
                }
            }
            "creditcard" => {
                let s: String = value.as_string().chars().filter(|c| c.is_ascii_digit()).collect();
                if s.len() < 13 || s.len() > 19 { return Ok(CfmlValue::Bool(false)); }
                let mut sum = 0u32;
                let mut double = false;
                for c in s.chars().rev() {
                    let mut d = c.to_digit(10).unwrap_or(0);
                    if double { d *= 2; if d > 9 { d -= 9; } }
                    sum += d;
                    double = !double;
                }
                Ok(CfmlValue::Bool(sum % 10 == 0))
            }
            "zipcode" => {
                let s = value.as_string();
                Ok(CfmlValue::Bool(ZIPCODE_REGEX.is_match(&s)))
            }
            "telephone" | "phone" => {
                let digits: String = value.as_string().chars().filter(|c| c.is_ascii_digit()).collect();
                Ok(CfmlValue::Bool(digits.len() >= 10 && digits.len() <= 15))
            }
            "ssn" | "social_security_number" => {
                let s = value.as_string();
                Ok(CfmlValue::Bool(SSN_REGEX.is_match(&s)))
            }
            // Lucee/ACF: isValid("xml", x) / isValid("json", x) delegate to
            // IsXml/IsJson. Without these, the type name fell through to the
            // catch-all `false`, so `isValid("xml", <well-formed xml>)` was
            // always false even though IsXml(x) was true — TestBox's
            // `.toBeXML()` (→ isValid("xml", …)) failed on Wheels renderWith's
            // valid XML output (controller.renderingSpec). The `xml` arm is
            // gated on the `xml` feature (like IsXml itself, which pulls in
            // quick_xml); on builds without it (the wasm worker) `isValid("xml",…)`
            // falls through to the catch-all, matching the absence of IsXml there.
            #[cfg(feature = "xml")]
            "xml" => fn_is_xml(vec![value.clone()]),
            "json" => fn_is_json(vec![value.clone()]),
            _ => Ok(CfmlValue::Bool(false)),
        }
    } else {
        Ok(CfmlValue::Bool(false))
    }
}

/// Runtime helper emitted by the `cfparam`/`param` lowering to enforce the
/// `type` (and `min`/`max`/`pattern`) attribute. CFML validates the resulting
/// value's type and throws on mismatch — previously `type` was silently
/// dropped. Args: (value, type, name, min, max, pattern).
fn fn_cfparam_validate(args: Vec<CfmlValue>) -> CfmlResult {
    let value = args.first().cloned().unwrap_or(CfmlValue::Null);
    let type_name = get_str(&args, 1).to_lowercase();
    let name = get_str(&args, 2);
    // Types we know how to validate. Unknown / unsupported type names are
    // accepted (no-op) rather than wrongly rejected.
    const KNOWN: &[&str] = &[
        "string", "numeric", "float", "double", "integer", "boolean", "date",
        "array", "struct", "query", "email", "url", "uuid", "guid",
        "creditcard", "zipcode", "telephone", "phone", "ssn",
        "social_security_number", "range", "regex", "regular_expression",
    ];
    if type_name.is_empty() || type_name == "any" || !KNOWN.contains(&type_name.as_str()) {
        return Ok(CfmlValue::Null);
    }
    let result = match type_name.as_str() {
        "range" => {
            // isValid("range", value, min, max)
            let mut a = vec![CfmlValue::string("range"), value.clone()];
            if let Some(m) = args.get(3) {
                a.push(m.clone());
            }
            if let Some(m) = args.get(4) {
                a.push(m.clone());
            }
            fn_is_valid(a)?
        }
        "regex" | "regular_expression" => {
            // isValid("regex", value, pattern)
            let mut a = vec![CfmlValue::string("regex"), value.clone()];
            if let Some(p) = args.get(5) {
                a.push(p.clone());
            }
            fn_is_valid(a)?
        }
        _ => fn_is_valid(vec![CfmlValue::string(type_name.clone()), value.clone()])?,
    };
    if !matches!(result, CfmlValue::Bool(true)) {
        return Err(CfmlError::runtime(format!(
            "The value [{}] passed to parameter [{}] is not of type [{}].",
            value.as_string(),
            name,
            type_name
        )));
    }
    Ok(CfmlValue::Null)
}

// ===============================================
// CONVERSION FUNCTIONS
// ===============================================

fn fn_to_string(args: Vec<CfmlValue>) -> CfmlResult {
    match args.first() {
        Some(CfmlValue::Binary(bytes)) => {
            Ok(CfmlValue::string(String::from_utf8_lossy(bytes).to_string()))
        }
        // Lucee parity: `toString()` of a complex value throws
        // `Can't cast Complex Object Type [Struct] to String` (type `expression`)
        // rather than dumping it. Scalars/dates/binary coerce as before.
        Some(v) => Ok(CfmlValue::string(v.to_string_strict()?)),
        None => Ok(CfmlValue::string(String::new())),
    }
}

fn fn_to_numeric(args: Vec<CfmlValue>) -> CfmlResult {
    match args.first() {
        Some(CfmlValue::Int(i)) => Ok(CfmlValue::Int(*i)),
        Some(CfmlValue::Double(d)) => Ok(CfmlValue::Double(*d)),
        Some(CfmlValue::Bool(b)) => Ok(CfmlValue::Int(if *b { 1 } else { 0 })),
        Some(CfmlValue::String(s)) => {
            let trimmed = s.trim();
            if let Ok(i) = trimmed.parse::<i64>() {
                Ok(CfmlValue::Int(i))
            } else if let Ok(d) = trimmed.parse::<f64>() {
                Ok(CfmlValue::Double(d))
            } else {
                Err(CfmlError::runtime(format!("Cannot convert '{}' to a number", s)))
            }
        }
        _ => Err(CfmlError::runtime("Cannot convert value to a number".to_string())),
    }
}

fn fn_to_boolean(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Bool(args.first().map_or(false, |v| v.is_true())))
}

fn fn_val(args: Vec<CfmlValue>) -> CfmlResult {
    // val() extracts the leading numeric value from a string.
    // Matches Lucee: stops at 'E' (does NOT parse scientific notation),
    // booleans convert to 0 (treated as the strings "true"/"false").
    let s = get_str(&args, 0).trim().to_string();
    let mut num_str = String::new();
    let mut has_dot = false;
    let mut has_sign = false;
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_digit() {
            num_str.push(c);
        } else if c == '.' && !has_dot {
            has_dot = true;
            num_str.push(c);
        } else if (c == '-' || c == '+') && i == 0 {
            has_sign = true;
            // Parse handles '-' but not leading '+', drop the '+'
            if c == '-' {
                num_str.push(c);
            }
        } else {
            break;
        }
    }
    if num_str.is_empty() || num_str == "-" || num_str == "." {
        // If we only had a '+' sign, still return 0 (no digits)
        let _ = has_sign;
        return Ok(CfmlValue::Int(0));
    }
    if has_dot {
        Ok(CfmlValue::Double(num_str.parse().unwrap_or(0.0)))
    } else {
        Ok(CfmlValue::Int(num_str.parse().unwrap_or(0)))
    }
}

fn fn_int(args: Vec<CfmlValue>) -> CfmlResult {
    let n = get_float(&args, 0);
    Ok(CfmlValue::Int(n.floor() as i64))
}

fn fn_java_cast(args: Vec<CfmlValue>) -> CfmlResult {
    // Simplified javacast
    if args.len() >= 2 {
        let type_name = get_str(&args, 0).to_lowercase();
        match type_name.as_str() {
            "string" => Ok(CfmlValue::string(args[1].as_string())),
            "int" | "integer" | "long" => Ok(CfmlValue::Int(get_int(&args, 1))),
            "double" | "float" => Ok(CfmlValue::Double(get_float(&args, 1))),
            "boolean" => Ok(CfmlValue::Bool(args[1].is_true())),
            "null" => Ok(CfmlValue::Null),
            _ => Ok(args[1].clone()),
        }
    } else {
        Ok(CfmlValue::Null)
    }
}

// ===============================================
// MATH FUNCTIONS
// ===============================================

fn fn_abs(args: Vec<CfmlValue>) -> CfmlResult {
    match args.first() {
        Some(CfmlValue::Int(i)) => Ok(CfmlValue::Int(i.abs())),
        Some(CfmlValue::Double(d)) => Ok(CfmlValue::Double(d.abs())),
        _ => Ok(CfmlValue::Double(get_float(&args, 0).abs())),
    }
}

fn fn_ceiling(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Int(get_float(&args, 0).ceil() as i64))
}

fn fn_floor(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Int(get_float(&args, 0).floor() as i64))
}

fn fn_round(args: Vec<CfmlValue>) -> CfmlResult {
    // Lucee/CFML uses Java Math.round: half-up towards positive infinity.
    // Rust's f64::round() rounds half away from zero, so -1.5 -> -2.
    // For CFML compatibility, use floor(n + 0.5).
    let n = get_float(&args, 0);
    if args.len() >= 2 {
        let precision = get_int(&args, 1);
        let factor = 10f64.powi(precision as i32);
        Ok(CfmlValue::Double((n * factor + 0.5).floor() / factor))
    } else {
        Ok(CfmlValue::Int((n + 0.5).floor() as i64))
    }
}

fn fn_rand(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Double(cfml_random()))
}

fn fn_rand_range(args: Vec<CfmlValue>) -> CfmlResult {
    let min = get_int(&args, 0);
    let max = get_int(&args, 1);
    let range = (max - min + 1) as f64;
    let result = min + (cfml_random() * range).floor() as i64;
    Ok(CfmlValue::Int(result.min(max)))
}

fn fn_randomize(args: Vec<CfmlValue>) -> CfmlResult {
    let seed = get_float(&args, 0);
    // Seed the thread-local PRNG for deterministic rand() output
    let seed_bits = (seed.to_bits()).max(1); // ensure non-zero
    PRNG_STATE.with(|state| state.set(seed_bits));
    PRNG_SEEDED.with(|seeded| seeded.set(true));
    Ok(CfmlValue::Double(0.0))
}

fn fn_max(args: Vec<CfmlValue>) -> CfmlResult {
    let a = get_float(&args, 0);
    let b = get_float(&args, 1);
    Ok(CfmlValue::Double(a.max(b)))
}

fn fn_min(args: Vec<CfmlValue>) -> CfmlResult {
    let a = get_float(&args, 0);
    let b = get_float(&args, 1);
    Ok(CfmlValue::Double(a.min(b)))
}

fn fn_sqr(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Double(get_float(&args, 0).sqrt()))
}

fn fn_exp(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Double(get_float(&args, 0).exp()))
}

fn fn_log(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Double(get_float(&args, 0).ln()))
}

fn fn_log10(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Double(get_float(&args, 0).log10()))
}

fn fn_sin(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Double(get_float(&args, 0).sin()))
}

fn fn_cos(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Double(get_float(&args, 0).cos()))
}

fn fn_tan(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Double(get_float(&args, 0).tan()))
}

fn fn_asin(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Double(get_float(&args, 0).asin()))
}

fn fn_acos(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Double(get_float(&args, 0).acos()))
}

fn fn_atan(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Double(get_float(&args, 0).atan()))
}

fn fn_pi(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Double(std::f64::consts::PI))
}

fn fn_sgn(args: Vec<CfmlValue>) -> CfmlResult {
    let n = get_float(&args, 0);
    Ok(CfmlValue::Int(if n > 0.0 { 1 } else if n < 0.0 { -1 } else { 0 }))
}

fn fn_fix(args: Vec<CfmlValue>) -> CfmlResult {
    let n = get_float(&args, 0);
    Ok(CfmlValue::Int(n.trunc() as i64))
}

fn fn_pow(args: Vec<CfmlValue>) -> CfmlResult {
    let base = get_float(&args, 0);
    let exp = get_float(&args, 1);
    Ok(CfmlValue::Double(base.powf(exp)))
}

fn fn_bit_and(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Int(get_int(&args, 0) & get_int(&args, 1)))
}

fn fn_bit_or(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Int(get_int(&args, 0) | get_int(&args, 1)))
}

fn fn_bit_xor(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Int(get_int(&args, 0) ^ get_int(&args, 1)))
}

fn fn_bit_not(args: Vec<CfmlValue>) -> CfmlResult {
    let n = get_int(&args, 0) as i32;
    Ok(CfmlValue::Int((!n) as i64))
}

fn fn_bit_shln(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Int(get_int(&args, 0) << get_int(&args, 1)))
}

fn fn_bit_shrn(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Int(get_int(&args, 0) >> get_int(&args, 1)))
}

fn fn_bit_mask_read(args: Vec<CfmlValue>) -> CfmlResult {
    let number = get_int(&args, 0);
    let start = get_int(&args, 1);
    let length = get_int(&args, 2);
    Ok(CfmlValue::Int((number >> start) & ((1 << length) - 1)))
}

fn fn_bit_mask_set(args: Vec<CfmlValue>) -> CfmlResult {
    let number = get_int(&args, 0);
    let mask = get_int(&args, 1);
    let start = get_int(&args, 2);
    let length = get_int(&args, 3);
    let clear_mask = ((1i64 << length) - 1) << start;
    Ok(CfmlValue::Int((number & !clear_mask) | ((mask & ((1 << length) - 1)) << start)))
}

fn fn_bit_mask_clear(args: Vec<CfmlValue>) -> CfmlResult {
    let number = get_int(&args, 0);
    let start = get_int(&args, 1);
    let length = get_int(&args, 2);
    Ok(CfmlValue::Int(number & !(((1i64 << length) - 1) << start)))
}

// ===============================================
// DATE/TIME HELPERS
// ===============================================

/// Convert 2-digit year to 4-digit: 0-29 → 2000-2029, 30-99 → 1930-1999
fn short_year(y: i64) -> i64 {
    if y >= 0 && y <= 29 { 2000 + y }
    else if y >= 30 && y <= 99 { 1900 + y }
    else { y }
}

/// Days in a given month/year
fn days_in_month_calc(year: i32, month: u32) -> u32 {
    match month {
        1 => 31,
        2 => if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 { 29 } else { 28 },
        3 => 31, 4 => 30, 5 => 31, 6 => 30,
        7 => 31, 8 => 31, 9 => 30, 10 => 31, 11 => 30, 12 => 31,
        _ => 30,
    }
}

/// Add months to a NaiveDateTime, clamping day to valid range
fn add_months(dt: &NaiveDateTime, months: i64) -> Option<NaiveDateTime> {
    let total = (dt.year() as i64) * 12 + (dt.month0() as i64) + months;
    let new_year = total.div_euclid(12) as i32;
    let new_month = (total.rem_euclid(12) as u32) + 1;
    let max_day = days_in_month_calc(new_year, new_month);
    let new_day = dt.day().min(max_day);
    NaiveDate::from_ymd_opt(new_year, new_month, new_day)
        .and_then(|d| d.and_hms_opt(dt.hour(), dt.minute(), dt.second()))
}

/// Parse ODBC literal: {d '...'}, {t '...'}, {ts '...'}
fn parse_odbc_literal(s: &str) -> Option<NaiveDateTime> {
    let start = s.find('\'')?;
    let end = s.rfind('\'')?;
    if start >= end { return None; }
    let inner = &s[start+1..end];
    let lower = s.to_lowercase();
    if lower.starts_with("{ts ") {
        NaiveDateTime::parse_from_str(inner, "%Y-%m-%d %H:%M:%S").ok()
    } else if lower.starts_with("{d ") {
        NaiveDate::parse_from_str(inner, "%Y-%m-%d").ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
    } else if lower.starts_with("{t ") {
        NaiveTime::parse_from_str(inner, "%H:%M:%S").ok()
            .and_then(|t| NaiveDate::from_ymd_opt(2000, 1, 1).map(|d| d.and_time(t)))
    } else {
        None
    }
}

/// Central date parser: tries ODBC, ISO 8601, common US/EU formats, time-only, date serial
fn parse_cfml_date(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    if s.is_empty() { return None; }

    // ODBC literals
    if s.starts_with('{') {
        return parse_odbc_literal(s);
    }

    // DateTime formats (most specific first). `%.f` optionally consumes a
    // fractional-seconds component (".177"), so the fractional variants also
    // match values with no fraction.
    for fmt in &[
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%m/%d/%Y %H:%M:%S",
        "%m/%d/%Y %I:%M:%S %p",
        "%m/%d/%Y %I:%M %p",
        "%m/%d/%Y %H:%M",
        "%d %b %Y %H:%M:%S",
        "%b %d, %Y %H:%M:%S",
        "%B %d, %Y %H:%M:%S",
        "%d-%b-%Y %H:%M:%S",
        // Month-name forms without a comma (Lucee/ACF accept these) — e.g.
        // "January 1 1970 00:00", used by cbsecurity's JwtService epoch base.
        "%B %d %Y %H:%M:%S",
        "%b %d %Y %H:%M:%S",
        "%B %d %Y %H:%M",
        "%b %d %Y %H:%M",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt);
        }
    }

    // Date-only formats → midnight
    for fmt in &[
        "%Y-%m-%d",
        "%m/%d/%Y",
        "%m-%d-%Y",
        "%d %b %Y",
        "%b %d, %Y",
        "%B %d, %Y",
        "%d-%b-%Y",
        "%B %d %Y",
        "%b %d %Y",
    ] {
        if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
            return d.and_hms_opt(0, 0, 0);
        }
    }

    // Time-only → base date 2000-01-01
    for fmt in &["%H:%M:%S", "%I:%M:%S %p", "%H:%M"] {
        if let Ok(t) = NaiveTime::parse_from_str(s, fmt) {
            return NaiveDate::from_ymd_opt(2000, 1, 1).map(|d| d.and_time(t));
        }
    }

    // RFC 3339 / ISO 8601 with a timezone offset or 'Z' suffix
    // ("2026-06-10T07:20:42.177+00:00", "...Z"). Return the wall-clock fields
    // as written (naive); callers that need the absolute instant (timestamptz)
    // use `parse_cfml_datetime_utc`, which honours the offset instead.
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.naive_local());
    }

    // Date serial number (days since 1899-12-30, OLE Automation date)
    if let Ok(n) = s.parse::<f64>() {
        if n.is_finite() {
            let base = NaiveDate::from_ymd_opt(1899, 12, 30)?;
            let days = n.floor() as i64;
            let frac = n - n.floor();
            // Round to the nearest millisecond (NOT truncate) — the fraction is
            // a float, so 51 seconds can land at 50.9999s and truncate to 50.
            // Milliseconds also preserve sub-second precision through a
            // date -> serial -> date round-trip.
            let ms = (frac * 86_400_000.0).round() as i64;
            // try_days/try_milliseconds return None on overflow instead of
            // panicking ("TimeDelta::days out of bounds") — a bare epoch-millis
            // value like 1.7e12 parsed as a date serial would otherwise crash.
            return base.and_hms_opt(0, 0, 0)
                .and_then(|dt| chrono::Duration::try_days(days).and_then(|d| dt.checked_add_signed(d)))
                .and_then(|dt| chrono::Duration::try_milliseconds(ms).and_then(|d| dt.checked_add_signed(d)));
        }
    }

    None
}

/// Determines whether `m`/`mm` means month or minute
#[derive(Clone, Copy)]
enum FormatMode { Date, Time, DateTime }

fn month_name_full(m: u32) -> &'static str {
    match m {
        1 => "January", 2 => "February", 3 => "March", 4 => "April",
        5 => "May", 6 => "June", 7 => "July", 8 => "August",
        9 => "September", 10 => "October", 11 => "November", 12 => "December",
        _ => "",
    }
}
fn month_name_short(m: u32) -> &'static str {
    match m {
        1 => "Jan", 2 => "Feb", 3 => "Mar", 4 => "Apr",
        5 => "May", 6 => "Jun", 7 => "Jul", 8 => "Aug",
        9 => "Sep", 10 => "Oct", 11 => "Nov", 12 => "Dec",
        _ => "",
    }
}
fn day_name_full(w: chrono::Weekday) -> &'static str {
    match w {
        chrono::Weekday::Mon => "Monday", chrono::Weekday::Tue => "Tuesday",
        chrono::Weekday::Wed => "Wednesday", chrono::Weekday::Thu => "Thursday",
        chrono::Weekday::Fri => "Friday", chrono::Weekday::Sat => "Saturday",
        chrono::Weekday::Sun => "Sunday",
    }
}
fn day_name_short(w: chrono::Weekday) -> &'static str {
    match w {
        chrono::Weekday::Mon => "Mon", chrono::Weekday::Tue => "Tue",
        chrono::Weekday::Wed => "Wed", chrono::Weekday::Thu => "Thu",
        chrono::Weekday::Fri => "Fri", chrono::Weekday::Sat => "Sat",
        chrono::Weekday::Sun => "Sun",
    }
}
fn hour12(h: u32) -> u32 {
    match h % 12 { 0 => 12, other => other }
}

/// Resolve preset mask names into actual mask patterns
fn resolve_preset(mask: &str, mode: &FormatMode) -> String {
    let lower = mask.to_lowercase();
    match mode {
        FormatMode::Date => match lower.as_str() {
            "" => "dd-mmm-yy".into(),
            "short" => "m/d/yy".into(),
            "medium" => "mmm d, yyyy".into(),
            "long" => "mmmm d, yyyy".into(),
            "full" => "dddd, mmmm d, yyyy".into(),
            _ => mask.into(),
        },
        FormatMode::Time => match lower.as_str() {
            "" => "hh:mm tt".into(),
            "short" => "h:mm tt".into(),
            "medium" => "h:mm:ss tt".into(),
            "long" | "full" => "h:mm:ss tt".into(),
            _ => mask.into(),
        },
        FormatMode::DateTime => match lower.as_str() {
            "" => "dd-mmm-yyyy HH:nn:ss".into(),
            "short" => "m/d/yy h:nn tt".into(),
            "medium" => "mmm d, yyyy h:nn:ss tt".into(),
            "long" => "mmmm d, yyyy h:nn:ss tt".into(),
            "full" => "dddd, mmmm d, yyyy h:nn:ss tt".into(),
            _ => mask.into(),
        },
    }
}

/// Match a format token at position `pos` in the mask character array
fn match_format_token(chars: &[char], pos: usize, dt: &NaiveDateTime, mode: FormatMode) -> Option<(usize, String)> {
    let remaining = chars.len() - pos;
    // 4-char tokens
    if remaining >= 4 {
        let four: String = chars[pos..pos+4].iter().collect();
        match four.to_lowercase().as_str() {
            "dddd" => return Some((4, day_name_full(dt.weekday()).into())),
            "mmmm" => return Some((4, match mode {
                FormatMode::Time => format!("{:02}", dt.minute()),
                _ => month_name_full(dt.month()).into(),
            })),
            "yyyy" => return Some((4, format!("{:04}", dt.year()))),
            _ => {}
        }
    }
    // 3-char tokens
    if remaining >= 3 {
        let three: String = chars[pos..pos+3].iter().collect();
        match three.to_lowercase().as_str() {
            "ddd" => return Some((3, day_name_short(dt.weekday()).into())),
            "mmm" => return Some((3, match mode {
                FormatMode::Time => format!("{:02}", dt.minute()),
                _ => month_name_short(dt.month()).into(),
            })),
            _ => {}
        }
    }
    // 2-char tokens
    if remaining >= 2 {
        let two: String = chars[pos..pos+2].iter().collect();
        match two.as_str() {
            "dd" | "DD" => return Some((2, format!("{:02}", dt.day()))),
            "mm" | "MM" => return Some((2, match mode {
                FormatMode::Time => format!("{:02}", dt.minute()),
                _ => format!("{:02}", dt.month()),
            })),
            "yy" | "YY" => return Some((2, format!("{:02}", dt.year() % 100))),
            "HH" => return Some((2, format!("{:02}", dt.hour()))),
            "hh" => return Some((2, format!("{:02}", hour12(dt.hour())))),
            "nn" | "NN" => return Some((2, format!("{:02}", dt.minute()))),
            "ss" | "SS" => return Some((2, format!("{:02}", dt.second()))),
            "tt" | "TT" => return Some((2, if dt.hour() < 12 { "AM".into() } else { "PM".into() })),
            _ => {}
        }
    }
    // 1-char tokens
    if remaining >= 1 {
        match chars[pos] {
            'd' | 'D' => return Some((1, format!("{}", dt.day()))),
            'm' | 'M' => return Some((1, match mode {
                FormatMode::Time => format!("{}", dt.minute()),
                _ => format!("{}", dt.month()),
            })),
            'y' | 'Y' => return Some((1, format!("{:02}", dt.year() % 100))),
            'H' => return Some((1, format!("{}", dt.hour()))),
            'h' => return Some((1, format!("{}", hour12(dt.hour())))),
            'n' | 'N' => return Some((1, format!("{}", dt.minute()))),
            's' | 'S' => return Some((1, format!("{}", dt.second()))),
            't' | 'T' => return Some((1, if dt.hour() < 12 { "A".into() } else { "P".into() })),
            'l' | 'L' => return Some((1, "000".into())),
            _ => {}
        }
    }
    None
}

/// Format a NaiveDateTime using a CFML mask string
fn format_cfml_date(dt: &NaiveDateTime, mask: &str, mode: FormatMode) -> String {
    let resolved = match mask.to_lowercase().as_str() {
        "" | "short" | "medium" | "long" | "full" => resolve_preset(mask, &mode),
        _ => mask.to_string(),
    };
    let chars: Vec<char> = resolved.chars().collect();
    let mut result = String::new();
    let mut i = 0;
    while i < chars.len() {
        // Single-quoted segments are emitted verbatim (Java SimpleDateFormat
        // convention, also used by Lucee/ACF). `''` inside or outside a quoted
        // segment yields a literal apostrophe.
        if chars[i] == '\'' {
            if i + 1 < chars.len() && chars[i + 1] == '\'' {
                result.push('\'');
                i += 2;
                continue;
            }
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' {
                    if i + 1 < chars.len() && chars[i + 1] == '\'' {
                        result.push('\'');
                        i += 2;
                    } else {
                        i += 1;
                        break;
                    }
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            continue;
        }
        if let Some((len, replacement)) = match_format_token(&chars, i, dt, mode) {
            result.push_str(&replacement);
            i += len;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

// ===============================================
// DATE/TIME FUNCTIONS
// ===============================================

fn fn_now(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::string(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()))
}

fn fn_create_date(args: Vec<CfmlValue>) -> CfmlResult {
    let year = short_year(get_int(&args, 0));
    let month = get_int(&args, 1);
    let day = get_int(&args, 2);
    // Lucee/ACF treat createDate() as a midnight timestamp, not a date-only
    // value, so it compares equal to createDateTime(y,m,d,0,0,0) and to
    // DateAdd("d",1,...). Emit the full datetime representation to match.
    Ok(CfmlValue::string(format!("{:04}-{:02}-{:02} 00:00:00", year, month, day)))
}

fn fn_create_date_time(args: Vec<CfmlValue>) -> CfmlResult {
    let year = short_year(get_int(&args, 0));
    let month = get_int(&args, 1);
    let day = get_int(&args, 2);
    let hour = get_int(&args, 3);
    let minute = get_int(&args, 4);
    let second = get_int(&args, 5);
    Ok(CfmlValue::string(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hour, minute, second
    )))
}

fn fn_create_time(args: Vec<CfmlValue>) -> CfmlResult {
    let hour = get_int(&args, 0);
    let minute = get_int(&args, 1);
    let second = get_int(&args, 2);
    Ok(CfmlValue::string(format!("{:02}:{:02}:{:02}", hour, minute, second)))
}

fn fn_create_odbc_date(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    if let Some(dt) = parse_cfml_date(&s) {
        Ok(CfmlValue::string(format!("{{d '{}'}}", dt.format("%Y-%m-%d"))))
    } else {
        Ok(CfmlValue::string(format!("{{d '{}'}}", s)))
    }
}

fn fn_create_odbc_date_time(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    if let Some(dt) = parse_cfml_date(&s) {
        Ok(CfmlValue::string(format!("{{ts '{}'}}", dt.format("%Y-%m-%d %H:%M:%S"))))
    } else {
        Ok(CfmlValue::string(format!("{{ts '{}'}}", s)))
    }
}

fn fn_create_odbc_time(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    if let Some(dt) = parse_cfml_date(&s) {
        Ok(CfmlValue::string(format!("{{t '{}'}}", dt.format("%H:%M:%S"))))
    } else {
        Ok(CfmlValue::string(format!("{{t '{}'}}", s)))
    }
}

fn fn_date_add(args: Vec<CfmlValue>) -> CfmlResult {
    let part = get_str(&args, 0).to_lowercase();
    let number = get_int(&args, 1);
    let date_str = get_str(&args, 2);
    let dt = parse_cfml_date(&date_str)
        .ok_or_else(|| CfmlError::runtime(format!("Invalid date: {}", date_str)))?;

    let result: Option<NaiveDateTime> = match part.as_str() {
        "yyyy" => add_months(&dt, number * 12),
        "q" => add_months(&dt, number * 3),
        "m" => add_months(&dt, number),
        // try_* variants return None on overflow rather than panicking, so an
        // out-of-range delta surfaces as a clean "Date arithmetic overflow".
        "y" | "d" => chrono::Duration::try_days(number).and_then(|x| dt.checked_add_signed(x)),
        "w" => chrono::Duration::try_days(number).and_then(|x| dt.checked_add_signed(x)),
        "ww" => chrono::Duration::try_weeks(number).and_then(|x| dt.checked_add_signed(x)),
        "h" => chrono::Duration::try_hours(number).and_then(|x| dt.checked_add_signed(x)),
        "n" => chrono::Duration::try_minutes(number).and_then(|x| dt.checked_add_signed(x)),
        "s" => chrono::Duration::try_seconds(number).and_then(|x| dt.checked_add_signed(x)),
        "l" => chrono::Duration::try_milliseconds(number).and_then(|x| dt.checked_add_signed(x)),
        // Unknown datepart: Lucee/ACF throw an `expression` error rather than
        // silently no-op'ing. Frameworks rely on the throw — e.g. Preside's
        // RulesEngineTimePeriodService wraps dateAdd in try/catch and returns an
        // empty struct when the user-supplied unit is invalid.
        _ => {
            return Err(CfmlError::expression(format!(
                "invalid datepart identifier [{}] for function dateAdd",
                get_str(&args, 0)
            )));
        }
    };

    match result {
        Some(r) => Ok(CfmlValue::string(r.format("%Y-%m-%d %H:%M:%S").to_string())),
        None => Err(CfmlError::runtime("Date arithmetic overflow".into())),
    }
}

fn fn_date_diff(args: Vec<CfmlValue>) -> CfmlResult {
    let part = get_str(&args, 0).to_lowercase();
    let date1 = parse_cfml_date(&get_str(&args, 1))
        .ok_or_else(|| CfmlError::runtime("Invalid date1".into()))?;
    let date2 = parse_cfml_date(&get_str(&args, 2))
        .ok_or_else(|| CfmlError::runtime("Invalid date2".into()))?;

    let diff = match part.as_str() {
        "yyyy" => date2.year() as i64 - date1.year() as i64,
        "q" => {
            let q1 = (date1.year() as i64) * 4 + ((date1.month() as i64 - 1) / 3);
            let q2 = (date2.year() as i64) * 4 + ((date2.month() as i64 - 1) / 3);
            q2 - q1
        }
        "m" => {
            (date2.year() as i64 - date1.year() as i64) * 12
                + date2.month() as i64 - date1.month() as i64
        }
        "y" | "d" => (date2 - date1).num_days(),
        "w" => (date2 - date1).num_days() / 7,
        "ww" => (date2 - date1).num_days() / 7,
        "h" => (date2 - date1).num_hours(),
        "n" => (date2 - date1).num_minutes(),
        "s" => (date2 - date1).num_seconds(),
        "l" => (date2 - date1).num_milliseconds(),
        // Unknown datepart: Lucee/ACF throw an `expression` error (mirrors the
        // dateAdd fix) rather than silently returning 0.
        _ => {
            return Err(CfmlError::expression(format!(
                "invalid datepart identifier [{}] for function dateDiff",
                get_str(&args, 0)
            )));
        }
    };
    Ok(CfmlValue::Int(diff))
}

fn fn_date_format(args: Vec<CfmlValue>) -> CfmlResult {
    let date_str = get_str(&args, 0);
    // Lucee returns an empty string for a blank date in the FORMAT functions
    // (dateFormat/timeFormat/dateTimeFormat) — unlike dateAdd/year/parseDateTime,
    // which throw a cast error. Masa's admin content-edit + staging views pass
    // empty date columns straight to dateTimeFormat(); throwing 500'd the page.
    if date_str.trim().is_empty() {
        return Ok(CfmlValue::string(String::new()));
    }
    let mask = if args.len() > 1 { get_str(&args, 1) } else { String::new() };
    let dt = parse_cfml_date(&date_str)
        .ok_or_else(|| CfmlError::runtime(format!("Invalid date: {}", date_str)))?;
    Ok(CfmlValue::string(format_cfml_date(&dt, &mask, FormatMode::Date)))
}

fn fn_time_format(args: Vec<CfmlValue>) -> CfmlResult {
    let date_str = get_str(&args, 0);
    // Blank input → "" (Lucee parity; see fn_date_format).
    if date_str.trim().is_empty() {
        return Ok(CfmlValue::string(String::new()));
    }
    let mask = if args.len() > 1 { get_str(&args, 1) } else { String::new() };
    let dt = parse_cfml_date(&date_str)
        .ok_or_else(|| CfmlError::runtime(format!("Invalid date: {}", date_str)))?;
    Ok(CfmlValue::string(format_cfml_date(&dt, &mask, FormatMode::Time)))
}

fn fn_date_time_format(args: Vec<CfmlValue>) -> CfmlResult {
    let date_str = get_str(&args, 0);
    // Blank input → "" (Lucee parity; see fn_date_format).
    if date_str.trim().is_empty() {
        return Ok(CfmlValue::string(String::new()));
    }
    let mask = if args.len() > 1 { get_str(&args, 1) } else { String::new() };
    let dt = parse_cfml_date(&date_str)
        .ok_or_else(|| CfmlError::runtime(format!("Invalid date: {}", date_str)))?;
    Ok(CfmlValue::string(format_cfml_date(&dt, &mask, FormatMode::DateTime)))
}

fn fn_parse_date_time(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    match parse_cfml_date(&s) {
        Some(dt) => Ok(CfmlValue::string(dt.format("%Y-%m-%d %H:%M:%S").to_string())),
        None => Err(CfmlError::runtime(format!("Cannot parse date: {}", s))),
    }
}

fn fn_year(args: Vec<CfmlValue>) -> CfmlResult {
    let dt = parse_cfml_date(&get_str(&args, 0))
        .ok_or_else(|| CfmlError::runtime("Invalid date".into()))?;
    Ok(CfmlValue::Int(dt.year() as i64))
}

fn fn_month(args: Vec<CfmlValue>) -> CfmlResult {
    let dt = parse_cfml_date(&get_str(&args, 0))
        .ok_or_else(|| CfmlError::runtime("Invalid date".into()))?;
    Ok(CfmlValue::Int(dt.month() as i64))
}

fn fn_day(args: Vec<CfmlValue>) -> CfmlResult {
    let dt = parse_cfml_date(&get_str(&args, 0))
        .ok_or_else(|| CfmlError::runtime("Invalid date".into()))?;
    Ok(CfmlValue::Int(dt.day() as i64))
}

fn fn_hour(args: Vec<CfmlValue>) -> CfmlResult {
    let dt = parse_cfml_date(&get_str(&args, 0))
        .ok_or_else(|| CfmlError::runtime("Invalid date".into()))?;
    Ok(CfmlValue::Int(dt.hour() as i64))
}

fn fn_minute(args: Vec<CfmlValue>) -> CfmlResult {
    let dt = parse_cfml_date(&get_str(&args, 0))
        .ok_or_else(|| CfmlError::runtime("Invalid date".into()))?;
    Ok(CfmlValue::Int(dt.minute() as i64))
}

fn fn_second(args: Vec<CfmlValue>) -> CfmlResult {
    let dt = parse_cfml_date(&get_str(&args, 0))
        .ok_or_else(|| CfmlError::runtime("Invalid date".into()))?;
    Ok(CfmlValue::Int(dt.second() as i64))
}

/// CFML dayOfWeek: 1=Sunday, 2=Monday, ..., 7=Saturday
fn fn_day_of_week(args: Vec<CfmlValue>) -> CfmlResult {
    let dt = parse_cfml_date(&get_str(&args, 0))
        .ok_or_else(|| CfmlError::runtime("Invalid date".into()))?;
    Ok(CfmlValue::Int(dt.weekday().number_from_sunday() as i64))
}

fn fn_day_of_week_as_string(args: Vec<CfmlValue>) -> CfmlResult {
    let input = get_str(&args, 0);
    // Accept either a day number (1-7) or a date string
    let dow = if let Ok(n) = input.parse::<i64>() {
        n
    } else if let Some(dt) = parse_cfml_date(&input) {
        dt.weekday().number_from_sunday() as i64
    } else {
        return Ok(CfmlValue::string(String::new()));
    };
    let names = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    Ok(CfmlValue::string(names.get((dow - 1) as usize).unwrap_or(&"").to_string()))
}

fn fn_day_of_week_short_as_string(args: Vec<CfmlValue>) -> CfmlResult {
    let input = get_str(&args, 0);
    let dow = if let Ok(n) = input.parse::<i64>() {
        n
    } else if let Some(dt) = parse_cfml_date(&input) {
        dt.weekday().number_from_sunday() as i64
    } else {
        return Ok(CfmlValue::string(String::new()));
    };
    let names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    Ok(CfmlValue::string(names.get((dow - 1) as usize).unwrap_or(&"").to_string()))
}

fn fn_day_of_year(args: Vec<CfmlValue>) -> CfmlResult {
    let dt = parse_cfml_date(&get_str(&args, 0))
        .ok_or_else(|| CfmlError::runtime("Invalid date".into()))?;
    Ok(CfmlValue::Int(dt.ordinal() as i64))
}

fn fn_days_in_month(args: Vec<CfmlValue>) -> CfmlResult {
    let dt = parse_cfml_date(&get_str(&args, 0))
        .ok_or_else(|| CfmlError::runtime("Invalid date".into()))?;
    Ok(CfmlValue::Int(days_in_month_calc(dt.year(), dt.month()) as i64))
}

fn fn_days_in_year(args: Vec<CfmlValue>) -> CfmlResult {
    let dt = parse_cfml_date(&get_str(&args, 0))
        .ok_or_else(|| CfmlError::runtime("Invalid date".into()))?;
    let y = dt.year();
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    Ok(CfmlValue::Int(if leap { 366 } else { 365 }))
}

/// Returns the day-of-year for the first day of the date's month
fn fn_first_day_of_month(args: Vec<CfmlValue>) -> CfmlResult {
    let dt = parse_cfml_date(&get_str(&args, 0))
        .ok_or_else(|| CfmlError::runtime("Invalid date".into()))?;
    let first = NaiveDate::from_ymd_opt(dt.year(), dt.month(), 1)
        .ok_or_else(|| CfmlError::runtime("Invalid date".into()))?;
    Ok(CfmlValue::Int(first.ordinal() as i64))
}

fn fn_is_leap_year(args: Vec<CfmlValue>) -> CfmlResult {
    let input = get_str(&args, 0);
    // Accept either a year number or a date string
    let year = if let Ok(y) = input.parse::<i64>() {
        y
    } else if let Some(dt) = parse_cfml_date(&input) {
        dt.year() as i64
    } else {
        return Ok(CfmlValue::Bool(false));
    };
    Ok(CfmlValue::Bool((year % 4 == 0 && year % 100 != 0) || year % 400 == 0))
}

fn fn_month_as_string(args: Vec<CfmlValue>) -> CfmlResult {
    let input = get_str(&args, 0);
    let month = if let Ok(m) = input.parse::<i64>() {
        m
    } else if let Some(dt) = parse_cfml_date(&input) {
        dt.month() as i64
    } else {
        return Ok(CfmlValue::string(String::new()));
    };
    Ok(CfmlValue::string(month_name_full(month as u32).to_string()))
}

fn fn_month_short_as_string(args: Vec<CfmlValue>) -> CfmlResult {
    let input = get_str(&args, 0);
    let month = if let Ok(m) = input.parse::<i64>() {
        m
    } else if let Some(dt) = parse_cfml_date(&input) {
        dt.month() as i64
    } else {
        return Ok(CfmlValue::string(String::new()));
    };
    Ok(CfmlValue::string(month_name_short(month as u32).to_string()))
}

/// quarter(date) - returns 1-4 based on the month of the date
fn fn_quarter(args: Vec<CfmlValue>) -> CfmlResult {
    let input = get_str(&args, 0);
    let month = if let Ok(m) = input.parse::<i64>() {
        // Legacy: accept a raw month number
        m
    } else if let Some(dt) = parse_cfml_date(&input) {
        dt.month() as i64
    } else {
        return Ok(CfmlValue::Int(0));
    };
    Ok(CfmlValue::Int(((month - 1) / 3) + 1))
}

fn fn_week(args: Vec<CfmlValue>) -> CfmlResult {
    let dt = parse_cfml_date(&get_str(&args, 0))
        .ok_or_else(|| CfmlError::runtime("Invalid date".into()))?;
    // CFML week: ISO week number
    Ok(CfmlValue::Int(dt.iso_week().week() as i64))
}

/// datePart(datepart, date) - extracts the specified part from a date
fn fn_date_part(args: Vec<CfmlValue>) -> CfmlResult {
    let part = get_str(&args, 0).to_lowercase();
    let dt = parse_cfml_date(&get_str(&args, 1))
        .ok_or_else(|| CfmlError::runtime("Invalid date".into()))?;
    let val = match part.as_str() {
        "yyyy" => dt.year() as i64,
        "q" => ((dt.month() as i64 - 1) / 3) + 1,
        "m" => dt.month() as i64,
        "y" => dt.ordinal() as i64,
        "d" => dt.day() as i64,
        "w" => dt.weekday().number_from_sunday() as i64,
        "ww" => dt.iso_week().week() as i64,
        "h" => dt.hour() as i64,
        "n" => dt.minute() as i64,
        "s" => dt.second() as i64,
        "l" => 0, // milliseconds not tracked
        _ => return Err(CfmlError::runtime(format!("Invalid datepart: {}", part))),
    };
    Ok(CfmlValue::Int(val))
}

/// dateCompare(date1, date2 [, datePart]) - returns -1, 0, or 1
fn fn_date_compare(args: Vec<CfmlValue>) -> CfmlResult {
    let dt1 = parse_cfml_date(&get_str(&args, 0))
        .ok_or_else(|| CfmlError::runtime("Invalid date1".into()))?;
    let dt2 = parse_cfml_date(&get_str(&args, 1))
        .ok_or_else(|| CfmlError::runtime("Invalid date2".into()))?;
    let part = if args.len() > 2 { get_str(&args, 2).to_lowercase() } else { "s".into() };

    // Truncate precision based on datepart
    let (v1, v2) = match part.as_str() {
        "yyyy" => (
            NaiveDate::from_ymd_opt(dt1.year(), 1, 1).unwrap().and_hms_opt(0,0,0).unwrap(),
            NaiveDate::from_ymd_opt(dt2.year(), 1, 1).unwrap().and_hms_opt(0,0,0).unwrap(),
        ),
        "m" => (
            NaiveDate::from_ymd_opt(dt1.year(), dt1.month(), 1).unwrap().and_hms_opt(0,0,0).unwrap(),
            NaiveDate::from_ymd_opt(dt2.year(), dt2.month(), 1).unwrap().and_hms_opt(0,0,0).unwrap(),
        ),
        "d" => (
            dt1.date().and_hms_opt(0,0,0).unwrap(),
            dt2.date().and_hms_opt(0,0,0).unwrap(),
        ),
        "h" => (
            dt1.date().and_hms_opt(dt1.hour(), 0, 0).unwrap(),
            dt2.date().and_hms_opt(dt2.hour(), 0, 0).unwrap(),
        ),
        "n" => (
            dt1.date().and_hms_opt(dt1.hour(), dt1.minute(), 0).unwrap(),
            dt2.date().and_hms_opt(dt2.hour(), dt2.minute(), 0).unwrap(),
        ),
        _ => (dt1, dt2), // "s" or default: full precision
    };

    let cmp = if v1 < v2 { -1i64 } else if v1 > v2 { 1 } else { 0 };
    Ok(CfmlValue::Int(cmp))
}

fn fn_millisecond(args: Vec<CfmlValue>) -> CfmlResult {
    let dt = parse_cfml_date(&get_str(&args, 0))
        .ok_or_else(|| CfmlError::runtime("Invalid date".into()))?;
    let millis = dt.and_utc().timestamp_subsec_millis() as i64;
    Ok(CfmlValue::Int(millis))
}

fn fn_date_convert(args: Vec<CfmlValue>) -> CfmlResult {
    let conversion_type = get_str(&args, 0).to_lowercase();
    let date_str = get_str(&args, 1);
    let dt = parse_cfml_date(&date_str)
        .ok_or_else(|| CfmlError::runtime(format!("Invalid date: {}", date_str)))?;

    let result = match conversion_type.as_str() {
        "local2utc" => {
            let local_dt = Local.from_local_datetime(&dt)
                .single()
                .ok_or_else(|| CfmlError::runtime("Ambiguous or invalid local time".into()))?;
            local_dt.with_timezone(&Utc).naive_utc()
        }
        "utc2local" => {
            let utc_dt = Utc.from_utc_datetime(&dt);
            utc_dt.with_timezone(&Local).naive_local()
        }
        _ => return Err(CfmlError::runtime(
            format!("Invalid conversion type: {}. Use 'local2utc' or 'utc2local'.", conversion_type)
        )),
    };

    Ok(CfmlValue::string(result.format("%Y-%m-%d %H:%M:%S").to_string()))
}

fn fn_get_numeric_date(args: Vec<CfmlValue>) -> CfmlResult {
    let date_str = get_str(&args, 0);
    let dt = parse_cfml_date(&date_str)
        .ok_or_else(|| CfmlError::runtime(format!("Invalid date: {}", date_str)))?;

    let epoch = NaiveDate::from_ymd_opt(1899, 12, 30).unwrap().and_hms_opt(0, 0, 0).unwrap();
    let duration = dt - epoch;
    let days = duration.num_days() as f64;
    let remaining_secs = duration.num_seconds() - (duration.num_days() * 86400);
    let frac = remaining_secs as f64 / 86400.0;

    Ok(CfmlValue::Double(days + frac))
}

fn fn_get_http_time_string(args: Vec<CfmlValue>) -> CfmlResult {
    let date_str = get_str(&args, 0);
    let dt = parse_cfml_date(&date_str)
        .ok_or_else(|| CfmlError::runtime(format!("Invalid date: {}", date_str)))?;

    Ok(CfmlValue::string(dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string()))
}

fn fn_now_server(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::string(Local::now().format("%Y-%m-%d %H:%M:%S").to_string()))
}

fn fn_get_tick_count(args: Vec<CfmlValue>) -> CfmlResult {
    let unit = args.first()
        .and_then(|v| if let CfmlValue::String(s) = v { Some(s.to_lowercase()) } else { None })
        .unwrap_or_else(|| "milli".to_string());
    let val = match unit.as_str() {
        "nano" => cfml_common::clock::now_unix_nanos() as i64,
        "second" => cfml_common::clock::now_unix_secs() as i64,
        _ => cfml_common::clock::now_unix_millis() as i64,
    };
    Ok(CfmlValue::Int(val))
}

fn fn_get_function_called_name(_args: Vec<CfmlValue>) -> CfmlResult {
    // VM-intercepted — this stub only runs if the VM intercept misses (e.g.
    // called at the top level with no active call frame), where the called
    // name is undefined.
    Ok(CfmlValue::string(String::new()))
}

fn fn_get_function_list(_args: Vec<CfmlValue>) -> CfmlResult {
    // Return a struct of all registered builtin function names
    // Keys are function names, values are empty strings (matching CFML behavior)
    let mut result = ValueMap::default();
    for (name, _) in get_builtin_functions() {
        result.insert(name, CfmlValue::string(String::new()));
    }
    Ok(CfmlValue::strukt(result))
}

fn fn_get_context_root(_args: Vec<CfmlValue>) -> CfmlResult {
    // In a servlet context, returns the context root. For RustCFML, always "".
    Ok(CfmlValue::string(String::new()))
}

fn fn_is_in_thread(_args: Vec<CfmlValue>) -> CfmlResult {
    // Fallback only. The real isInThread() is VM-intercepted in `cfml-vm`
    // (reads the cfthread-body depth). This stub keeps the name registered for
    // resolution; off-VM it conservatively reports false.
    Ok(CfmlValue::Bool(false))
}

fn fn_get_page_context(_args: Vec<CfmlValue>) -> CfmlResult {
    // Fallback only. The real getPageContext() is VM-intercepted in
    // `cfml-vm` (`call_function` → `build_page_context_shim`) so it can read
    // the request's CGI scope and return a servlet bridge whose getRequest()/
    // getResponse() are method-faithful (getRequestURL, getMethod, setStatus,
    // …). This stub keeps the name registered as a builtin for resolution; it
    // is never reached when running on the VM.
    let mut ctx = ValueMap::default();
    ctx.insert("getRequest".to_string(), CfmlValue::Null);
    ctx.insert("getResponse".to_string(), CfmlValue::Null);
    Ok(CfmlValue::strukt(ctx))
}

// ===============================================
// LIST FUNCTIONS
// ===============================================

fn fn_list_new(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::string(String::new()))
}

fn fn_list_len(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    if list.is_empty() { return Ok(CfmlValue::Int(0)); }
    let delimiter = get_delimiter(&args, 1);
    Ok(CfmlValue::Int(cfml_list_split(&list, &delimiter).len() as i64))
}

fn fn_list_append(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let value = get_str(&args, 1);
    let delimiter = get_delimiter(&args, 2);
    if list.is_empty() {
        Ok(CfmlValue::string(value))
    } else {
        Ok(CfmlValue::string(format!("{}{}{}", list, delimiter, value)))
    }
}

fn fn_list_prepend(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let value = get_str(&args, 1);
    let delimiter = get_delimiter(&args, 2);
    if list.is_empty() {
        Ok(CfmlValue::string(value))
    } else {
        Ok(CfmlValue::string(format!("{}{}{}", value, delimiter, list)))
    }
}

fn fn_list_get_at(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let index = (get_int(&args, 1) as usize).saturating_sub(1);
    let delimiter = get_delimiter(&args, 2);
    let items = cfml_list_split(&list, &delimiter);
    Ok(CfmlValue::string(items.get(index).unwrap_or(&"").to_string()))
}

fn fn_list_set_at(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let index = get_int(&args, 1) as usize;
    let value = get_str(&args, 2);
    let delimiter = get_delimiter(&args, 3);
    let first_delim = delimiter.chars().next().unwrap_or(',').to_string();
    // CFML indexes by NON-EMPTY element (like ListLen) but PRESERVES empty fields.
    let mut items: Vec<String> =
        cfml_list_split_keep_empty(&list, &delimiter).iter().map(|s| s.to_string()).collect();
    if let Some(pos) = nth_nonempty_field_pos(
        &items.iter().map(|s| s.as_str()).collect::<Vec<_>>(), index,
    ) {
        items[pos] = value;
    }
    Ok(CfmlValue::string(items.join(&first_delim)))
}

fn fn_list_insert_at(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let index = get_int(&args, 1) as usize;
    let value = get_str(&args, 2);
    let delimiter = get_delimiter(&args, 3);
    let first_delim = delimiter.chars().next().unwrap_or(',').to_string();
    let mut items: Vec<String> =
        cfml_list_split_keep_empty(&list, &delimiter).iter().map(|s| s.to_string()).collect();
    let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
    // Insert BEFORE the Nth non-empty element; if N is past the last element,
    // append at the end (matches Lucee's append-on-overflow behavior).
    let pos = nth_nonempty_field_pos(&refs, index).unwrap_or(items.len());
    if pos <= items.len() {
        items.insert(pos, value);
    }
    Ok(CfmlValue::string(items.join(&first_delim)))
}

fn fn_list_delete_at(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let index = get_int(&args, 1) as usize;
    let delimiter = get_delimiter(&args, 2);
    let first_delim = delimiter.chars().next().unwrap_or(',').to_string();
    let mut items: Vec<String> =
        cfml_list_split_keep_empty(&list, &delimiter).iter().map(|s| s.to_string()).collect();
    let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
    if let Some(pos) = nth_nonempty_field_pos(&refs, index) {
        items.remove(pos);
    }
    Ok(CfmlValue::string(items.join(&first_delim)))
}

fn fn_list_find(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let value = get_str(&args, 1);
    let delimiter = get_delimiter(&args, 2);
    for (i, item) in cfml_list_split(&list, &delimiter).iter().enumerate() {
        if item.trim() == value { return Ok(CfmlValue::Int((i + 1) as i64)); }
    }
    Ok(CfmlValue::Int(0))
}

fn fn_list_find_no_case(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let value = get_str(&args, 1).to_lowercase();
    let delimiter = get_delimiter(&args, 2);
    for (i, item) in cfml_list_split(&list, &delimiter).iter().enumerate() {
        if item.trim().to_lowercase() == value { return Ok(CfmlValue::Int((i + 1) as i64)); }
    }
    Ok(CfmlValue::Int(0))
}

fn fn_list_contains(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let value = get_str(&args, 1);
    let delimiter = get_delimiter(&args, 2);
    for (i, item) in cfml_list_split(&list, &delimiter).iter().enumerate() {
        if item.trim().contains(&value) { return Ok(CfmlValue::Int((i + 1) as i64)); }
    }
    Ok(CfmlValue::Int(0))
}

fn fn_list_contains_no_case(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let value = get_str(&args, 1).to_lowercase();
    let delimiter = get_delimiter(&args, 2);
    for (i, item) in cfml_list_split(&list, &delimiter).iter().enumerate() {
        if item.trim().to_lowercase().contains(&value) { return Ok(CfmlValue::Int((i + 1) as i64)); }
    }
    Ok(CfmlValue::Int(0))
}

fn fn_list_sort(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let sort_type = if args.len() > 1 { get_str(&args, 1).to_lowercase() } else { "text".to_string() };
    let sort_order = if args.len() > 2 { get_str(&args, 2).to_lowercase() } else { "asc".to_string() };
    let delimiter = if args.len() > 3 { get_str(&args, 3) } else { ",".to_string() };
    let first_delim = delimiter.chars().next().unwrap_or(',').to_string();
    let mut items: Vec<String> = cfml_list_split(&list, &delimiter).iter().map(|s| s.trim().to_string()).collect();
    match sort_type.as_str() {
        "numeric" => {
            items.sort_by(|a, b| {
                let fa: f64 = a.parse().unwrap_or(0.0);
                let fb: f64 = b.parse().unwrap_or(0.0);
                fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        "textnocase" => {
            items.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        }
        _ => items.sort(), // "text" - case-sensitive
    }
    if sort_order == "desc" {
        items.reverse();
    }
    Ok(CfmlValue::string(items.join(&first_delim)))
}

fn fn_list_to_array(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let delimiter = get_delimiter(&args, 1);
    let include_empty = args.get(2).map(|v| v.is_true()).unwrap_or(false);
    // Lucee parity: an EMPTY delimiter splits the list into its individual
    // characters (this is specific to ListToArray — ListLen/ListFirst/ListGetAt
    // instead treat an empty delimiter as "no delimiter" = one element). With
    // includeEmptyFields=true, Lucee brackets the characters with a leading and
    // trailing empty element: "ab" -> ["","a","b",""], "" -> [""].
    if delimiter.is_empty() {
        let mut items: Vec<CfmlValue> = Vec::new();
        if include_empty {
            // "" -> [""] (a single empty element, not two).
            if list.is_empty() {
                return Ok(CfmlValue::array(vec![CfmlValue::string(String::new())]));
            }
            items.push(CfmlValue::string(String::new()));
            for ch in list.chars() {
                items.push(CfmlValue::string(ch.to_string()));
            }
            items.push(CfmlValue::string(String::new()));
        } else {
            for ch in list.chars() {
                items.push(CfmlValue::string(ch.to_string()));
            }
        }
        return Ok(CfmlValue::array(items));
    }
    if list.is_empty() {
        return Ok(CfmlValue::array(Vec::new()));
    }
    let items: Vec<CfmlValue> = if include_empty {
        cfml_list_split_keep_empty(&list, &delimiter).iter().map(|s| CfmlValue::string(s.to_string())).collect()
    } else {
        cfml_list_split(&list, &delimiter).iter().map(|s| CfmlValue::string(s.to_string())).collect()
    };
    Ok(CfmlValue::array(items))
}

fn fn_list_first(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let delimiter = get_delimiter(&args, 1);
    let items = cfml_list_split(&list, &delimiter);
    Ok(CfmlValue::string(items.first().unwrap_or(&"").to_string()))
}

fn fn_list_last(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let delimiter = get_delimiter(&args, 1);
    let items = cfml_list_split(&list, &delimiter);
    Ok(CfmlValue::string(items.last().unwrap_or(&"").to_string()))
}

fn fn_list_rest(args: Vec<CfmlValue>) -> CfmlResult {
    // Lucee/ACF/BoxLang parity: return the LITERAL substring of `list` from
    // the start of element 2 to the end — preserving interior/trailing empty
    // elements and the original delimiter chars. Empty-collapsing only
    // applies to the leading run of delimiters (so "/a/b/" treats "a" as
    // element 1) and to the run of delimiters that ends element 1.
    let list = get_str(&args, 0);
    let delimiter = get_delimiter(&args, 1);
    let is_delim = |c: char| delimiter.contains(c);
    let mut iter = list.char_indices().peekable();
    // Skip leading delimiter chars (collapse leading empty elements).
    while let Some(&(_, c)) = iter.peek() {
        if is_delim(c) {
            iter.next();
        } else {
            break;
        }
    }
    // Consume the first non-empty element.
    while let Some(&(_, c)) = iter.peek() {
        if is_delim(c) {
            break;
        }
        iter.next();
    }
    // Skip the run of delimiters that ends element 1.
    while let Some(&(_, c)) = iter.peek() {
        if is_delim(c) {
            iter.next();
        } else {
            break;
        }
    }
    let rest = match iter.peek() {
        Some(&(i, _)) => &list[i..],
        None => "",
    };
    Ok(CfmlValue::string(rest.to_string()))
}

fn fn_list_remove_duplicates(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let delimiter = get_delimiter(&args, 1);
    let ignore_case = args.get(2).map(|v| v.is_true()).unwrap_or(false);
    let first_delim = delimiter.chars().next().unwrap_or(',').to_string();
    let mut seen = Vec::new();
    let mut result = Vec::new();
    for item in cfml_list_split(&list, &delimiter) {
        let compare_key = if ignore_case { item.to_lowercase() } else { item.to_string() };
        if !seen.contains(&compare_key) {
            seen.push(compare_key);
            result.push(item.to_string());
        }
    }
    Ok(CfmlValue::string(result.join(&first_delim)))
}

fn fn_list_value_count(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let value = get_str(&args, 1);
    let delimiter = get_delimiter(&args, 2);
    let count = cfml_list_split(&list, &delimiter).iter().filter(|s| s.trim() == value).count();
    Ok(CfmlValue::Int(count as i64))
}

fn fn_list_value_count_no_case(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let value = get_str(&args, 1).to_lowercase();
    let delimiter = get_delimiter(&args, 2);
    let count = cfml_list_split(&list, &delimiter).iter().filter(|s| s.trim().to_lowercase() == value).count();
    Ok(CfmlValue::Int(count as i64))
}

fn fn_list_change_delims(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let new_delim = get_str(&args, 1);
    let old_delim = get_delimiter(&args, 2);
    Ok(CfmlValue::string(cfml_list_split(&list, &old_delim).join(&new_delim)))
}

fn fn_list_qualify(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let qualifier = get_str(&args, 1);
    let delimiter = get_delimiter(&args, 2);
    let first_delim = delimiter.chars().next().unwrap_or(',').to_string();
    let items: Vec<String> = cfml_list_split(&list, &delimiter).iter().map(|s| format!("{}{}{}", qualifier, s.trim(), qualifier)).collect();
    Ok(CfmlValue::string(items.join(&first_delim)))
}

fn fn_list_compact(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let delimiter = get_delimiter(&args, 1);
    let first_delim = delimiter.chars().next().unwrap_or(',').to_string();
    let items: Vec<&str> = cfml_list_split(&list, &delimiter);
    Ok(CfmlValue::string(items.join(&first_delim)))
}

fn fn_list_each(_args: Vec<CfmlValue>) -> CfmlResult {
    // Needs VM closure support
    Err(CfmlError::runtime("listEach() requires VM-level closure support".to_string()))
}

fn fn_list_map(_args: Vec<CfmlValue>) -> CfmlResult {
    // Needs VM closure support
    Err(CfmlError::runtime("listMap() requires VM-level closure support".to_string()))
}

fn fn_list_filter(_args: Vec<CfmlValue>) -> CfmlResult {
    // Needs VM closure support
    Err(CfmlError::runtime("listFilter() requires VM-level closure support".to_string()))
}

// ===============================================
// WEBSOCKET / REALTIME (stubs — VM-intercepted)
// ===============================================

/// Placeholder body for the realtime BIFs (`io`, `wsPublish`, ...). These are
/// always intercepted in the VM (which holds the connection registry on
/// ServerState), so this only runs if the engine reached a realtime BIF with no
/// VM context at all — an internal wiring error.
fn fn_ws_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime(
        "WebSocket realtime functions require a running server (no VM context)".to_string(),
    ))
}

// ===============================================
// JSON FUNCTIONS
// ===============================================

pub fn fn_serialize_json(args: Vec<CfmlValue>) -> CfmlResult {
    let mut visited: Vec<usize> = Vec::new();
    // serializeJSON(data [, serializeQueryByColumns] [, useSecureJSONPrefix]).
    // The second arg controls query layout: false (default) emits row-oriented
    // DATA (array of row arrays); true emits column-oriented DATA (a struct
    // keyed by uppercased column name) plus a ROWCOUNT. Matches Lucee 6.
    let by_columns = args.get(1).map(|v| v.is_true()).unwrap_or(false);
    let body = serialize_value(args.first().unwrap_or(&CfmlValue::Null), &mut visited, by_columns);
    let flags = security_flags();
    if flags.secure_json && !flags.secure_json_prefix.is_empty() {
        Ok(CfmlValue::string(format!("{}{}", flags.secure_json_prefix, body)))
    } else {
        Ok(CfmlValue::string(body))
    }
}

/// Serialize a value to JSON. `visited` tracks the backing-Arc pointers of the
/// containers currently on the recursion path: reference-typed arrays/structs
/// (and components, which materialise as marker-bearing structs) can alias and
/// form cycles — e.g. a TestBox mock holds `this.mockBox`, whose generator holds
/// the mock back. Without this guard such a cycle recurses until the native
/// stack overflows and aborts the whole process (uncatchable SIGABRT). On
/// revisiting a container we emit `null` to break the cycle, mirroring
/// `as_string_guarded`/`deep_copy_guarded`.
/// Escape a string for inclusion in a JSON string literal, per RFC 8259 §7.
/// Standard short escapes for `" \ \b \f \n \r \t`, and `\u00XX` for every other
/// control character (U+0000–U+001F). All other bytes (including non-ASCII
/// Unicode) pass through unchanged. Required so serializeJSON output containing a
/// control char (e.g. chr(7) BEL) is valid JSON that deserializeJSON — and any
/// RFC-conformant parser — accepts; previously the raw control byte was emitted
/// and the serializer's own parser rejected its output (GitHub #213). Also
/// escapes backslashes in struct keys / column names, which the old per-site
/// `"`-only replace missed.
fn json_escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn serialize_value(val: &CfmlValue, visited: &mut Vec<usize>, by_columns: bool) -> String {
    // Phase C.3 — Slice 4: a flyweight instance serializes its public DATA members
    // directly (no methods, no `__` filter — the data map is already clean, so
    // user `__`/`___` keys serialize like any other). `is_instance_backed()` is
    // const `false` in a default build, leaving the marker path (serialize_struct)
    // untouched.
    if let Some(comp) = val.as_component() {
        if comp.is_instance_backed() {
            // Cycle-guard on the INSTANCE's identity (its backing Arc), not the
            // materialised data struct — `instance_serialize_data()` allocates a
            // fresh struct each call, so a struct-ptr guard could never fire and a
            // self-/mutually-referential instance graph would recurse until the
            // native stack overflows (the flyweight re-opening of GH #178). Mirror
            // the Array/Struct arms below: revisit → "null".
            let id = comp.instance_identity_ptr();
            if let Some(p) = id {
                if visited.contains(&p) {
                    return "null".to_string();
                }
                visited.push(p);
            }
            let data = comp.instance_serialize_data();
            let items: Vec<String> = data
                .iter()
                .filter(|(_, v)| !matches!(v, CfmlValue::Function(_) | CfmlValue::Closure(_)))
                .map(|(k, v)| {
                    format!(
                        "\"{}\":{}",
                        json_escape_str(k),
                        serialize_value(v, visited, by_columns)
                    )
                })
                .collect();
            if id.is_some() {
                visited.pop();
            }
            return format!("{{{}}}", items.join(","));
        }
    }
    match val {
        CfmlValue::Null => "null".to_string(),
        CfmlValue::Bool(b) => b.to_string(),
        CfmlValue::Int(i) => i.to_string(),
        CfmlValue::Double(d) => d.to_string(),
        // A timespan serializes as its numeric (fractional-day) value, like Lucee.
        CfmlValue::TimeSpan(d) => d.to_string(),
        CfmlValue::String(s) => format!("\"{}\"", json_escape_str(s)),
        CfmlValue::Array(arr) => {
            let ptr = arr.backing_ptr();
            if visited.contains(&ptr) {
                return "null".to_string();
            }
            visited.push(ptr);
            let items: Vec<String> = arr.iter().map(|v| serialize_value(&v, visited, by_columns)).collect();
            visited.pop();
            format!("[{}]", items.join(","))
        }
        CfmlValue::Struct(s) => {
            let ptr = s.backing_ptr();
            if visited.contains(&ptr) {
                return "null".to_string();
            }
            visited.push(ptr);
            let out = serialize_struct(s, visited, by_columns);
            visited.pop();
            out
        }
        CfmlValue::Query(q) => {
            // Lucee/ACF serialize a query to a {COLUMNS, DATA} envelope (NOT an
            // array of row structs), so deserializeJSON can rebuild a native
            // Query. COLUMNS keeps the declared column case. Row-oriented DATA
            // (default) is an array of per-row arrays; column-oriented DATA
            // (serializeQueryByColumns=true) is a struct keyed by UPPERCASED
            // column name, and the envelope leads with ROWCOUNT. Verified vs
            // Lucee 6. See GH #231's sibling, GH #232.
            q.with_read(|d| {
                let row_count = d.row_count();
                let columns: Vec<String> = d
                    .columns
                    .iter()
                    .map(|c| format!("\"{}\"", json_escape_str(c)))
                    .collect();
                let columns_json = format!("[{}]", columns.join(","));
                if by_columns {
                    let cols: Vec<String> = d.columns.iter().enumerate().map(|(ci, col)| {
                        let vals: Vec<String> = (0..row_count)
                            .map(|r| serialize_value(&d.data[ci][r], visited, by_columns))
                            .collect();
                        format!("\"{}\":[{}]", json_escape_str(&col.to_uppercase()), vals.join(","))
                    }).collect();
                    format!(
                        "{{\"ROWCOUNT\":{},\"COLUMNS\":{},\"DATA\":{{{}}}}}",
                        row_count,
                        columns_json,
                        cols.join(",")
                    )
                } else {
                    let rows: Vec<String> = (0..row_count).map(|r| {
                        let fields: Vec<String> = d.columns.iter().enumerate()
                            .map(|(ci, _)| serialize_value(&d.data[ci][r], visited, by_columns))
                            .collect();
                        format!("[{}]", fields.join(","))
                    }).collect();
                    format!(
                        "{{\"COLUMNS\":{},\"DATA\":[{}]}}",
                        columns_json,
                        rows.join(",")
                    )
                }
            })
        }
        CfmlValue::NativeObject(obj) => {
            // Native Rust-backed objects don't have a JSON representation by
            // default. Emit a tagged marker rather than silently outputting
            // "null" — keeps the output visibly wrong if a caller forgets to
            // expose a Serializable method on their CfmlNative implementation.
            let name = obj.read().map(|g| g.class_name().to_string())
                .unwrap_or_else(|_| "poisoned".to_string());
            format!("\"<NativeObject:{}>\"", json_escape_str(&name))
        }
        CfmlValue::QueryColumn(..) => {
            // A bare query-column access (q.col) is a proxy standing in for its
            // first-row scalar in scalar contexts. Serializing a struct/array
            // holding a query cell must emit the value, not drop it to null
            // (Lucee/ACF/BoxLang treat a query cell as a simple value).
            serialize_value(val.query_column_scalar(), visited, by_columns)
        }
        _ => "null".to_string(),
    }
}

fn serialize_struct(s: &CfmlStruct, visited: &mut Vec<usize>, by_columns: bool) -> String {
    // A struct carrying CFC instance markers (`__variables` plus a
    // `this`/`__name` marker) is a component instance — this engine
    // materialises CFCs as marker-bearing structs. Lucee/ACF serialize
    // only a component's data members, never engine internals (`__*`),
    // the `this` scope, or its methods (UDFs). Filter those out so REST
    // serialization and framework model inspection see only data.
    let is_cfc = s.contains_key("__variables")
        && (s.contains_key("this") || s.contains_key("__name"));
    // An arguments-derived struct carries private sentinel markers.
    // structKeyList/Count/Exists/for-in already hide these; serializeJSON
    // must too, or a struct built via structAppend(s, arguments) leaks
    // them into the JSON (Lucee has no such keys).
    let is_args = s.contains_key("__arguments_scope");
    let mut items: Vec<String> = s
        .iter()
        .filter(|(k, _)| k.as_str() != cfml_common::dynamic::EMPTY_DEFAULT_SCOPE_MARKER)
        .filter(|(k, _)| {
            !is_args
                || (k.as_str() != "__arguments_scope"
                    && k.as_str() != "__arguments_params")
        })
        .filter(|(k, v)| {
            if !is_cfc {
                return true;
            }
            // Only EXACT engine-reserved keys are hidden; user/framework `__`/`___`
            // public data (FW/1 AOP `___orig`) is real data Lucee serializes.
            if cfml_common::component::is_reserved_component_key(k)
                || k.eq_ignore_ascii_case("this")
            {
                return false;
            }
            !matches!(v, CfmlValue::Function(_) | CfmlValue::Closure(_))
        })
        .map(|(k, v)| format!("\"{}\":{}", json_escape_str(&k), serialize_value(&v, visited, by_columns)))
        .collect();

    // For a CFC, accessor-`property` values that were never written to the
    // top-level `this` scope live only in the private `variables` backing — this
    // is the case for default-only properties (declared `default=` but never
    // set) and inherited ones. Lucee serializes these too; enumerating only
    // top-level keys drops them (GH #267). Pull each declared property's value
    // from `__variables` when it wasn't already emitted above.
    if is_cfc {
        let vars_val = s.get("__variables");
        if let (Some(props), Some(vars_val)) =
            (s.get("__properties").and_then(|v| v.as_array()), vars_val)
        {
            for prop in &props {
                let Some(pname) = prop
                    .as_cfml_struct()
                    .and_then(|ps| ps.get_ci("name"))
                    .map(|n| n.as_string())
                else {
                    continue;
                };
                // Already emitted from the top level? (case-insensitive)
                if s.contains_key_ci(&pname) {
                    continue;
                }
                if let Some(pv) = vars_val.get_ci(&pname) {
                    if matches!(pv, CfmlValue::Function(_) | CfmlValue::Closure(_)) {
                        continue;
                    }
                    items.push(format!(
                        "\"{}\":{}",
                        json_escape_str(&pname),
                        serialize_value(&pv, visited, by_columns)
                    ));
                }
            }
        }
    }
    format!("{{{}}}", items.join(","))
}

/// `Serialize(value)` — Lucee/ACF CFML-literal serialisation. Produces a string
/// that `Evaluate()` reads back into an equivalent value (the inverse pairing
/// Lucee documents). The output is a CFML expression literal, NOT JSON:
///   - strings are double-quoted with the embedded `"` doubled (`""`), the
///     CFML-literal escape — no backslash escaping; newlines/tabs/backslashes
///     are emitted verbatim;
///   - structs render as `{"key":value,...}` (keys always quoted);
///   - arrays as `[a,b,c]`;
///   - null as `nullValue()`, queries as `query("col":[...],...)`.
/// Unlike Lucee — which uppercases and reorders struct keys as a side effect of
/// its internal storage — RustCFML preserves key case and insertion order
/// engine-wide (ordered, case-preserving `IndexMap`), the same convention its
/// `serializeJSON` already follows. Both forms still round-trip via `evaluate`.
pub fn fn_serialize(args: Vec<CfmlValue>) -> CfmlResult {
    let mut visited: Vec<usize> = Vec::new();
    let body = serialize_cfml_value(args.first().unwrap_or(&CfmlValue::Null), &mut visited);
    Ok(CfmlValue::string(body))
}

/// Escape a string for the CFML double-quoted literal form: only `"` is special
/// and is escaped by doubling it. Everything else is literal.
fn cfml_literal_string(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn serialize_cfml_value(val: &CfmlValue, visited: &mut Vec<usize>) -> String {
    // Flyweight component: serialize its DATA members as a struct literal (mirrors
    // the marker path + `serializeJSON`). Without this an Instance missed every arm
    // and rendered as `nullValue()`.
    if let Some(comp) = val.as_component().filter(|c| c.is_instance_backed()) {
        // Cycle-guard on the INSTANCE's identity (its backing Arc): the fresh
        // `instance_serialize_data()` struct gets a new backing Arc each call, so
        // the Struct arm's `backing_ptr` guard below never catches an instance
        // cycle — key on the instance itself, or a self-/mutually-referential
        // graph overflows the native stack (flyweight re-opening of GH #178).
        let s = CfmlValue::strukt(comp.instance_serialize_data());
        if let Some(p) = comp.instance_identity_ptr() {
            if visited.contains(&p) {
                return "nullValue()".to_string();
            }
            visited.push(p);
            let out = serialize_cfml_value(&s, visited);
            visited.pop();
            return out;
        }
        return serialize_cfml_value(&s, visited);
    }
    match val {
        CfmlValue::Null => "nullValue()".to_string(),
        CfmlValue::Bool(b) => b.to_string(),
        CfmlValue::Int(i) => i.to_string(),
        CfmlValue::Double(d) => d.to_string(),
        // A timespan serializes as its numeric (fractional-day) value, like Lucee.
        CfmlValue::TimeSpan(d) => d.to_string(),
        CfmlValue::String(s) => cfml_literal_string(s),
        CfmlValue::Array(arr) => {
            let ptr = arr.backing_ptr();
            if visited.contains(&ptr) {
                return "nullValue()".to_string();
            }
            visited.push(ptr);
            let items: Vec<String> = arr.iter().map(|v| serialize_cfml_value(&v, visited)).collect();
            visited.pop();
            format!("[{}]", items.join(","))
        }
        CfmlValue::Struct(s) => {
            let ptr = s.backing_ptr();
            if visited.contains(&ptr) {
                return "nullValue()".to_string();
            }
            visited.push(ptr);
            let out = serialize_cfml_struct(s, visited);
            visited.pop();
            out
        }
        CfmlValue::Query(q) => {
            q.with_read(|d| {
                let cols: Vec<String> = d.columns.iter().enumerate().map(|(ci, col)| {
                    let vals: Vec<String> = d.data[ci].iter()
                        .map(|v| serialize_cfml_value(v, visited)).collect();
                    format!("{}:[{}]", cfml_literal_string(col), vals.join(","))
                }).collect();
                format!("query({})", cols.join(","))
            })
        }
        CfmlValue::QueryColumn(..) => serialize_cfml_value(val.query_column_scalar(), visited),
        CfmlValue::NativeObject(obj) => {
            let name = obj.read().map(|g| g.class_name().to_string())
                .unwrap_or_else(|_| "poisoned".to_string());
            cfml_literal_string(&format!("<NativeObject:{}>", name))
        }
        _ => "nullValue()".to_string(),
    }
}

fn serialize_cfml_struct(s: &CfmlStruct, visited: &mut Vec<usize>) -> String {
    // Mirror serialize_struct's CFC/arguments filtering so component instances
    // serialize only their data members (never engine internals or methods).
    let is_cfc = s.contains_key("__variables")
        && (s.contains_key("this") || s.contains_key("__name"));
    let is_args = s.contains_key("__arguments_scope");
    let items: Vec<String> = s
        .iter()
        .filter(|(k, _)| k.as_str() != cfml_common::dynamic::EMPTY_DEFAULT_SCOPE_MARKER)
        .filter(|(k, _)| {
            !is_args
                || (k.as_str() != "__arguments_scope"
                    && k.as_str() != "__arguments_params")
        })
        .filter(|(k, v)| {
            if !is_cfc {
                return true;
            }
            // Only EXACT engine-reserved keys are hidden; user/framework `__`/`___`
            // public data (FW/1 AOP `___orig`) is real data Lucee serializes.
            if cfml_common::component::is_reserved_component_key(k)
                || k.eq_ignore_ascii_case("this")
            {
                return false;
            }
            !matches!(v, CfmlValue::Function(_) | CfmlValue::Closure(_))
        })
        .map(|(k, v)| format!("{}:{}", cfml_literal_string(&k), serialize_cfml_value(&v, visited)))
        .collect();
    format!("{{{}}}", items.join(","))
}

pub fn fn_deserialize_json(args: Vec<CfmlValue>) -> CfmlResult {
    let json = get_str(&args, 0);
    // deserializeJSON(json [, strictMapping]). strictMapping defaults to true.
    // When false, a {COLUMNS, DATA} object is reconstructed into a native Query
    // (the inverse of serializeJSON(query, …)) — matching Lucee/ACF. GH #232.
    let strict = args.get(1).map(|v| v.is_true()).unwrap_or(true);
    let value = match serde_json::from_str::<serde_json::Value>(&json) {
        Ok(value) => value,
        // Strict RFC-8259 parsing failed. Lucee/ACF `deserializeJSON` is lenient —
        // it accepts unquoted object keys, single-quoted strings/keys, trailing
        // commas, and `//` / `/* */` comments. Fall back to a lenient parse so we
        // match the reference engine instead of rejecting valid-per-CFML input.
        // On lenient failure, surface the original strict error text (Lucee still
        // rejects genuinely malformed JSON such as `{a:1 b:2}`).
        Err(strict_err) => parse_lenient_json(&json)
            .map_err(|_| CfmlError::runtime(format!("Invalid JSON: {}", strict_err)))?,
    };
    Ok(serde_json_to_cfml_strict(value, strict))
}

/// Lenient (Lucee/ACF-compatible) JSON parser used only as a fallback when strict
/// `serde_json` parsing fails. Matches Lucee 7's `deserializeJSON` leniency set:
/// unquoted object keys, single-quoted strings/keys, trailing commas, and `//`
/// line + `/* */` block comments. Commas between members are still required
/// (Lucee rejects `{a:1 b:2}`), so it is not arbitrarily permissive.
fn parse_lenient_json(input: &str) -> Result<serde_json::Value, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut pos = 0usize;
    let value = parse_lenient_value(&chars, &mut pos)?;
    skip_ws_comments(&chars, &mut pos);
    if pos != chars.len() {
        return Err(format!("unexpected trailing characters at column {}", pos + 1));
    }
    Ok(value)
}

fn skip_ws_comments(chars: &[char], pos: &mut usize) {
    loop {
        while *pos < chars.len() && chars[*pos].is_whitespace() {
            *pos += 1;
        }
        if *pos + 1 < chars.len() && chars[*pos] == '/' && chars[*pos + 1] == '/' {
            *pos += 2;
            while *pos < chars.len() && chars[*pos] != '\n' {
                *pos += 1;
            }
        } else if *pos + 1 < chars.len() && chars[*pos] == '/' && chars[*pos + 1] == '*' {
            *pos += 2;
            while *pos + 1 < chars.len() && !(chars[*pos] == '*' && chars[*pos + 1] == '/') {
                *pos += 1;
            }
            *pos = (*pos + 2).min(chars.len());
        } else {
            break;
        }
    }
}

fn parse_lenient_value(chars: &[char], pos: &mut usize) -> Result<serde_json::Value, String> {
    skip_ws_comments(chars, pos);
    if *pos >= chars.len() {
        return Err("unexpected end of JSON input".to_string());
    }
    match chars[*pos] {
        '{' => parse_lenient_object(chars, pos),
        '[' => parse_lenient_array(chars, pos),
        '"' | '\'' => Ok(serde_json::Value::String(parse_lenient_string(chars, pos)?)),
        _ => parse_lenient_literal(chars, pos),
    }
}

fn parse_lenient_object(chars: &[char], pos: &mut usize) -> Result<serde_json::Value, String> {
    *pos += 1; // consume '{'
    let mut map = serde_json::Map::new();
    loop {
        skip_ws_comments(chars, pos);
        if *pos >= chars.len() {
            return Err("unterminated object".to_string());
        }
        if chars[*pos] == '}' {
            *pos += 1;
            break;
        }
        let key = if chars[*pos] == '"' || chars[*pos] == '\'' {
            parse_lenient_string(chars, pos)?
        } else {
            parse_lenient_bare_key(chars, pos)?
        };
        skip_ws_comments(chars, pos);
        if *pos >= chars.len() || chars[*pos] != ':' {
            return Err(format!("expected ':' after key at column {}", *pos + 1));
        }
        *pos += 1; // consume ':'
        let val = parse_lenient_value(chars, pos)?;
        map.insert(key, val);
        skip_ws_comments(chars, pos);
        if *pos >= chars.len() {
            return Err("unterminated object".to_string());
        }
        match chars[*pos] {
            ',' => {
                *pos += 1;
            }
            '}' => {
                *pos += 1;
                break;
            }
            _ => return Err(format!("expected ',' or '}}' at column {}", *pos + 1)),
        }
    }
    Ok(serde_json::Value::Object(map))
}

fn parse_lenient_array(chars: &[char], pos: &mut usize) -> Result<serde_json::Value, String> {
    *pos += 1; // consume '['
    let mut arr = Vec::new();
    loop {
        skip_ws_comments(chars, pos);
        if *pos >= chars.len() {
            return Err("unterminated array".to_string());
        }
        if chars[*pos] == ']' {
            *pos += 1;
            break;
        }
        arr.push(parse_lenient_value(chars, pos)?);
        skip_ws_comments(chars, pos);
        if *pos >= chars.len() {
            return Err("unterminated array".to_string());
        }
        match chars[*pos] {
            ',' => {
                *pos += 1;
            }
            ']' => {
                *pos += 1;
                break;
            }
            _ => return Err(format!("expected ',' or ']' at column {}", *pos + 1)),
        }
    }
    Ok(serde_json::Value::Array(arr))
}

fn parse_lenient_string(chars: &[char], pos: &mut usize) -> Result<String, String> {
    let quote = chars[*pos];
    *pos += 1; // consume opening quote
    let mut s = String::new();
    while *pos < chars.len() {
        let c = chars[*pos];
        if c == quote {
            *pos += 1;
            return Ok(s);
        }
        if c == '\\' {
            *pos += 1;
            if *pos >= chars.len() {
                break;
            }
            match chars[*pos] {
                '"' => s.push('"'),
                '\'' => s.push('\''),
                '\\' => s.push('\\'),
                '/' => s.push('/'),
                'n' => s.push('\n'),
                'r' => s.push('\r'),
                't' => s.push('\t'),
                'b' => s.push('\u{0008}'),
                'f' => s.push('\u{000C}'),
                'u' => {
                    let code = parse_lenient_hex4(chars, pos)?;
                    // Combine a UTF-16 surrogate pair when present.
                    if (0xD800..=0xDBFF).contains(&code)
                        && *pos + 2 < chars.len()
                        && chars[*pos + 1] == '\\'
                        && chars[*pos + 2] == 'u'
                    {
                        *pos += 2; // move onto the second '\uXXXX'
                        let low = parse_lenient_hex4(chars, pos)?;
                        let combined =
                            0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                        if let Some(ch) = char::from_u32(combined) {
                            s.push(ch);
                        }
                    } else if let Some(ch) = char::from_u32(code) {
                        s.push(ch);
                    }
                }
                other => s.push(other),
            }
            *pos += 1;
        } else {
            s.push(c);
            *pos += 1;
        }
    }
    Err("unterminated string".to_string())
}

/// Reads the four hex digits of a `\uXXXX` escape. `*pos` points at the `u`;
/// on success it points at the last hex digit consumed.
fn parse_lenient_hex4(chars: &[char], pos: &mut usize) -> Result<u32, String> {
    let mut code = 0u32;
    for _ in 0..4 {
        *pos += 1;
        if *pos >= chars.len() {
            return Err("invalid \\u escape".to_string());
        }
        let h = chars[*pos]
            .to_digit(16)
            .ok_or_else(|| "invalid \\u escape".to_string())?;
        code = code * 16 + h;
    }
    Ok(code)
}

fn parse_lenient_bare_key(chars: &[char], pos: &mut usize) -> Result<String, String> {
    let start = *pos;
    while *pos < chars.len() {
        let c = chars[*pos];
        if c.is_alphanumeric() || c == '_' || c == '$' {
            *pos += 1;
        } else {
            break;
        }
    }
    if *pos == start {
        return Err(format!("expected object key at column {}", start + 1));
    }
    Ok(chars[start..*pos].iter().collect())
}

fn parse_lenient_literal(chars: &[char], pos: &mut usize) -> Result<serde_json::Value, String> {
    let start = *pos;
    while *pos < chars.len() {
        let c = chars[*pos];
        if c.is_whitespace() || matches!(c, ',' | '}' | ']' | ':') {
            break;
        }
        if c == '/' && *pos + 1 < chars.len() && (chars[*pos + 1] == '/' || chars[*pos + 1] == '*')
        {
            break;
        }
        *pos += 1;
    }
    let token: String = chars[start..*pos].iter().collect();
    match token.as_str() {
        "true" => Ok(serde_json::Value::Bool(true)),
        "false" => Ok(serde_json::Value::Bool(false)),
        "null" => Ok(serde_json::Value::Null),
        _ => {
            if let Ok(i) = token.parse::<i64>() {
                Ok(serde_json::Value::Number(i.into()))
            } else if let Ok(f) = token.parse::<f64>() {
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .ok_or_else(|| format!("invalid number '{}'", token))
            } else {
                Err(format!("unexpected token '{}' at column {}", token, start + 1))
            }
        }
    }
}

/// Try to reconstruct a native Query from a deserialized JSON object with the
/// Lucee/ACF `{COLUMNS:[...], DATA:...}` envelope. DATA is either row-oriented
/// (an array of per-row arrays) or column-oriented (a struct keyed by the
/// UPPERCASED column name -> array of that column's values). Returns None when
/// the object isn't query-shaped, so the caller falls back to a plain struct.
fn try_query_from_json_object(
    obj: &serde_json::Map<String, serde_json::Value>,
    strict: bool,
) -> Option<CfmlValue> {
    let find = |name: &str| obj.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v);
    let columns_val = find("COLUMNS")?;
    let data_val = find("DATA")?;
    let columns: Vec<String> = match columns_val {
        serde_json::Value::Array(a) => a
            .iter()
            .map(|v| v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string()))
            .collect(),
        _ => return None,
    };
    let mut rows: Vec<ValueMap> = Vec::new();
    match data_val {
        // Row-oriented: [[c0,c1,...], ...]
        serde_json::Value::Array(data_rows) => {
            for row in data_rows {
                let cells = match row {
                    serde_json::Value::Array(cells) => cells,
                    _ => return None,
                };
                let mut r = ValueMap::default();
                for (i, col) in columns.iter().enumerate() {
                    let cell = cells.get(i).cloned().unwrap_or(serde_json::Value::Null);
                    r.insert(col.clone(), serde_json_to_cfml_strict(cell, strict));
                }
                rows.push(r);
            }
        }
        // Column-oriented: {"COL":[v0,v1,...], ...}, keys uppercased by serialize.
        serde_json::Value::Object(cols) => {
            let col_arrays: Vec<&Vec<serde_json::Value>> = columns
                .iter()
                .map(|c| {
                    cols.iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(c))
                        .and_then(|(_, v)| v.as_array())
                })
                .collect::<Option<Vec<_>>>()?;
            let row_count = col_arrays.iter().map(|a| a.len()).max().unwrap_or(0);
            for ri in 0..row_count {
                let mut r = ValueMap::default();
                for (ci, col) in columns.iter().enumerate() {
                    let cell = col_arrays[ci].get(ri).cloned().unwrap_or(serde_json::Value::Null);
                    r.insert(col.clone(), serde_json_to_cfml_strict(cell, strict));
                }
                rows.push(r);
            }
        }
        _ => return None,
    }
    Some(CfmlValue::Query(CfmlQuery::from_parts(columns, rows)))
}

fn serde_json_to_cfml_strict(value: serde_json::Value, strict: bool) -> CfmlValue {
    match value {
        serde_json::Value::Null => CfmlValue::Null,
        serde_json::Value::Bool(b) => CfmlValue::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                CfmlValue::Int(i)
            } else if let Some(f) = n.as_f64() {
                CfmlValue::Double(f)
            } else {
                CfmlValue::Int(0)
            }
        }
        serde_json::Value::String(s) => CfmlValue::string(s),
        serde_json::Value::Array(arr) => {
            CfmlValue::array(arr.into_iter().map(|v| serde_json_to_cfml_strict(v, strict)).collect())
        }
        serde_json::Value::Object(obj) => {
            // Non-strict mapping reconstructs a native Query from the {COLUMNS,
            // DATA} envelope (Lucee/ACF query-JSON round trip); strict mapping
            // (the default) always keeps it a struct.
            if !strict {
                if let Some(q) = try_query_from_json_object(&obj, strict) {
                    return q;
                }
            }
            let mut map = ValueMap::default();
            for (k, v) in obj {
                map.insert(k, serde_json_to_cfml_strict(v, strict));
            }
            CfmlValue::strukt(map)
        }
    }
}

fn fn_is_json(args: Vec<CfmlValue>) -> CfmlResult {
    // Only a simple value can be JSON text. Lucee/ACF answer `false` for any
    // complex argument rather than throwing — and crucially, never coerce it:
    // our lenient `as_string` unwraps a one-member struct to that member's
    // string form, so `isJSON({ msg = "true" })` used to be true (GH #289).
    let arg = args.first().map(|v| v.query_column_scalar());
    match arg {
        Some(
            CfmlValue::Bool(_)
            | CfmlValue::Int(_)
            | CfmlValue::Double(_)
            | CfmlValue::TimeSpan(_)
            | CfmlValue::String(_),
        ) => {}
        _ => return Ok(CfmlValue::Bool(false)),
    }
    let s = get_str(&args, 0);
    // Match deserializeJSON's leniency: Lucee's isJSON also accepts unquoted
    // keys, single quotes, trailing commas and comments.
    let valid = serde_json::from_str::<serde_json::Value>(&s).is_ok()
        || parse_lenient_json(&s).is_ok();
    Ok(CfmlValue::Bool(valid))
}

// ===============================================
// QUERY FUNCTIONS
// ===============================================

fn fn_query_new(args: Vec<CfmlValue>) -> CfmlResult {
    if args.is_empty() {
        return Ok(CfmlValue::Query(CfmlQuery::new(Vec::new())));
    }
    // queryNew("col1,col2") or queryNew(["col1","col2"])
    let columns: Vec<String> = match &args[0] {
        // queryNew("") yields ZERO columns (Lucee), not one empty-named column.
        CfmlValue::String(s) => s
            .split(',')
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect(),
        CfmlValue::Array(arr) => arr.iter().map(|v| v.as_string()).collect(),
        _ => Vec::new(),
    };
    let mut rows: Vec<ValueMap> = Vec::new();
    // 3rd arg: initial data as array of arrays, array of structs, or a flat
    // array of scalars.
    if args.len() >= 3 {
        if let CfmlValue::Array(data_rows) = &args[2] {
            let data: Vec<CfmlValue> = data_rows.snapshot();
            // A FLAT array of scalars (no inner arrays/structs) is chunked by
            // column count into rows — Lucee semantics: queryNew("a,b","..",
            // [1,2,3,4]) yields two rows {a:1,b:2},{a:3,b:4}, and the common
            // single-column case ([1,2,3] over one column) still yields one row
            // per value. A single leftover chunk fills the leading columns.
            let flat_scalars = !data.is_empty()
                && !columns.is_empty()
                && data
                    .iter()
                    .all(|v| !matches!(v, CfmlValue::Array(_) | CfmlValue::Struct(_)));
            if flat_scalars {
                for chunk in data.chunks(columns.len()) {
                    let mut row = ValueMap::default();
                    for (i, val) in chunk.iter().enumerate() {
                        row.insert(columns[i].clone(), val.clone());
                    }
                    rows.push(row);
                }
            } else {
                for row_data in data.iter() {
                    match row_data {
                        CfmlValue::Array(values) => {
                            // Array of arrays: each inner array maps positionally to columns
                            let mut row = ValueMap::default();
                            for (i, val) in values.iter().enumerate() {
                                if i < columns.len() {
                                    row.insert(columns[i].clone(), val.clone());
                                }
                            }
                            rows.push(row);
                        }
                        CfmlValue::Struct(s) => {
                            rows.push(s.snapshot());
                        }
                        _ => {
                            // Single-column shortcut: wrap scalar in a row
                            let mut row = ValueMap::default();
                            if !columns.is_empty() {
                                row.insert(columns[0].clone(), row_data.clone());
                            }
                            rows.push(row);
                        }
                    }
                }
            }
        }
    }
    Ok(CfmlValue::Query(CfmlQuery::from_parts(columns, rows)))
}

fn fn_query_add_row(args: Vec<CfmlValue>) -> CfmlResult {
    // Reference-typed: mutate the shared handle in place (the caller's query
    // grows), then return it. The VM normally intercepts `queryAddRow` (returning
    // the new row count); this builtin is the fallback path.
    if let Some(CfmlValue::Query(q)) = args.first() {
        let num_rows = if args.len() >= 2 {
            match &args[1] {
                CfmlValue::Int(n) => *n as usize,
                CfmlValue::Struct(data) => {
                    q.add_row(data.snapshot());
                    return Ok(CfmlValue::Query(q.clone()));
                }
                CfmlValue::Array(items) => {
                    // Lucee semantics: array-of-arrays → one positional row per
                    // inner array; array-of-structs → one row per struct; a flat
                    // array of scalars → a single positional row.
                    let items = items.snapshot();
                    let all_arrays = !items.is_empty()
                        && items.iter().all(|it| matches!(it, CfmlValue::Array(_)));
                    if all_arrays {
                        for it in items.into_iter() {
                            if let CfmlValue::Array(vals) = it {
                                q.add_row_positional(vals.snapshot());
                            }
                        }
                    } else if items.iter().all(|it| matches!(it, CfmlValue::Struct(_))) {
                        for it in &items {
                            if let CfmlValue::Struct(s) = it {
                                q.add_row(s.snapshot());
                            }
                        }
                    } else {
                        q.add_row_positional(items);
                    }
                    return Ok(CfmlValue::Query(q.clone()));
                }
                _ => 1,
            }
        } else {
            1
        };
        for _ in 0..num_rows {
            q.add_row(ValueMap::default());
        }
        Ok(CfmlValue::Query(q.clone()))
    } else {
        Ok(CfmlValue::Null)
    }
}

fn fn_query_set_cell(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 3 {
        if let CfmlValue::Query(q) = &args[0] {
            let column = args[1].as_string();
            let value = args[2].clone();
            let row_idx = if args.len() >= 4 {
                (get_int(&args, 3) as usize).saturating_sub(1)
            } else {
                q.row_count().saturating_sub(1)
            };
            q.set_cell(row_idx, column, value);
            return Ok(CfmlValue::Query(q.clone()));
        }
    }
    Ok(CfmlValue::Null)
}

fn fn_query_add_column(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let CfmlValue::Query(q) = &args[0] {
            let col_name = args[1].as_string();
            let values: Vec<CfmlValue> = match args.get(2) {
                Some(CfmlValue::Array(arr)) => arr.snapshot(),
                _ => Vec::new(),
            };
            // Mutate the shared handle IN PLACE (CFML by-reference): callers that
            // don't reassign the return (Wheels' afterFind $queryCallback) still
            // see the new column, and subsequent per-row cell writes hit the same
            // query rather than an orphaned clone.
            q.with_write(|d| d.add_column(col_name, values));
            return Ok(args[0].clone());
        }
    }
    Ok(CfmlValue::Null)
}

fn fn_query_get_row(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let CfmlValue::Query(q) = &args[0] {
            let row_idx = (get_int(&args, 1) as usize).saturating_sub(1);
            if let Some(row) = q.get_row(row_idx) {
                return Ok(CfmlValue::strukt(row));
            }
            return Err(CfmlError::runtime(format!("queryGetRow: row {} is out of range (query has {} rows)", row_idx + 1, q.row_count())));
        }
    }
    Err(CfmlError::runtime("queryGetRow requires a query and row number".to_string()))
}

fn fn_query_get_cell(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let CfmlValue::Query(q) = &args[0] {
            let column = args[1].as_string();
            let row_idx = if args.len() >= 3 {
                (get_int(&args, 2) as usize).saturating_sub(1)
            } else {
                0
            };
            if let Some(row) = q.get_row(row_idx) {
                let col_lower = column.to_lowercase();
                for (k, v) in &row {
                    if k.eq_ignore_ascii_case(&col_lower) {
                        return Ok(v.clone());
                    }
                }
                return Ok(CfmlValue::Null);
            }
            return Err(CfmlError::runtime(format!("queryGetCell: row {} is out of range", row_idx + 1)));
        }
    }
    Err(CfmlError::runtime("queryGetCell requires a query and column name".to_string()))
}

fn fn_query_record_count(args: Vec<CfmlValue>) -> CfmlResult {
    match args.first() {
        Some(CfmlValue::Query(q)) => Ok(CfmlValue::Int(q.row_count() as i64)),
        _ => Ok(CfmlValue::Int(0)),
    }
}

fn fn_query_column_count(args: Vec<CfmlValue>) -> CfmlResult {
    match args.first() {
        Some(CfmlValue::Query(q)) => Ok(CfmlValue::Int(q.column_count() as i64)),
        _ => Ok(CfmlValue::Int(0)),
    }
}

fn fn_query_column_list(args: Vec<CfmlValue>) -> CfmlResult {
    match args.first() {
        // columnList reports column names uppercased, matching Lucee/ACF.
        Some(CfmlValue::Query(q)) => Ok(CfmlValue::string(q.column_list())),
        _ => Ok(CfmlValue::string(String::new())),
    }
}

fn fn_query_delete_row(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let CfmlValue::Query(q) = &args[0] {
            let mut data = q.with_read(|d| d.clone());
            let row_idx = (get_int(&args, 1) as usize).saturating_sub(1);
            if data.remove_row(row_idx).is_some() {
                return Ok(CfmlValue::Query(CfmlQuery::from_data(data)));
            }
            return Err(CfmlError::runtime(format!("queryDeleteRow: row {} is out of range", row_idx + 1)));
        }
    }
    Ok(CfmlValue::Null)
}

fn fn_query_delete_column(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let CfmlValue::Query(q) = &args[0] {
            let mut data = q.with_read(|d| d.clone());
            let col_name = args[1].as_string();
            data.remove_column_by_name(&col_name);
            return Ok(CfmlValue::Query(CfmlQuery::from_data(data)));
        }
    }
    Ok(CfmlValue::Null)
}

fn fn_query_append(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let (CfmlValue::Query(q1), CfmlValue::Query(q2)) = (&args[0], &args[1]) {
            let mut data = q1.with_read(|d| d.clone());
            q2.with_read(|d2| data.append_query(d2));
            return Ok(CfmlValue::Query(CfmlQuery::from_data(data)));
        }
    }
    Err(CfmlError::runtime("queryAppend() requires two query arguments".to_string()))
}

fn fn_query_insert_at(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 3 {
        if let CfmlValue::Query(q) = &args[0] {
            let mut data = q.with_read(|d| d.clone());
            let position = (get_int(&args, 2) as usize).saturating_sub(1);
            if position > data.row_count() {
                return Err(CfmlError::runtime(format!(
                    "queryInsertAt: position {} is out of range (query has {} rows)",
                    position + 1, data.row_count()
                )));
            }
            let row_data: ValueMap = match &args[1] {
                CfmlValue::Struct(d) => d.snapshot(),
                _ => ValueMap::default(),
            };
            data.insert_row_named(position, row_data);
            return Ok(CfmlValue::Query(CfmlQuery::from_data(data)));
        }
    }
    Err(CfmlError::runtime("queryInsertAt() requires a query, row data, and position".to_string()))
}

fn fn_query_prepend(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let (CfmlValue::Query(q1), CfmlValue::Query(q2)) = (&args[0], &args[1]) {
            let mut data = q1.with_read(|d| d.clone());
            q2.with_read(|d2| data.prepend_query(d2));
            return Ok(CfmlValue::Query(CfmlQuery::from_data(data)));
        }
    }
    Err(CfmlError::runtime("queryPrepend() requires two query arguments".to_string()))
}

fn fn_query_reverse(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(CfmlValue::Query(q)) = args.first() {
        let mut data = q.with_read(|d| d.clone());
        data.reverse_rows();
        return Ok(CfmlValue::Query(CfmlQuery::from_data(data)));
    }
    Err(CfmlError::runtime("queryReverse() requires a query argument".to_string()))
}

fn fn_query_row_swap(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 3 {
        if let CfmlValue::Query(q) = &args[0] {
            let mut data = q.with_read(|d| d.clone());
            let row1 = (get_int(&args, 1) as usize).saturating_sub(1);
            let row2 = (get_int(&args, 2) as usize).saturating_sub(1);
            let rc = data.row_count();
            if row1 >= rc {
                return Err(CfmlError::runtime(format!(
                    "queryRowSwap: row1 {} is out of range (query has {} rows)",
                    row1 + 1, rc
                )));
            }
            if row2 >= rc {
                return Err(CfmlError::runtime(format!(
                    "queryRowSwap: row2 {} is out of range (query has {} rows)",
                    row2 + 1, rc
                )));
            }
            data.swap_rows(row1, row2);
            return Ok(CfmlValue::Query(CfmlQuery::from_data(data)));
        }
    }
    Err(CfmlError::runtime("queryRowSwap() requires a query and two row numbers".to_string()))
}

fn fn_query_set_row(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 3 {
        if let CfmlValue::Query(q) = &args[0] {
            let mut data = q.with_read(|d| d.clone());
            let row_idx = (get_int(&args, 1) as usize).saturating_sub(1);
            let rc = data.row_count();
            if row_idx >= rc {
                return Err(CfmlError::runtime(format!(
                    "querySetRow: row {} is out of range (query has {} rows)",
                    row_idx + 1, rc
                )));
            }
            let row_data: ValueMap = match &args[2] {
                CfmlValue::Struct(d) => d.snapshot(),
                _ => ValueMap::default(),
            };
            // Replace cells in the existing row.
            for ci in 0..data.columns.len() {
                let col_name = data.columns[ci].clone();
                let val = row_data
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&col_name))
                    .map(|(_, v)| v.clone())
                    .unwrap_or(CfmlValue::Null);
                std::sync::Arc::make_mut(&mut data.data[ci])[row_idx] = val;
            }
            return Ok(CfmlValue::Query(CfmlQuery::from_data(data)));
        }
    }
    Err(CfmlError::runtime("querySetRow() requires a query, row number, and row data".to_string()))
}

fn fn_query_ho_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("Query higher-order function requires VM-level closure support and was not intercepted.".to_string()))
}

fn fn_query_register_function_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime(
        "queryRegisterFunction requires VM-level intercept and was not intercepted.".to_string(),
    ))
}

// ===============================================
// UTILITY FUNCTIONS
// ===============================================

fn fn_evaluate(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("evaluate() is not implemented. Use direct variable references or struct bracket notation instead.".to_string()))
}

fn fn_iif(args: Vec<CfmlValue>) -> CfmlResult {
    // IIf evaluates the string branches as expressions (like CFML's evaluate()).
    // The canonical pattern is iif(cond, de("yes"), de("no")) where de() wraps
    // in quotes and iif() unwraps via evaluation.
    fn eval_branch(v: &CfmlValue) -> CfmlValue {
        if let CfmlValue::String(s) = v {
            // Simple quoted-string case: "yes" -> yes
            if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
                return CfmlValue::string(s[1..s.len()-1].replace("\"\"", "\""));
            }
            if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
                return CfmlValue::string(s[1..s.len()-1].replace("''", "'").to_string());
            }
            // Numeric literal
            if let Ok(i) = s.parse::<i64>() {
                return CfmlValue::Int(i);
            }
            if let Ok(f) = s.parse::<f64>() {
                return CfmlValue::Double(f);
            }
        }
        v.clone()
    }
    if args.len() >= 3 {
        if args[0].is_true() { Ok(eval_branch(&args[1])) } else { Ok(eval_branch(&args[2])) }
    } else {
        Ok(CfmlValue::Null)
    }
}

fn fn_duplicate(args: Vec<CfmlValue>) -> CfmlResult {
    // Deep copy: arrays/structs are reference-typed, so duplicate() must break
    // all sharing and return a fully independent value.
    Ok(args
        .first()
        .map(|v| v.deep_copy())
        .unwrap_or(CfmlValue::Null))
}

fn fn_sleep(args: Vec<CfmlValue>) -> CfmlResult {
    let ms = get_int(&args, 0).max(0) as u64;
    std::thread::sleep(std::time::Duration::from_millis(ms));
    Ok(CfmlValue::Null)
}

fn fn_get_metadata(args: Vec<CfmlValue>) -> CfmlResult {
    let mut meta = ValueMap::default();
    if let Some(val) = args.first() {
        match val {
            // GetMetadata(query) -> an ARRAY of column-metadata structs in ordinal
            // order ({name, typeName, isCaseSensitive}), matching Lucee/ACF. Wheels'
            // $optionsForSelect reads `.name` from each entry to discover columns.
            CfmlValue::Query(q) => {
                let cols = q.columns();
                let entries: Vec<CfmlValue> = q.with_read(|d| {
                    cols.iter()
                        .enumerate()
                        .map(|(ci, name)| {
                            // Infer a Lucee-ish typeName from the first non-null cell.
                            let type_name = d
                                .data
                                .get(ci)
                                .and_then(|col| col.iter().find(|v| !matches!(v, CfmlValue::Null)))
                                .map(|v| match v {
                                    CfmlValue::Int(_) | CfmlValue::Double(_) => "DOUBLE",
                                    CfmlValue::Bool(_) => "BOOLEAN",
                                    CfmlValue::Binary(_) => "OBJECT",
                                    _ => "VARCHAR",
                                })
                                .unwrap_or("VARCHAR");
                            let mut m = ValueMap::default();
                            m.insert("name".to_string(), CfmlValue::string(name.clone()));
                            m.insert("typeName".to_string(), CfmlValue::string(type_name.to_string()));
                            m.insert("isCaseSensitive".to_string(), CfmlValue::Bool(false));
                            CfmlValue::strukt(m)
                        })
                        .collect()
                });
                return Ok(CfmlValue::array(entries));
            }
            CfmlValue::Struct(s) => {
                // Extract __name
                if let Some(name) = s.get("__name") {
                    meta.insert("name".to_string(), name.clone());
                    // fullname: the fully-qualified dotted component path. Lucee
                    // and ACF expose both `name` and `fullname`; frameworks read
                    // it (e.g. Wheels' Mapper.cfc keys mix-ins off .fullname).
                    meta.insert("fullname".to_string(), name.clone());
                }
                // Type
                meta.insert("type".to_string(), CfmlValue::string("component".to_string()));

                // `path` = the absolute filesystem path to the .cfc. Lucee/ACF
                // include it in component metadata; Preside's PresideObjectReader
                // reads `meta.path` to re-parse the source for declared-property
                // order (calling `.reReplace()` on it), so a missing key NPE'd.
                if let Some(CfmlValue::String(src)) = s.get("__source_file") {
                    meta.insert("path".to_string(), CfmlValue::string(src.to_string()));
                }

                // Extract __extends info
                if let Some(CfmlValue::Array(chain)) = s.get("__extends_chain") {
                    if let Some(first) = chain.first() {
                        let mut extends_meta = ValueMap::default();
                        extends_meta.insert("name".to_string(), first.clone());
                        meta.insert("extends".to_string(), CfmlValue::strukt(extends_meta));
                    }
                    meta.insert("fullExtends".to_string(), CfmlValue::Array(chain.clone()));
                }

                // `implements`: a struct keyed by each implemented interface's
                // declared FQN -> a minimal interface metadata stub. Lucee/ACF
                // expose `implements` this way (keyed by interface name);
                // frameworks (Wheels interface specs, WireBox) read the keys to
                // detect a declared interface contract.
                if let Some(imp) = build_implements_meta(&s.snapshot()) {
                    meta.insert("implements".to_string(), imp);
                }

                // Extract __metadata (custom attributes)
                // In CFML, custom attributes appear as top-level keys in getMetadata()
                if let Some(CfmlValue::Struct(md)) = s.get("__metadata") {
                    for (mk, mv) in md.iter() {
                        meta.insert(mk.clone(), mv.clone());
                    }
                    meta.insert("metadata".to_string(), CfmlValue::Struct(md.clone()));
                }

                // Enumerate functions. `all_entries()` unions the shared method
                // table (component flyweight) so methods that now live once per
                // class — not per instance — still appear in getMetadata().
                let mut functions = Vec::new();
                for (k, v) in s.all_entries() {
                    if k.starts_with("__") { continue; }
                    if let CfmlValue::Function(f) = v {
                        let mut func_meta = ValueMap::default();
                        func_meta.insert("name".to_string(), CfmlValue::string(k.clone()));
                        func_meta.insert("access".to_string(), CfmlValue::string(
                            match f.access {
                                CfmlAccess::Public => "public",
                                CfmlAccess::Private => "private",
                                CfmlAccess::Package => "package",
                                CfmlAccess::Remote => "remote",
                            }.to_string()
                        ));
                        if let Some(ref rt) = f.return_type {
                            func_meta.insert("returnType".to_string(), CfmlValue::string(rt.clone()));
                        }
                        // Parameter details
                        let params: Vec<CfmlValue> = f.params.iter().map(|p| {
                            let mut pm = ValueMap::default();
                            pm.insert("name".to_string(), CfmlValue::string(p.name.clone()));
                            if let Some(ref t) = p.param_type {
                                pm.insert("type".to_string(), CfmlValue::string(t.clone()));
                            }
                            pm.insert("required".to_string(), CfmlValue::Bool(p.required));
                            if let Some(ref d) = p.default {
                                pm.insert("default".to_string(), d.clone());
                            }
                            // Javadoc/inline annotations (e.g. WireBox `inject`)
                            // appear as top-level keys on the parameter struct.
                            for (k, v) in &p.annotations {
                                pm.insert(k.clone(), CfmlValue::string(v.clone()));
                            }
                            CfmlValue::strukt(pm)
                        }).collect();
                        func_meta.insert("parameters".to_string(), CfmlValue::array(params));
                        // Function metadata (__funcmeta_<name>): doc-comment /
                        // inline annotations (@beforeEach, @aroundEach,
                        // @expectedException, ...). Lucee/ACF surface these as
                        // FLAT top-level keys on the function struct — which is
                        // exactly what TestBox's getAnnotatedMethods reads via
                        // `structKeyExists(thisFunction, annotation)` — so flatten
                        // them there (without clobbering the reserved keys above),
                        // while keeping the `metadata` sub-struct for callers that
                        // read it directly.
                        let meta_key = format!("__funcmeta_{}", k);
                        if let Some(CfmlValue::Struct(fm)) = s.get(&meta_key) {
                            for (mk, mv) in fm.iter() {
                                if !func_meta.contains_key(&mk) {
                                    func_meta.insert(mk, mv);
                                }
                            }
                            func_meta.insert("metadata".to_string(), CfmlValue::Struct(fm.clone()));
                        }
                        functions.push(CfmlValue::strukt(func_meta));
                    }
                }
                meta.insert("functions".to_string(), CfmlValue::array(functions));

                // Enumerate properties. Declared properties live in the
                // __properties array (with full annotations incl. inject/type),
                // matching getComponentMetadata and Lucee/ACF. This is what
                // WireBox reads for property/DSL injection. Fall back to
                // top-level non-function keys only when a component declares no
                // properties (no __properties key).
                if let Some(CfmlValue::Array(props)) = s.get("__properties") {
                    meta.insert("properties".to_string(), CfmlValue::Array(props.clone()));
                } else {
                    let mut properties = Vec::new();
                    for (k, v) in s.iter() {
                        if k.starts_with("__") { continue; }
                        if matches!(v, CfmlValue::Function(_)) { continue; }
                        let mut prop_meta = ValueMap::default();
                        prop_meta.insert("name".to_string(), CfmlValue::string(k.clone()));
                        prop_meta.insert("type".to_string(), CfmlValue::string(v.type_name().to_string()));
                        properties.push(CfmlValue::strukt(prop_meta));
                    }
                    meta.insert("properties".to_string(), CfmlValue::array(properties));
                }
            }
            CfmlValue::Function(f) => {
                meta.insert("name".to_string(), CfmlValue::string(f.name.clone()));
                meta.insert("access".to_string(), CfmlValue::string(
                    match f.access {
                        CfmlAccess::Public => "public",
                        CfmlAccess::Private => "private",
                        CfmlAccess::Package => "package",
                        CfmlAccess::Remote => "remote",
                    }.to_string()
                ));
                if let Some(ref rt) = f.return_type {
                    meta.insert("returnType".to_string(), CfmlValue::string(rt.clone()));
                }
                let params: Vec<CfmlValue> = f.params.iter().map(|p| {
                    let mut pm = ValueMap::default();
                    pm.insert("name".to_string(), CfmlValue::string(p.name.clone()));
                    if let Some(ref t) = p.param_type {
                        pm.insert("type".to_string(), CfmlValue::string(t.clone()));
                    }
                    pm.insert("required".to_string(), CfmlValue::Bool(p.required));
                    if let Some(ref d) = p.default {
                        pm.insert("default".to_string(), d.clone());
                    }
                    CfmlValue::strukt(pm)
                }).collect();
                meta.insert("parameters".to_string(), CfmlValue::array(params));
            }
            _ => {
                meta.insert("type".to_string(), CfmlValue::string(val.type_name().to_string()));
            }
        }
    }
    Ok(CfmlValue::strukt(meta))
}

fn fn_is_instance_of(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() < 2 {
        return Ok(CfmlValue::Bool(false));
    }
    let obj = &args[0];
    let type_name = args[1].as_string();
    let type_lower = type_name.to_lowercase();

    // Phase C.3 — Slice 5: flyweight instance — match against its precomputed type
    // identifiers (own name + superclass chain + interfaces), plus the base
    // "component" type. `is_instance_backed()` is const false in a default build.
    if let Some(comp) = obj.as_component() {
        if comp.is_instance_backed() {
            if type_lower == "component" {
                return Ok(CfmlValue::Bool(true));
            }
            let matched = comp.type_identifiers().iter().any(|id| {
                let l = id.to_lowercase();
                l == type_lower
                    || l.rsplit('.').next().map(|seg| seg == type_lower).unwrap_or(false)
            });
            return Ok(CfmlValue::Bool(matched));
        }
    }

    if let CfmlValue::Struct(s) = obj {
        // Lucee parity: every CFC is an instance of the base type "Component"
        // (case-insensitive), and ONLY that exact name — NOT "Object", NOT
        // "lucee.runtime.Component", and NOT a plain struct. Verified against a
        // live Lucee server. MockBox's `normalizeArguments` depends on this to
        // pick `serializeJSON(cfc)` over a member `cfc.toString()` call (line 510
        // of MockBox.cfc) — the latter throws "has no function with name
        // [toString]" on both engines, which is why component args failed to
        // match and ~40 cfflow specs errored. A component carries `__name` or the
        // `__variables` scope; a plain struct carries neither.
        let is_component = s.get("__name").is_some() || s.get("__variables").is_some();
        if is_component && type_lower == "component" {
            return Ok(CfmlValue::Bool(true));
        }
        // Check direct name match
        if let Some(CfmlValue::String(name)) = s.get("__name") {
            if name.to_lowercase() == type_lower {
                return Ok(CfmlValue::Bool(true));
            }
            // Also check last segment (e.g., "resource" matches "taffy.core.resource")
            if let Some(last) = name.split('.').last() {
                if last.to_lowercase() == type_lower {
                    return Ok(CfmlValue::Bool(true));
                }
            }
        }

        // Java shim structs carry __java_class ("java.lang.stringbuilder" etc.)
        if let Some(CfmlValue::String(jclass)) = s.get("__java_class") {
            if jclass.to_lowercase() == type_lower {
                return Ok(CfmlValue::Bool(true));
            }
            if let Some(last) = jclass.split('.').last() {
                if last.to_lowercase() == type_lower {
                    return Ok(CfmlValue::Bool(true));
                }
            }
        }

        // Walk extends chain
        if let Some(CfmlValue::Array(chain)) = s.get("__extends_chain") {
            for item in chain.iter() {
                let item_str = item.as_string();
                if item_str.to_lowercase() == type_lower {
                    return Ok(CfmlValue::Bool(true));
                }
                // Check last segment
                if let Some(last) = item_str.split('.').last() {
                    if last.to_lowercase() == type_lower {
                        return Ok(CfmlValue::Bool(true));
                    }
                }
            }
        }

        // Check direct interfaces (__implements)
        if let Some(CfmlValue::Array(ifaces)) = s.get("__implements") {
            for item in ifaces.iter() {
                let item_str = item.as_string();
                if item_str.to_lowercase() == type_lower {
                    return Ok(CfmlValue::Bool(true));
                }
                if let Some(last) = item_str.split('.').last() {
                    if last.to_lowercase() == type_lower {
                        return Ok(CfmlValue::Bool(true));
                    }
                }
            }
        }

        // Check inherited interfaces (__implements_chain)
        if let Some(CfmlValue::Array(ifaces)) = s.get("__implements_chain") {
            for item in ifaces.iter() {
                let item_str = item.as_string();
                if item_str.to_lowercase() == type_lower {
                    return Ok(CfmlValue::Bool(true));
                }
                if let Some(last) = item_str.split('.').last() {
                    if last.to_lowercase() == type_lower {
                        return Ok(CfmlValue::Bool(true));
                    }
                }
            }
        }

        // Check package-qualified interface FQNs (issue #206) — an unqualified
        // implements="X" on a component loaded via a package path resolves to
        // "<pkg>.X", so isInstanceOf(obj, "<pkg>.X") must match. Path-aware
        // (exact FQN), matching Lucee: "wrong.pkg.X" does NOT match.
        if let Some(CfmlValue::Array(ifaces)) = s.get("__implements_fqns") {
            for item in ifaces.iter() {
                if item.as_string().to_lowercase() == type_lower {
                    return Ok(CfmlValue::Bool(true));
                }
            }
        }
    }

    // Fallback for non-component values: match against the value's native Java
    // identity, mirroring how Lucee's isInstanceOf walks the concrete runtime
    // class's type hierarchy. The alias sets below are taken VERBATIM from what
    // Lucee 6 returns true for (probed against a live server) — notably Lucee
    // does NOT accept the bare CFML type names "Array"/"Struct"/"Boolean"/
    // "Query" here (only the Java/Lucee class + interface names), but DOES accept
    // "String" and "numeric". A numeric is a java.lang.Double regardless of
    // Int/Double storage, so neither maps to java.lang.Integer. A component
    // Struct (carrying __name / __java_class) is NOT eligible: if its metadata
    // above didn't match, the answer is genuinely false. Verified vs Lucee 6.1.
    let native_aliases: &[&str] = match obj {
        CfmlValue::Array(_) => &[
            "java.util.list",
            "java.util.collection",
            "java.lang.iterable",
            "lucee.runtime.type.arrayimpl",
            "lucee.runtime.type.array",
        ],
        CfmlValue::Struct(s)
            if !s.contains_key("__name") && !s.contains_key("__java_class") =>
        {
            &[
                "java.util.map",
                "lucee.runtime.type.structimpl",
                "lucee.runtime.type.struct",
            ]
        }
        CfmlValue::Query(_) => &[
            "lucee.runtime.type.queryimpl",
            "lucee.runtime.type.query",
        ],
        CfmlValue::String(_) => &[
            "string",
            "java.lang.string",
            "java.lang.charsequence",
            "java.lang.comparable",
        ],
        CfmlValue::Bool(_) => &["java.lang.boolean"],
        // A CFML numeric is a java.lang.Double in Lucee irrespective of whether
        // RustCFML stored it as Int or Double, so both map to the same aliases.
        CfmlValue::Int(_) | CfmlValue::Double(_) => &[
            "numeric",
            "java.lang.double",
            "java.lang.number",
        ],
        _ => &[],
    };
    if native_aliases.iter().any(|a| *a == type_lower) {
        return Ok(CfmlValue::Bool(true));
    }

    Ok(CfmlValue::Bool(false))
}

fn fn_create_object(args: Vec<CfmlValue>) -> CfmlResult {
    // Stub - VM intercepts this call before it reaches here
    // If we get here, just return a struct with a marker
    if args.len() >= 2 {
        let obj_type = args[0].as_string().to_lowercase();
        if obj_type == "component" {
            let mut s = ValueMap::default();
            s.insert("__createObject".to_string(), CfmlValue::string(args[1].as_string()));
            return Ok(CfmlValue::strukt(s));
        }
    }
    Ok(CfmlValue::Null)
}

fn fn_create_uuid(_args: Vec<CfmlValue>) -> CfmlResult {
    let nanos = cfml_common::clock::now_unix_nanos() as u64;
    let random_bits = ((cfml_random() * u32::MAX as f64) as u64) << 32
                    | (cfml_random() * u32::MAX as f64) as u64;
    let mixed = nanos ^ random_bits;
    // CFML UUID format: 8-4-4-16
    Ok(CfmlValue::string(format!(
        "{:08X}-{:04X}-{:04X}-{:016X}",
        ((mixed >> 32) as u32),
        ((mixed >> 16) as u16),
        (mixed as u16),
        (nanos.wrapping_mul(6364136223846793005).wrapping_add(random_bits)),
    )))
}

fn fn_preserve_single_quotes(args: Vec<CfmlValue>) -> CfmlResult {
    // Tells cfquery not to escape single quotes inside the value. Outside a
    // query the string is returned verbatim (the "preservation" is a no-op
    // marker), which matches Lucee's behaviour when used as a plain function.
    Ok(CfmlValue::string(get_str(&args, 0)))
}

fn fn_create_unique_id(args: Vec<CfmlValue>) -> CfmlResult {
    use std::sync::atomic::{AtomicU64, Ordering};
    // createUniqueID("counter") returns a per-instance lifecycle counter (1, 2, ...).
    if !args.is_empty() && get_str(&args, 0).eq_ignore_ascii_case("counter") {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
        return Ok(CfmlValue::string(n.to_string()));
    }

    // Default: a 16-byte UUID encoded as URL-safe Base64 without padding (22 chars).
    let nanos = cfml_common::clock::now_unix_nanos() as u64;
    let random_bits = ((cfml_random() * u32::MAX as f64) as u64) << 32
                    | (cfml_random() * u32::MAX as f64) as u64;
    let hi = nanos ^ random_bits;
    let lo = nanos.wrapping_mul(6364136223846793005).wrapping_add(random_bits);
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&hi.to_be_bytes());
    bytes[8..].copy_from_slice(&lo.to_be_bytes());

    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut result = String::with_capacity(22);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        result.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            result.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            result.push(ALPHABET[(n & 63) as usize] as char);
        }
    }
    Ok(CfmlValue::string(result))
}

fn fn_create_guid(_args: Vec<CfmlValue>) -> CfmlResult {
    let nanos = cfml_common::clock::now_unix_nanos() as u64;
    let random_bits = ((cfml_random() * u32::MAX as f64) as u64) << 32
                    | (cfml_random() * u32::MAX as f64) as u64;
    let mixed = nanos ^ random_bits;
    let extra = nanos.wrapping_mul(6364136223846793005).wrapping_add(random_bits);
    // Standard GUID format: 8-4-4-4-12
    Ok(CfmlValue::string(format!(
        "{:08X}-{:04X}-{:04X}-{:04X}-{:012X}",
        (mixed >> 32) as u32,
        (mixed >> 16) as u16,
        ((mixed as u16) & 0x0FFF) | 0x4000,
        ((extra >> 48) as u16 & 0x3FFF) | 0x8000,
        extra & 0xFFFFFFFFFFFF,
    )))
}

fn fn_hash(args: Vec<CfmlValue>) -> CfmlResult {
    use md5::Md5;
    use sha2::{Sha256, Sha384, Sha512, Digest};
    use sha1::Sha1;
    let input = get_str(&args, 0);
    let algorithm = if args.len() >= 2 { get_str(&args, 1).to_uppercase() } else { "MD5".to_string() };
    let hex = match algorithm.as_str() {
        "MD5" => {
            let mut hasher = Md5::new();
            hasher.update(input.as_bytes());
            format!("{:X}", hasher.finalize())
        }
        "SHA-1" | "SHA1" => {
            let mut hasher = Sha1::new();
            hasher.update(input.as_bytes());
            format!("{:X}", hasher.finalize())
        }
        "SHA-256" | "SHA256" => {
            let mut hasher = Sha256::new();
            hasher.update(input.as_bytes());
            format!("{:X}", hasher.finalize())
        }
        "SHA-384" | "SHA384" => {
            let mut hasher = Sha384::new();
            hasher.update(input.as_bytes());
            format!("{:X}", hasher.finalize())
        }
        "SHA-512" | "SHA512" => {
            let mut hasher = Sha512::new();
            hasher.update(input.as_bytes());
            format!("{:X}", hasher.finalize())
        }
        _ => {
            // Lucee throws java.security.NoSuchAlgorithmException here
            // ("bogus-alg MessageDigest not available", verified on 7.0.4). This
            // used to fall back to MD5, so `hash(secret, "SHA-3")` — a typo, or
            // an algorithm we simply don't implement — silently produced a
            // plausible-looking MD5 digest instead of failing.
            return Err(CfmlError::no_such_algorithm(&algorithm.to_lowercase()));
        }
    };
    Ok(CfmlValue::string(hex))
}

fn fn_ls_parse_number(args: Vec<CfmlValue>) -> CfmlResult {
    fn_to_numeric(args)
}

// ===============================================
// INI FILE FUNCTIONS
// ===============================================

/// Parse an INI file into sections: HashMap<section_name, Vec<(key, value)>>
fn parse_ini_file(content: &str) -> (Vec<String>, HashMap<String, Vec<(String, String)>>) {
    let mut sections: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut section_order: Vec<String> = Vec::new();
    let mut current_section = String::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len()-1].trim().to_string();
            if !sections.contains_key(&current_section) {
                section_order.push(current_section.clone());
                sections.insert(current_section.clone(), Vec::new());
            }
        } else if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim().to_string();
            let value = line[eq_pos+1..].trim().to_string();
            sections.entry(current_section.clone()).or_default().push((key, value));
        }
    }
    (section_order, sections)
}

/// getProfileString(iniPath, section, entry) — read a value from an INI file
fn fn_get_profile_string(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() < 3 {
        return Err(CfmlError::runtime("getProfileString requires 3 arguments: iniPath, section, entry".to_string()));
    }
    let path = get_str(&args, 0);
    let section = get_str(&args, 1);
    let entry = get_str(&args, 2);

    let content = std::fs::read_to_string(&path).map_err(|e| {
        CfmlError::runtime(format!("getProfileString: cannot read '{}': {}", path, e))
    })?;

    let (_, sections) = parse_ini_file(&content);
    let section_lower = section.to_lowercase();
    let entry_lower = entry.to_lowercase();

    for (sec_name, entries) in &sections {
        if sec_name.to_lowercase() == section_lower {
            for (k, v) in entries {
                if k.eq_ignore_ascii_case(&entry_lower) {
                    return Ok(CfmlValue::string(v.clone()));
                }
            }
        }
    }
    Ok(CfmlValue::string(String::new()))
}

/// setProfileString(iniPath, section, entry, value) — write a value to an INI file
fn fn_set_profile_string(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() < 4 {
        return Err(CfmlError::runtime("setProfileString requires 4 arguments: iniPath, section, entry, value".to_string()));
    }
    let path = get_str(&args, 0);
    let section = get_str(&args, 1);
    let entry = get_str(&args, 2);
    let value = get_str(&args, 3);

    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let (section_order, mut sections) = parse_ini_file(&content);

    // Find or create section (case-insensitive match)
    let section_lower = section.to_lowercase();
    let actual_section = section_order.iter()
        .find(|s| s.to_lowercase() == section_lower)
        .cloned()
        .unwrap_or_else(|| section.clone());

    let entries = sections.entry(actual_section.clone()).or_default();

    // Update existing key or append new one
    let entry_lower = entry.to_lowercase();
    let mut found = false;
    for (k, v) in entries.iter_mut() {
        if k.eq_ignore_ascii_case(&entry_lower) {
            *v = value.clone();
            found = true;
            break;
        }
    }
    if !found {
        entries.push((entry.clone(), value.clone()));
    }

    // Write back — preserve section order, add new sections at end
    let mut output = String::new();
    let mut written_sections: Vec<String> = Vec::new();

    for sec_name in &section_order {
        if let Some(entries) = sections.get(sec_name) {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&format!("[{}]\n", sec_name));
            for (k, v) in entries {
                output.push_str(&format!("{}={}\n", k, v));
            }
            written_sections.push(sec_name.clone());
        }
    }
    // New sections not in original order
    for (sec_name, entries) in &sections {
        if !written_sections.contains(sec_name) {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&format!("[{}]\n", sec_name));
            for (k, v) in entries {
                output.push_str(&format!("{}={}\n", k, v));
            }
        }
    }

    std::fs::write(&path, &output).map_err(|e| {
        CfmlError::runtime(format!("setProfileString: cannot write '{}': {}", path, e))
    })?;

    Ok(CfmlValue::Null)
}

/// getProfileSections(iniPath) — return a struct of section names → comma-separated key lists
fn fn_get_profile_sections(args: Vec<CfmlValue>) -> CfmlResult {
    if args.is_empty() {
        return Err(CfmlError::runtime("getProfileSections requires 1 argument: iniPath".to_string()));
    }
    let path = get_str(&args, 0);

    let content = std::fs::read_to_string(&path).map_err(|e| {
        CfmlError::runtime(format!("getProfileSections: cannot read '{}': {}", path, e))
    })?;

    let (section_order, sections) = parse_ini_file(&content);
    let mut result = ValueMap::default();

    for sec_name in &section_order {
        if let Some(entries) = sections.get(sec_name) {
            let keys: Vec<String> = entries.iter().map(|(k, _)| k.clone()).collect();
            result.insert(sec_name.clone(), CfmlValue::string(keys.join(",")));
        }
    }

    Ok(CfmlValue::strukt(result))
}

// FILE I/O FUNCTIONS
// ===============================================

fn fn_file_read(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    match std::fs::read_to_string(&path) {
        Ok(contents) => Ok(CfmlValue::string(contents)),
        // Lucee surfaces a missing file from fileRead() as an `expression` error
        // (only fileReadBinary uses FileNotFoundException) — match that asymmetry.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(CfmlError::expression(format!("The file [{}] does not exist", path)))
        }
        Err(e) => Err(CfmlError::runtime(format!("fileRead: {}", e))),
    }
}

fn fn_file_write(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() < 2 {
        return Err(CfmlError::runtime("fileWrite requires path and data".to_string()));
    }
    let path = get_str(&args, 0);
    // Binary data must be written as raw bytes; only stringify simple values.
    // Otherwise a CfmlValue::Binary would serialize to the placeholder
    // "<Binary>" (as_string), corrupting every file written from a binary
    // (Preside's FileSystemStorageProvider.putObject → FileWrite(path, binary)).
    let result = match args.get(1) {
        Some(CfmlValue::Binary(bytes)) => std::fs::write(&path, bytes),
        _ => std::fs::write(&path, get_str(&args, 1).as_bytes()),
    };
    match result {
        Ok(_) => Ok(CfmlValue::Null),
        Err(e) => Err(CfmlError::runtime(format!("fileWrite: {}", e))),
    }
}

fn fn_file_append(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() < 2 {
        return Err(CfmlError::runtime("fileAppend requires path and data".to_string()));
    }
    let path = get_str(&args, 0);
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| CfmlError::runtime(format!("fileAppend: {}", e)))?;
    // Append raw bytes for binary data; stringify simple values (see fileWrite).
    let write_result = match args.get(1) {
        Some(CfmlValue::Binary(bytes)) => file.write_all(bytes),
        _ => file.write_all(get_str(&args, 1).as_bytes()),
    };
    write_result.map_err(|e| CfmlError::runtime(format!("fileAppend: {}", e)))?;
    Ok(CfmlValue::Null)
}

/// `fileExists()` — TRUE only for a regular file. A directory is NOT a file
/// (Lucee/ACF both answer `false`; `directoryExists()` is the directory test),
/// so this must be `is_file`, not `exists`. Reached only when the VM's
/// existence intercept is bypassed — the VM routes this through its configured
/// VFS so an embedded archive / engine-CFC overlay / S3 root is visible too.
fn fn_file_exists(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    Ok(CfmlValue::Bool(std::path::Path::new(&path).is_file()))
}

fn fn_file_delete(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    match std::fs::remove_file(&path) {
        Ok(_) => Ok(CfmlValue::Bool(true)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Lucee/ACF message contains "does not exist"; callers branch on it
            // to silently no-op a delete of a missing file (e.g. Preside's
            // FileSystemStorageProvider.deleteObject).
            Err(CfmlError::file_not_found(format!("The file [{}] does not exist", path)))
        }
        Err(e) => Err(CfmlError::runtime(format!("fileDelete: {}", e))),
    }
}

fn fn_file_move(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() < 2 {
        return Err(CfmlError::runtime("fileMove requires source and destination".to_string()));
    }
    let src = get_str(&args, 0);
    let dest = get_str(&args, 1);
    match std::fs::rename(&src, &dest) {
        Ok(_) => Ok(CfmlValue::Null),
        Err(e) => Err(CfmlError::runtime(format!("fileMove: {}", e))),
    }
}

fn fn_file_copy(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() < 2 {
        return Err(CfmlError::runtime("fileCopy requires source and destination".to_string()));
    }
    let src = get_str(&args, 0);
    let dest = get_str(&args, 1);
    match std::fs::copy(&src, &dest) {
        Ok(_) => Ok(CfmlValue::Null),
        Err(e) => Err(CfmlError::runtime(format!("fileCopy: {}", e))),
    }
}

fn fn_directory_create(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    match std::fs::create_dir_all(&path) {
        Ok(_) => Ok(CfmlValue::Null),
        Err(e) => Err(CfmlError::runtime(format!("directoryCreate: {}", e))),
    }
}

fn fn_directory_exists(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    Ok(CfmlValue::Bool(std::path::Path::new(&path).is_dir()))
}

fn fn_directory_delete(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    let recursive = if args.len() >= 2 { args[1].is_true() } else { false };
    let result = if recursive {
        std::fs::remove_dir_all(&path)
    } else {
        std::fs::remove_dir(&path)
    };
    match result {
        Ok(_) => Ok(CfmlValue::Null),
        Err(e) => Err(CfmlError::runtime(format!("directoryDelete: {}", e))),
    }
}

fn fn_directory_list(args: Vec<CfmlValue>) -> CfmlResult {
    // directoryList(path [, recurse [, listInfo [, filter [, sort [, type]]]]])
    let path = get_str(&args, 0);
    let recurse = if args.len() >= 2 { args[1].is_true() } else { false };
    let list_info = if args.len() >= 3 { get_str(&args, 2).to_lowercase() } else { "path".to_string() };
    let filter = if args.len() >= 4 { get_str(&args, 3) } else { String::new() };
    // 6th arg `type` (dir|file|all, default all) — Lucee parity. Wheels' plugin
    // loader uses directoryList(..., "dir") to enumerate only sub-directories.
    let type_filter = if args.len() >= 6 { get_str(&args, 5).to_lowercase() } else { "all".to_string() };

    // A filter is a pipe-delimited list of glob patterns (e.g. "*.cfm|*.cfc") —
    // match if ANY sub-pattern matches. Each sub-pattern is compiled ONCE here
    // (not per directory entry): a wildcard-free pattern (Sticker's asset lookups
    // pass an exact filename like "user.svg") becomes a cheap case-insensitive
    // string compare with no regex at all; a glob compiles to a single anchored,
    // case-insensitive regex reused across every entry. Compiling per-entry made
    // a filtered scan of a 1000-file dir do ~1000 regex compilations — Preside's
    // Sticker re-scans a FontAwesome icon dir hundreds of times per request, so
    // the old code cost ~500K compilations and pinned /admin at ~100s of CPU.
    // …but `compile_filter` still ran on every directoryList CALL, so a repeated
    // scan recompiled the same glob each time — 187 MiB of allocation churn on a
    // live Preside request. The derived regex depends only on the glob text, so
    // memoize it. Keyed by the glob (not the derived pattern) to skip rebuilding
    // the translation too. Deliberately a separate cache from REGEX_CACHE: these
    // patterns are already Rust-syntax, and must NOT go through
    // `translate_cfml_regex`, whose CFML/Java rewrites would change their meaning.
    static GLOB_RX_CACHE: Lazy<std::sync::RwLock<HashMap<String, std::sync::Arc<Regex>>>> =
        Lazy::new(|| std::sync::RwLock::new(HashMap::new()));
    // Same bound and wholesale-clear policy as REGEX_CACHE: globs come from
    // application code, so the set is small in practice, but it must not be
    // unbounded.
    const GLOB_RX_CACHE_CAP: usize = 1024;

    enum PatMatcher {
        Exact(String),              // lowercased literal — case-insensitive equality
        Rx(std::sync::Arc<Regex>),  // compiled glob, shared so the scratch pool stays warm
        Contains(String),           // lowercased fallback for a malformed glob
    }
    fn compile_filter(filter: &str) -> Vec<PatMatcher> {
        if filter.is_empty() {
            return Vec::new();
        }
        filter
            .split('|')
            .filter(|p| !p.is_empty())
            .map(|pattern| {
                if !pattern.contains('*') && !pattern.contains('?') {
                    return PatMatcher::Exact(pattern.to_lowercase());
                }
                if let Some(rx) = GLOB_RX_CACHE.read().unwrap().get(pattern) {
                    return PatMatcher::Rx(std::sync::Arc::clone(rx));
                }
                let mut re = String::with_capacity(pattern.len() + 8);
                re.push_str("(?i)^");
                for ch in pattern.chars() {
                    match ch {
                        '*' => re.push_str(".*"),
                        '?' => re.push('.'),
                        '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '\\' | '|' => {
                            re.push('\\');
                            re.push(ch);
                        }
                        _ => re.push(ch),
                    }
                }
                re.push('$');
                match Regex::new(&re) {
                    Ok(r) => {
                        let r = std::sync::Arc::new(r);
                        let mut cache = GLOB_RX_CACHE.write().unwrap();
                        if cache.len() >= GLOB_RX_CACHE_CAP {
                            cache.clear();
                        }
                        PatMatcher::Rx(std::sync::Arc::clone(
                            cache
                                .entry(pattern.to_string())
                                .or_insert_with(|| std::sync::Arc::clone(&r)),
                        ))
                    }
                    Err(_) => PatMatcher::Contains(pattern.replace('*', "").to_lowercase()),
                }
            })
            .collect()
    }
    fn matches_filter(filename: &str, matchers: &[PatMatcher]) -> bool {
        if matchers.is_empty() {
            return true;
        }
        // Only lowercase once, and only if a case-insensitive matcher needs it.
        let lower = filename.to_lowercase();
        matchers.iter().any(|m| match m {
            PatMatcher::Exact(p) => lower == *p,
            PatMatcher::Rx(r) => r.is_match(filename),
            PatMatcher::Contains(p) => lower.contains(p),
        })
    }

    enum Entry {
        Scalar(CfmlValue),
        Row { name: String, directory: String, size: u64, is_dir: bool },
    }

    fn list_dir(
        path: &str,
        recurse: bool,
        filter: &[PatMatcher],
        list_info: &str,
        type_filter: &str,
        visited: &mut std::collections::HashSet<std::path::PathBuf>,
    ) -> Result<Vec<Entry>, std::io::Error> {
        let mut results = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let entry_path = entry.path();
            let full_path = entry_path.to_string_lossy().to_string();
            let file_name = entry.file_name().to_string_lossy().to_string();

            // Determine dir/file WITHOUT a stat syscall for the common case:
            // `read_dir` already yields the entry's type (via `d_type`), so a
            // regular file or directory costs zero extra syscalls. Only a symlink
            // needs a follow-the-target `stat` to classify it — preserving the
            // previous `is_dir()`/`is_file()` (symlink-following) semantics while
            // dropping two stat calls per entry. Preside's admin re-scans a
            // ~1000-file FontAwesome icon dir hundreds of times per request; the
            // old double-stat made that ~1M syscalls and pinned a cold /admin boot
            // at ~100s of CPU.
            let (is_dir, is_file) = match entry.file_type() {
                Ok(ft) if ft.is_symlink() => match entry_path.metadata() {
                    Ok(m) => (m.is_dir(), m.is_file()),
                    Err(_) => (false, false),
                },
                Ok(ft) => (ft.is_dir(), ft.is_file()),
                Err(_) => (false, false),
            };

            // The name filter applies to BOTH files and directories (Lucee/ACF
            // behavior); directories are still always recursed into below
            // regardless of whether their own name matches the filter. The type
            // filter (dir|file|all) gates which entries are emitted but never
            // suppresses recursion.
            let type_ok = match type_filter {
                "dir" => is_dir,
                "file" => is_file,
                _ => true,
            };
            if (is_file || is_dir) && type_ok && matches_filter(&file_name, filter) {
                match list_info {
                    "query" => {
                        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                        let directory = entry_path.parent()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();
                        results.push(Entry::Row { name: file_name.clone(), directory, size, is_dir });
                    }
                    "name" => results.push(Entry::Scalar(CfmlValue::string(file_name.clone()))),
                    _ => results.push(Entry::Scalar(CfmlValue::string(full_path.clone()))),
                };
            }
            if recurse && is_dir {
                // Guard against symlink cycles: canonicalize the target and skip a
                // directory already on the current recursion's visited set, so a
                // looping symlink can never spin forever (a real hang, not a
                // divergence — no engine should follow a directory cycle).
                let canon = std::fs::canonicalize(&entry_path).unwrap_or_else(|_| entry_path.clone());
                if visited.insert(canon) {
                    results.extend(list_dir(&full_path, true, filter, list_info, type_filter, visited)?);
                }
            }
        }
        Ok(results)
    }

    // Lucee/ACF return an empty result for a non-existent directory rather
    // than throwing (ColdBox/Preside scan optional module locations like
    // `/app/extensions_app` that may not exist). Only a genuine I/O failure
    // (permissions, etc.) still surfaces as an error.
    let compiled_filter = compile_filter(&filter);
    let mut visited = std::collections::HashSet::new();
    let listing = match list_dir(&path, recurse, &compiled_filter, &list_info, &type_filter, &mut visited) {
        Ok(entries) => Ok(entries),
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        // A path that exists but is a FILE yields ENOTDIR (os error 20). Lucee
        // returns an empty listing for directoryList() on a file rather than
        // throwing. (ErrorKind::NotADirectory is unstable on the pinned
        // toolchain, so match the raw errno instead.)
        Err(ref e) if e.raw_os_error() == Some(20) => Ok(Vec::new()),
        Err(e) => Err(e),
    };

    match listing {
        Ok(entries) => {
            if list_info == "query" {
                let columns = vec![
                    "name".to_string(),
                    "directory".to_string(),
                    "size".to_string(),
                    "type".to_string(),
                    "dateLastModified".to_string(),
                    "attributes".to_string(),
                    "mode".to_string(),
                ];
                let q = cfml_common::dynamic::CfmlQuery::new(columns);
                for e in entries {
                    if let Entry::Row { name, directory, size, is_dir } = e {
                        let mut row = ValueMap::default();
                        row.insert("name".to_string(), CfmlValue::string(name));
                        row.insert("directory".to_string(), CfmlValue::string(directory));
                        row.insert("size".to_string(), CfmlValue::Int(size as i64));
                        row.insert("type".to_string(), CfmlValue::string(if is_dir { "Dir" } else { "File" }));
                        row.insert("dateLastModified".to_string(), CfmlValue::string(String::new()));
                        row.insert("attributes".to_string(), CfmlValue::string(String::new()));
                        row.insert("mode".to_string(), CfmlValue::string(String::new()));
                        q.add_row(row);
                    }
                }
                Ok(CfmlValue::Query(q))
            } else {
                let files: Vec<CfmlValue> = entries.into_iter().filter_map(|e| {
                    if let Entry::Scalar(v) = e { Some(v) } else { None }
                }).collect();
                Ok(CfmlValue::array(files))
            }
        }
        Err(e) => Err(CfmlError::runtime(format!("directoryList: {} (path: {})", e, path))),
    }
}

fn fn_get_temp_directory(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::string(std::env::temp_dir().to_string_lossy().to_string()))
}

fn fn_get_temp_file(args: Vec<CfmlValue>) -> CfmlResult {
    let dir = if args.is_empty() {
        std::env::temp_dir().to_string_lossy().to_string()
    } else {
        get_str(&args, 0)
    };
    let prefix = if args.len() >= 2 { get_str(&args, 1) } else { "tmp".to_string() };
    let ts = cfml_common::clock::now_unix_nanos();
    let path = std::path::Path::new(&dir).join(format!("{}{}.tmp", prefix, ts));
    Ok(CfmlValue::string(path.to_string_lossy().to_string()))
}

fn fn_get_file_info(args: Vec<CfmlValue>) -> CfmlResult {
    let path_str = get_str(&args, 0);
    let path = std::path::Path::new(&path_str);
    let meta = std::fs::metadata(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            // Lucee/ACF message contains "does not exist"; Preside's
            // FileSystemStorageProvider.getObjectInfo branches on it.
            CfmlError::file_not_found(format!("file or directory [{}] does not exist", path_str))
        } else {
            CfmlError::runtime(format!("getFileInfo: {}", e))
        }
    })?;

    let mut info = ValueMap::default();
    info.insert("name".to_string(), CfmlValue::string(
        path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    ));
    info.insert("size".to_string(), CfmlValue::Int(meta.len() as i64));
    info.insert("type".to_string(), CfmlValue::string(
        if meta.is_dir() { "dir".to_string() } else { "file".to_string() }
    ));
    info.insert("canRead".to_string(), CfmlValue::Bool(!meta.permissions().readonly()));
    info.insert("canWrite".to_string(), CfmlValue::Bool(!meta.permissions().readonly()));
    if let Ok(modified) = meta.modified() {
        let secs = modified.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        // Emit a CFML date string (local time) so IsDate()/date functions accept
        // it — Lucee's getFileInfo().lastmodified is a date, and callers (e.g.
        // Preside's getObjectInfo) assert IsDate() on it. A raw epoch int failed.
        use chrono::TimeZone;
        let lm = chrono::Local
            .timestamp_opt(secs as i64, 0)
            .single()
            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_default();
        info.insert("lastModified".to_string(), CfmlValue::string(lm));
    }
    Ok(CfmlValue::strukt(info))
}

fn fn_expand_path(_args: Vec<CfmlValue>) -> CfmlResult {
    // Stub — the VM intercepts expandPath to resolve the path against the
    // serve-mode webroot / CLI entry-template dir / this.mappings (it needs
    // VM state the plain builtin can't see). Registered only so the name
    // resolves as a known function; this body never runs.
    Ok(CfmlValue::string(String::new()))
}

fn fn_get_directory_from_path(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    if path.is_empty() {
        return Ok(CfmlValue::string(String::new()));
    }
    // Lucee/ACF: everything up to AND INCLUDING the final separator, verbatim —
    // NO normalization of redundant separators (Path::parent() collapses `//`,
    // which broke Wheels' $fileExistsNoCase Replace() match). Honor both / and \.
    match path.rfind(['/', '\\']) {
        Some(idx) => Ok(CfmlValue::string(path[..=idx].to_string())),
        None => Ok(CfmlValue::string(path)),
    }
}

fn fn_get_current_template_path(_args: Vec<CfmlValue>) -> CfmlResult {
    // Stub — VM intercepts this call to return the actual template path
    Ok(CfmlValue::string(String::new()))
}

fn fn_get_component_metadata(_args: Vec<CfmlValue>) -> CfmlResult {
    // Stub — VM intercepts this call to resolve component metadata
    Ok(CfmlValue::strukt(ValueMap::default()))
}

fn fn_get_component_static_scope(_args: Vec<CfmlValue>) -> CfmlResult {
    // Stub — VM intercepts this call to resolve the component's static scope
    Ok(CfmlValue::strukt(ValueMap::default()))
}

// ===============================================
// ADDITIONAL BUILTINS (Feature 3)
// ===============================================

fn fn_encode_for_url(args: Vec<CfmlValue>) -> CfmlResult {
    // GH #283: encodeForURL encodes a space as `+` (form-encoding semantics),
    // unlike urlEncodedFormat which uses `%20`.
    Ok(CfmlValue::string(url_encode_impl(&get_str(&args, 0), true)))
}

fn fn_encode_for_css(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let mut result = String::new();
    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' => result.push(c),
            _ => {
                result.push('\\');
                result.push_str(&format!("{:06X}", c as u32));
            }
        }
    }
    Ok(CfmlValue::string(result))
}

fn fn_encode_for_javascript(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let mut result = String::new();
    for c in s.chars() {
        match c {
            '\\' => result.push_str("\\\\"),
            '\'' => result.push_str("\\'"),
            '"' => result.push_str("\\\""),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '/' => result.push_str("\\/"),
            '<' => result.push_str("\\u003C"),
            '>' => result.push_str("\\u003E"),
            _ => result.push(c),
        }
    }
    Ok(CfmlValue::string(result))
}

// ===============================================
// ENCODING/DECODING FUNCTIONS
// ===============================================

fn fn_charset_decode(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let _encoding = get_str(&args, 1).to_lowercase();
    Ok(CfmlValue::Binary(s.as_bytes().to_vec()))
}

fn fn_charset_encode(args: Vec<CfmlValue>) -> CfmlResult {
    let bytes = match args.first() {
        Some(CfmlValue::Binary(b)) => b.clone(),
        // A native Java byte[] surfaces in CFML as an Array of SIGNED-byte ints
        // (e.g. `String.getBytes()`, `ByteArrayOutputStream.toByteArray()` — see
        // GH #271/#276). Lucee's charsetEncode accepts that byte[] directly; treat
        // an all-integer array as raw bytes (each masked to its low 8 bits) rather
        // than stringifying it to "72,105,...".
        Some(CfmlValue::Array(a)) => {
            let elems = a.snapshot();
            // An EMPTY byte[]-as-Array must yield "" (an empty byte run), not the
            // stringified "[]" — `iter().all()` is vacuously true for an empty
            // iterator, so the byte-mapping branch naturally produces no bytes
            // (GH #278: base32decodeString("") must be "").
            if elems.iter().all(|e| matches!(e, CfmlValue::Int(_) | CfmlValue::Double(_))) {
                elems
                    .iter()
                    .map(|e| match e {
                        CfmlValue::Int(i) => (*i & 0xFF) as u8,
                        CfmlValue::Double(d) => (*d as i64 & 0xFF) as u8,
                        _ => 0,
                    })
                    .collect()
            } else {
                CfmlValue::Array(a.clone()).as_string().into_bytes()
            }
        }
        Some(other) => other.as_string().into_bytes(),
        None => Vec::new(),
    };
    let _encoding = get_str(&args, 1).to_lowercase();
    match String::from_utf8(bytes.clone()) {
        Ok(s) => Ok(CfmlValue::string(s)),
        Err(_) => Ok(CfmlValue::string(String::from_utf8_lossy(&bytes).to_string())),
    }
}

fn fn_encode_for_html_attribute(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let mut result = String::new();
    // OWASP/ESAPI HTMLEntityCodec semantics for an attribute context: every
    // non-alphanumeric char below U+0100 is encoded — including space (&#x20;)
    // and = (&#x3d;), because an UNQUOTED attribute value can be broken out of
    // with a raw space + = — EXCEPT the ESAPI immune set `, . - _`, which are
    // inert in any attribute context and pass through. (Adobe CF and BoxLang
    // match this; Lucee 7's encodeForHTMLAttribute is the outlier — it encodes
    // almost nothing. Keeping the immune set OWASP-exact is what Wheels' asset/
    // form helpers expect, e.g. `href="/a/b.css"` → `&#x2f;a&#x2f;b.css` with the
    // dots untouched.) Named entity where one exists, else a hex numeric entity.
    // Codepoints >= U+0100 pass through (immune), mirroring ESAPI's default.
    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' => result.push(c),
            ',' | '.' | '-' | '_' => result.push(c),
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            c if (c as u32) >= 0x100 => result.push(c),
            c => result.push_str(&format!("&#x{:x};", c as u32)),
        }
    }
    Ok(CfmlValue::string(result))
}

fn fn_encode_for_xml(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    Ok(CfmlValue::string(
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;"),
    ))
}

fn fn_encode_for_xml_attribute(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let mut result = String::new();
    for c in s.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&apos;"),
            '\t' => result.push_str("&#x9;"),
            '\n' => result.push_str("&#xA;"),
            '\r' => result.push_str("&#xD;"),
            _ => result.push(c),
        }
    }
    Ok(CfmlValue::string(result))
}

fn fn_encode_for(args: Vec<CfmlValue>) -> CfmlResult {
    let encoding_type = get_str(&args, 0).to_lowercase();
    let value_args = if args.len() > 1 {
        vec![args[1].clone()]
    } else {
        vec![CfmlValue::string(String::new())]
    };
    match encoding_type.as_str() {
        "html" => fn_encode_for_html(value_args),
        "htmlattribute" => fn_encode_for_html_attribute(value_args),
        "xml" => fn_encode_for_xml(value_args),
        "xmlattribute" => fn_encode_for_xml_attribute(value_args),
        "javascript" | "js" => fn_encode_for_javascript(value_args),
        "css" => fn_encode_for_css(value_args),
        "url" => fn_encode_for_url(value_args), // ESAPI URL codec: space → `+` (GH #283)
        _ => Err(CfmlError::runtime(format!("Unsupported encoding type: {}", encoding_type))),
    }
}

fn fn_decode_for_html(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    Ok(CfmlValue::string(decode_html_entities(&s)))
}

fn decode_html_entities(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            let mut entity = String::new();
            entity.push('&');
            let mut found_semi = false;
            for _ in 0..12 {
                match chars.peek() {
                    Some(&';') => {
                        entity.push(';');
                        chars.next();
                        found_semi = true;
                        break;
                    }
                    Some(&ch) => {
                        entity.push(ch);
                        chars.next();
                    }
                    None => break,
                }
            }
            if found_semi {
                match entity.as_str() {
                    "&amp;" => result.push('&'),
                    "&lt;" => result.push('<'),
                    "&gt;" => result.push('>'),
                    "&quot;" => result.push('"'),
                    "&apos;" => result.push('\''),
                    "&#39;" => result.push('\''),
                    "&#x27;" | "&#X27;" => result.push('\''),
                    "&#x2f;" | "&#X2f;" | "&#x2F;" | "&#X2F;" => result.push('/'),
                    "&nbsp;" => result.push('\u{00A0}'),
                    _ => {
                        if entity.starts_with("&#x") || entity.starts_with("&#X") {
                            let hex_str = &entity[3..entity.len() - 1];
                            if let Ok(code) = u32::from_str_radix(hex_str, 16) {
                                if let Some(ch) = char::from_u32(code) {
                                    result.push(ch);
                                } else {
                                    result.push_str(&entity);
                                }
                            } else {
                                result.push_str(&entity);
                            }
                        } else if entity.starts_with("&#") {
                            let num_str = &entity[2..entity.len() - 1];
                            if let Ok(code) = num_str.parse::<u32>() {
                                if let Some(ch) = char::from_u32(code) {
                                    result.push(ch);
                                } else {
                                    result.push_str(&entity);
                                }
                            } else {
                                result.push_str(&entity);
                            }
                        } else {
                            result.push_str(&entity);
                        }
                    }
                }
            } else {
                result.push_str(&entity);
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn fn_decode_from_url(args: Vec<CfmlValue>) -> CfmlResult {
    fn_url_decode(args)
}

fn fn_url_encode_alias(args: Vec<CfmlValue>) -> CfmlResult {
    fn_url_encode(args)
}

fn fn_canonicalize(args: Vec<CfmlValue>) -> CfmlResult {
    let mut s = get_str(&args, 0);
    let _restrict_multiple = if args.len() > 1 {
        let v = args[1].as_string().to_lowercase();
        v == "true" || v == "yes" || v == "1"
    } else {
        false
    };
    let _restrict_mixed = if args.len() > 2 {
        let v = args[2].as_string().to_lowercase();
        v == "true" || v == "yes" || v == "1"
    } else {
        false
    };
    for _ in 0..5 {
        let prev = s.clone();
        s = decode_html_entities(&s);
        // OWASP ESAPI Canonicalize does PERCENT decoding only — it does NOT treat
        // `+` as a space (that's x-www-form-urlencoded, for urlDecode). A literal
        // `+` (e.g. a Google Fonts "Istok+Web" URL) must survive.
        s = url_decode_string_opt(&s, false);
        // ESAPI DefaultEncoder.canonicalize() runs a third default codec:
        // JavaScriptCodec. It decodes JS backslash escapes — `a\"b`->`a"b`,
        // `\x41`->`A`, `A`->`A` (GitHub #252). Without it, Canonicalize
        // over-preserved backslash escapes, so EncodeForHTMLAttribute(Canonicalize(v))
        // produced `&#x5c;&quot;` where the JVM engines produce `&quot;`.
        s = decode_javascript(&s);
        if s == prev {
            break;
        }
    }
    Ok(CfmlValue::string(s))
}

/// Decode JavaScript backslash escapes, mirroring
/// `org.owasp.esapi.codecs.JavaScriptCodec.decodeCharacter`: the single-char
/// escapes (`\b \t \n \v \f \r \" \' \\ \/ \0`), hex `\xHH`, and unicode
/// `\uHHHH`. An unrecognized or truncated escape drops the backslash and keeps
/// the following characters verbatim (ESAPI behaviour).
fn decode_javascript(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    // Read up to `n` hex digits from the iterator, consuming only valid ones.
    fn take_hex(chars: &mut std::iter::Peekable<std::str::Chars>, n: usize) -> String {
        let mut hex = String::new();
        for _ in 0..n {
            match chars.peek() {
                Some(h) if h.is_ascii_hexdigit() => {
                    hex.push(*h);
                    chars.next();
                }
                _ => break,
            }
        }
        hex
    }
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // Trailing backslash: keep it verbatim.
            None => out.push('\\'),
            Some('b') => out.push('\u{0008}'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('v') => out.push('\u{000B}'),
            Some('f') => out.push('\u{000C}'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some('0') => out.push('\u{0000}'),
            Some('x') => {
                let hex = take_hex(&mut chars, 2);
                match (hex.len() == 2)
                    .then(|| u32::from_str_radix(&hex, 16).ok())
                    .flatten()
                    .and_then(char::from_u32)
                {
                    Some(ch) => out.push(ch),
                    // Truncated/invalid: drop the backslash, keep `x` + digits.
                    None => {
                        out.push('x');
                        out.push_str(&hex);
                    }
                }
            }
            Some('u') => {
                let hex = take_hex(&mut chars, 4);
                match (hex.len() == 4)
                    .then(|| u32::from_str_radix(&hex, 16).ok())
                    .flatten()
                    .and_then(char::from_u32)
                {
                    Some(ch) => out.push(ch),
                    None => {
                        out.push('u');
                        out.push_str(&hex);
                    }
                }
            }
            // Unrecognized escape: ESAPI drops the backslash, keeps the char.
            Some(other) => out.push(other),
        }
    }
    out
}

fn url_decode_string_opt(s: &str, plus_as_space: bool) -> String {
    let mut result = String::new();
    let mut bytes = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '%' => {
                let hex: String = chars.by_ref().take(2).collect();
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    bytes.push(byte);
                }
                if chars.peek() != Some(&'%') {
                    if let Ok(decoded) = String::from_utf8(bytes.clone()) {
                        result.push_str(&decoded);
                    } else {
                        for b in &bytes { result.push(*b as char); }
                    }
                    bytes.clear();
                }
            }
            '+' if plus_as_space => {
                if !bytes.is_empty() {
                    if let Ok(decoded) = String::from_utf8(bytes.clone()) {
                        result.push_str(&decoded);
                    }
                    bytes.clear();
                }
                result.push(' ');
            }
            _ => {
                if !bytes.is_empty() {
                    if let Ok(decoded) = String::from_utf8(bytes.clone()) {
                        result.push_str(&decoded);
                    }
                    bytes.clear();
                }
                result.push(c);
            }
        }
    }
    if !bytes.is_empty() {
        if let Ok(decoded) = String::from_utf8(bytes.clone()) {
            result.push_str(&decoded);
        }
    }
    result
}

fn fn_list_reduce(_args: Vec<CfmlValue>) -> CfmlResult {
    // Needs VM closure support - stub
    Err(CfmlError::runtime("listReduce() requires VM-level closure support".to_string()))
}

fn fn_array_pop(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(CfmlValue::Array(arr)) = args.first() {
        // In-place: removes the last element from the shared array.
        match arr.with_write(|v| v.pop()) {
            Some(last) => Ok(last),
            None => Err(CfmlError::runtime("Cannot pop from empty array".to_string())),
        }
    } else {
        Err(CfmlError::runtime("arrayPop requires an array".to_string()))
    }
}

fn fn_array_shift(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(CfmlValue::Array(arr)) = args.first() {
        // In-place: removes the first element from the shared array.
        match arr.with_write(|v| if v.is_empty() { None } else { Some(v.remove(0)) }) {
            Some(first) => Ok(first),
            None => Err(CfmlError::runtime("Cannot shift from empty array".to_string())),
        }
    } else {
        Err(CfmlError::runtime("arrayShift requires an array".to_string()))
    }
}

// ===============================================
// HTTP CLIENT (cfhttp)
// ===============================================

#[cfg(feature = "http")]
/// Expand a top-level `attributeCollection` struct into the cfhttp options
/// struct. Collection keys provide the base; any attribute also supplied
/// explicitly (case-insensitively) wins, per Lucee/BoxLang semantics. No-op
/// when `arg` isn't a struct or carries no `attributeCollection`.
fn merge_cfhttp_attribute_collection(arg: CfmlValue) -> CfmlValue {
    let CfmlValue::Struct(opts) = &arg else { return arg; };
    let Some(CfmlValue::Struct(ac)) = opts.get_ci("attributeCollection") else { return arg; };
    let mut merged: ValueMap = ac.snapshot();
    for (k, v) in opts.iter() {
        if k.eq_ignore_ascii_case("attributeCollection") { continue; }
        // Explicit attribute wins: drop any same-named collection key first
        // so a differing-case duplicate can't shadow it.
        if let Some(existing) = merged.keys().find(|ek| ek.eq_ignore_ascii_case(&k)).cloned() {
            merged.shift_remove(&existing);
        }
        merged.insert(k, v);
    }
    CfmlValue::Struct(CfmlStruct::new(merged))
}

/// Whether a `getAsBinary` attribute value requests a binary response body.
/// Lucee treats "yes"/"true"/"1"/"binary"/"always" (and truthy numbers/bools)
/// as enabling binary decoding; everything else stays text.
#[cfg(feature = "http")]
fn cfhttp_get_as_binary_enabled(value: &CfmlValue) -> bool {
    match value {
        CfmlValue::Bool(b) => *b,
        CfmlValue::Int(i) => *i != 0,
        CfmlValue::Double(d) => *d != 0.0,
        CfmlValue::String(s) => matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "true" | "yes" | "1" | "binary" | "always"
        ),
        _ => false,
    }
}

/// Process-wide pooled HTTP agents so cfhttp reuses keep-alive connections
/// instead of opening (and client-closing) a fresh TCP socket per call.
///
/// A `ureq::Agent` *is* the connection pool; building a new one on every cfhttp
/// call — as the code used to — defeats keep-alive entirely. Every request then
/// connects and closes, and each client-closed socket lingers in TIME_WAIT for
/// ~2·MSL holding an ephemeral local port. Under a bursty caller (e.g. Preside
/// reindexing to a local ElasticSearch, where trashing a page fires a cascade of
/// index calls) that transiently exhausts the OS ephemeral-port range, so a run
/// of connect() calls fails with a Transport error even though the server is
/// perfectly healthy — surfacing as "made N attempts but none returned a
/// response". Sharing one pooled agent keeps a small set of connections alive and
/// reused, eliminating the churn. ureq safely retries a recycled connection on a
/// fresh socket if the server has closed it (unit.rs `send_prelude` early-retry),
/// so pooling introduces no spurious failures. Per-call timeouts are applied on
/// the `Request` (see below), so a shared agent doesn't flatten them.
#[cfg(feature = "http")]
static HTTP_AGENT: Lazy<ureq::Agent> = Lazy::new(|| {
    ureq::AgentBuilder::new()
        .max_idle_connections(100)
        .max_idle_connections_per_host(20)
        .build()
});

/// Same pool but with redirect-following disabled, for `redirect="no"` callers.
#[cfg(feature = "http")]
static HTTP_AGENT_NO_REDIRECT: Lazy<ureq::Agent> = Lazy::new(|| {
    ureq::AgentBuilder::new()
        .max_idle_connections(100)
        .max_idle_connections_per_host(20)
        .redirects(0)
        .build()
});

/// Read a response body either as decoded text or as raw bytes. Decoding bytes
/// through `into_string()` would corrupt non-UTF-8 payloads (images, etc.).
#[cfg(feature = "http")]
fn cfhttp_file_content(resp: ureq::Response, get_as_binary: bool) -> CfmlValue {
    if !get_as_binary {
        return CfmlValue::string(resp.into_string().unwrap_or_default());
    }

    let mut reader = resp.into_reader();
    let mut body = Vec::new();
    if std::io::Read::read_to_end(&mut reader, &mut body).is_err() {
        body.clear();
    }
    CfmlValue::Binary(body)
}

#[cfg(feature = "http")]
fn fn_cfhttp(args: Vec<CfmlValue>) -> CfmlResult {
    use std::collections::HashMap;

    let arg = merge_cfhttp_attribute_collection(args.into_iter().next().unwrap_or(CfmlValue::Null));

    // Parse arguments: either a URL string or an options struct
    let (mut url, method, headers, body, timeout_secs, throw_on_error, follow_redirects, encode_url, port, proxy_server, proxy_port, get_as_binary) = match &arg {
        CfmlValue::String(url) => ((**url).clone(), "GET".to_string(), HashMap::<String, String>::new(), None::<Vec<u8>>, 30u64, false, true, true, None::<u16>, None::<String>, None::<u16>, false),
        CfmlValue::Struct(opts) => {
            let mut url = opts.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("url"))
                .map(|(_, v)| v.as_string())
                .unwrap_or_default();
            let method = opts.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("method"))
                .map(|(_, v)| v.as_string().to_uppercase())
                .unwrap_or_else(|| "GET".to_string());
            let mut hdrs: HashMap<String, String> = opts.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("headers"))
                .and_then(|(_,v)| if let CfmlValue::Struct(h) = v {
                    Some(h.iter().map(|(k, v)| (k.clone(), v.as_string())).collect())
                } else { None })
                .unwrap_or_default();
            // Process cfhttpparam params array
            if let Some((_, CfmlValue::Array(params))) = opts.iter().find(|(k, _)| k.eq_ignore_ascii_case("params")) {
                for param in params.iter() {
                    if let CfmlValue::Struct(p) = param {
                        let ptype = p.iter().find(|(k, _)| k.eq_ignore_ascii_case("type"))
                            .map(|(_, v)| v.as_string().to_lowercase()).unwrap_or_default();
                        let pname = p.iter().find(|(k, _)| k.eq_ignore_ascii_case("name"))
                            .map(|(_, v)| v.as_string()).unwrap_or_default();
                        let pvalue = p.iter().find(|(k, _)| k.eq_ignore_ascii_case("value"))
                            .map(|(_, v)| v.as_string()).unwrap_or_default();
                        match ptype.as_str() {
                            "header" => { hdrs.insert(pname, pvalue); }
                            "cookie" => { hdrs.entry("Cookie".to_string()).and_modify(|v| { v.push_str(&format!("; {}={}", pname, pvalue)); }).or_insert(format!("{}={}", pname, pvalue)); }
                            "url" => {
                                let sep = if url.contains('?') { "&" } else { "?" };
                                url = format!("{}{}{}={}", url, sep, pname, pvalue);
                            }
                            _ => {} // formfield, body, xml, file handled below
                        }
                    }
                }
            }
            // Explicit body attr (string) — wins over param-built body.
            let explicit_body: Option<Vec<u8>> = opts.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("body"))
                .and_then(|(_, v)| if matches!(v, CfmlValue::Null) { None } else { Some(v.as_string().into_bytes()) });

            // Multipart attr: true/yes/"true"/"yes" → opt-in. A type="file"
            // cfhttpparam also forces multipart (Lucee parity — you can't send
            // file uploads as urlencoded).
            let multipart_attr = opts.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("multipart"))
                .map(|(_, v)| match v {
                    CfmlValue::Bool(b) => b,
                    CfmlValue::String(s) => s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("yes"),
                    _ => false,
                })
                .unwrap_or(false);
            let has_file_param = if let Some((_, CfmlValue::Array(params))) = opts.iter().find(|(k, _)| k.eq_ignore_ascii_case("params")) {
                params.iter().any(|p| {
                    if let CfmlValue::Struct(s) = p {
                        s.iter().any(|(k, v)| k.eq_ignore_ascii_case("type") && v.as_string().eq_ignore_ascii_case("file"))
                    } else { false }
                })
            } else { false };
            let use_multipart = multipart_attr || has_file_param;

            // Build body from cfhttpparam params if no explicit body attr.
            let body: Option<Vec<u8>> = if explicit_body.is_none() {
                if let Some((_, CfmlValue::Array(params))) = opts.iter().find(|(k, _)| k.eq_ignore_ascii_case("params")) {
                    if use_multipart {
                        // Generate a boundary unlikely to collide with body bytes.
                        // Combines a fixed prefix, the request URL hash, and a
                        // per-process atomic counter — sufficient for uniqueness
                        // without pulling in `rand`.
                        use std::sync::atomic::{AtomicU64, Ordering};
                        static MULTIPART_SEQ: AtomicU64 = AtomicU64::new(0);
                        let seq = MULTIPART_SEQ.fetch_add(1, Ordering::Relaxed);
                        let boundary = format!("----RustCFMLBoundary{:016x}{:016x}", seq, url.len() as u64);

                        let mut buf: Vec<u8> = Vec::new();
                        let dashes = b"--";
                        let crlf = b"\r\n";
                        for param in params.iter() {
                            let p = match param { CfmlValue::Struct(s) => s, _ => continue };
                            let ptype = p.iter().find(|(k, _)| k.eq_ignore_ascii_case("type"))
                                .map(|(_, v)| v.as_string().to_lowercase()).unwrap_or_default();
                            let pname = p.iter().find(|(k, _)| k.eq_ignore_ascii_case("name"))
                                .map(|(_, v)| v.as_string()).unwrap_or_default();
                            match ptype.as_str() {
                                "formfield" => {
                                    let pvalue = p.iter().find(|(k, _)| k.eq_ignore_ascii_case("value"))
                                        .map(|(_, v)| v.as_string()).unwrap_or_default();
                                    buf.extend_from_slice(dashes);
                                    buf.extend_from_slice(boundary.as_bytes());
                                    buf.extend_from_slice(crlf);
                                    buf.extend_from_slice(format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", pname).as_bytes());
                                    buf.extend_from_slice(pvalue.as_bytes());
                                    buf.extend_from_slice(crlf);
                                }
                                "file" => {
                                    let file_path = p.iter().find(|(k, _)| k.eq_ignore_ascii_case("file"))
                                        .map(|(_, v)| v.as_string()).unwrap_or_default();
                                    let mime = p.iter().find(|(k, _)| k.eq_ignore_ascii_case("mimetype"))
                                        .map(|(_, v)| v.as_string())
                                        .filter(|s| !s.is_empty())
                                        .unwrap_or_else(|| "application/octet-stream".to_string());
                                    let file_bytes = match std::fs::read(&file_path) {
                                        Ok(b) => b,
                                        Err(e) => return Err(CfmlError::runtime(format!(
                                            "cfhttp: failed to read file '{}' for multipart upload: {}", file_path, e
                                        ))),
                                    };
                                    let filename = std::path::Path::new(&file_path)
                                        .file_name()
                                        .map(|s| s.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| pname.clone());
                                    buf.extend_from_slice(dashes);
                                    buf.extend_from_slice(boundary.as_bytes());
                                    buf.extend_from_slice(crlf);
                                    buf.extend_from_slice(format!(
                                        "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
                                        pname, filename, mime
                                    ).as_bytes());
                                    buf.extend_from_slice(&file_bytes);
                                    buf.extend_from_slice(crlf);
                                }
                                _ => {}
                            }
                        }
                        if !buf.is_empty() {
                            buf.extend_from_slice(dashes);
                            buf.extend_from_slice(boundary.as_bytes());
                            buf.extend_from_slice(dashes);
                            buf.extend_from_slice(crlf);
                            hdrs.entry("Content-Type".to_string())
                                .or_insert(format!("multipart/form-data; boundary={}", boundary));
                            Some(buf)
                        } else {
                            None
                        }
                    } else {
                        let mut form_parts = Vec::new();
                        let mut xml_body = None;
                        for param in params.iter() {
                            if let CfmlValue::Struct(p) = param {
                                let ptype = p.iter().find(|(k, _)| k.eq_ignore_ascii_case("type"))
                                    .map(|(_, v)| v.as_string().to_lowercase()).unwrap_or_default();
                                let pname = p.iter().find(|(k, _)| k.eq_ignore_ascii_case("name"))
                                    .map(|(_, v)| v.as_string()).unwrap_or_default();
                                let pvalue = p.iter().find(|(k, _)| k.eq_ignore_ascii_case("value"))
                                    .map(|(_, v)| v.as_string()).unwrap_or_default();
                                match ptype.as_str() {
                                    "formfield" => form_parts.push(format!("{}={}", pname, pvalue)),
                                    "body" => xml_body = Some(pvalue),
                                    "xml" => xml_body = Some(pvalue),
                                    _ => {}
                                }
                            }
                        }
                        if let Some(xml) = xml_body {
                            Some(xml.into_bytes())
                        } else if !form_parts.is_empty() {
                            hdrs.entry("Content-Type".to_string()).or_insert("application/x-www-form-urlencoded".to_string());
                            Some(form_parts.join("&").into_bytes())
                        } else {
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                explicit_body
            };
            let timeout = opts.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("timeout"))
                .map(|(_, v)| match v { CfmlValue::Int(i) => i as u64, CfmlValue::Double(d) => d as u64, CfmlValue::String(s) => s.parse().unwrap_or(30), _ => 30 })
                .unwrap_or(30);

            // username/password -> Basic Auth
            let username = opts.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("username"))
                .map(|(_, v)| v.as_string());
            let password = opts.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("password"))
                .map(|(_, v)| v.as_string());
            if let (Some(ref user), Some(ref pass)) = (&username, &password) {
                if !user.is_empty() {
                    let credentials = format!("{}:{}", user, pass);
                    let encoded = base64_encode_bytes(credentials.as_bytes());
                    hdrs.entry("Authorization".to_string()).or_insert(format!("Basic {}", encoded));
                }
            }

            // useragent -> User-Agent header
            if let Some((_, v)) = opts.iter().find(|(k, _)| k.eq_ignore_ascii_case("useragent")) {
                let ua = v.as_string();
                if !ua.is_empty() {
                    hdrs.entry("User-Agent".to_string()).or_insert(ua);
                }
            }

            // throwonerror
            let throw_on_error = opts.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("throwonerror"))
                .map(|(_, v)| match v {
                    CfmlValue::Bool(b) => b,
                    CfmlValue::String(s) => s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("yes"),
                    _ => false,
                })
                .unwrap_or(false);

            // redirect
            let follow_redirects = opts.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("redirect"))
                .map(|(_, v)| match v {
                    CfmlValue::Bool(b) => b,
                    CfmlValue::String(s) => !s.eq_ignore_ascii_case("false") && !s.eq_ignore_ascii_case("no"),
                    _ => true,
                })
                .unwrap_or(true);

            // encodeurl (default true)
            let encode_url = opts.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("encodeurl"))
                .map(|(_, v)| match v {
                    CfmlValue::Bool(b) => b,
                    CfmlValue::String(s) => !s.eq_ignore_ascii_case("false") && !s.eq_ignore_ascii_case("no"),
                    _ => true,
                })
                .unwrap_or(true);

            // port
            let port = opts.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("port"))
                .and_then(|(_, v)| match v {
                    CfmlValue::Int(i) => Some(i as u16),
                    CfmlValue::Double(d) => Some(d as u16),
                    CfmlValue::String(s) => s.parse::<u16>().ok(),
                    _ => None,
                });

            // proxyserver / proxyport
            let proxy_server = opts.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("proxyserver"))
                .map(|(_, v)| v.as_string());
            let proxy_port = opts.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("proxyport"))
                .and_then(|(_, v)| match v {
                    CfmlValue::Int(i) => Some(i as u16),
                    CfmlValue::Double(d) => Some(d as u16),
                    CfmlValue::String(s) => s.parse::<u16>().ok(),
                    _ => None,
                });

            let get_as_binary = opts.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("getasbinary"))
                .map(|(_, v)| cfhttp_get_as_binary_enabled(&v))
                .unwrap_or(false);

            (url, method, hdrs, body, timeout, throw_on_error, follow_redirects, encode_url, port, proxy_server, proxy_port, get_as_binary)
        }
        _ => return Err(CfmlError::runtime("cfhttp requires a URL string or options struct".to_string())),
    };

    if url.is_empty() {
        return Err(CfmlError::runtime("cfhttp: url is required".to_string()));
    }

    // Apply port to URL if specified and not already present
    if let Some(port_num) = port {
        if let Some(scheme_end) = url.find("://") {
            let after_scheme = &url[scheme_end + 3..];
            let host_end = after_scheme.find('/').unwrap_or(after_scheme.len());
            let host_part = &after_scheme[..host_end];
            if !host_part.contains(':') {
                let insert_pos = scheme_end + 3 + host_end;
                url.insert_str(insert_pos, &format!(":{}", port_num));
            }
        }
    }

    // URL-encode unsafe chars in path/query when encodeurl is true
    if encode_url {
        if let Some(scheme_end) = url.find("://") {
            let after_scheme = &url[scheme_end + 3..];
            if let Some(path_start) = after_scheme.find('/') {
                let host = url[..scheme_end + 3 + path_start].to_string();
                let path_and_query = &url[scheme_end + 3 + path_start..];
                let encoded: String = path_and_query.chars().map(|c| {
                    match c {
                        ' ' => "%20".to_string(),
                        '{' => "%7B".to_string(),
                        '}' => "%7D".to_string(),
                        '|' => "%7C".to_string(),
                        '^' => "%5E".to_string(),
                        '[' => "%5B".to_string(),
                        ']' => "%5D".to_string(),
                        '`' => "%60".to_string(),
                        _ => c.to_string(),
                    }
                }).collect();
                url = format!("{}{}", host, encoded);
            }
        }
    }

    // Reuse a process-wide pooled agent (keep-alive) for the common no-proxy
    // case; only a proxy — which is per-call configuration — forces a one-off
    // agent. This is what keeps cfhttp from churning a fresh TCP connection per
    // call (see HTTP_AGENT above). The one-off agent is held in `owned_agent` so
    // it outlives the borrow below.
    let use_proxy = proxy_server.as_deref().map(|h| !h.is_empty()).unwrap_or(false);
    let owned_agent;
    let agent: &ureq::Agent = if use_proxy {
        let mut agent_builder = ureq::AgentBuilder::new()
            .max_idle_connections_per_host(20);
        if !follow_redirects {
            agent_builder = agent_builder.redirects(0);
        }
        if let Some(ref proxy_host) = proxy_server {
            let proxy_url = if let Some(pp) = proxy_port {
                format!("http://{}:{}", proxy_host, pp)
            } else {
                format!("http://{}", proxy_host)
            };
            if let Ok(proxy) = ureq::Proxy::new(&proxy_url) {
                agent_builder = agent_builder.proxy(proxy);
            }
        }
        owned_agent = agent_builder.build();
        &owned_agent
    } else if follow_redirects {
        &HTTP_AGENT
    } else {
        &HTTP_AGENT_NO_REDIRECT
    };

    let mut request = match method.as_str() {
        "GET" => agent.get(&url),
        "POST" => agent.post(&url),
        "PUT" => agent.put(&url),
        "DELETE" => agent.delete(&url),
        "PATCH" => agent.request("PATCH", &url),
        "HEAD" => agent.head(&url),
        "OPTIONS" => agent.request("OPTIONS", &url),
        _ => agent.get(&url),
    };

    // Per-call timeout lives on the Request now (the shared agent has none), so
    // each cfhttp still honours its own `timeout` attribute.
    request = request.timeout(std::time::Duration::from_secs(timeout_secs));

    for (k, v) in &headers {
        request = request.set(k, v);
    }

    let response = if let Some(body_bytes) = &body {
        request.send_bytes(body_bytes)
    } else {
        request.call()
    };

    let mut result_struct: ValueMap = ValueMap::default();

    match response {
        Ok(resp) => {
            let status = resp.status();
            let status_text = resp.status_text().to_string();
            let http_version = resp.http_version().to_string();
            let content_type = resp.content_type().to_string();

            let mut resp_headers: ValueMap = ValueMap::default();
            for name in resp.headers_names() {
                if let Some(val) = resp.header(&name) {
                    resp_headers.insert(name, CfmlValue::string(val.to_string()));
                }
            }
            // ACF/Lucee inject the status into responseHeader itself (numeric
            // status_code + explanation), alongside the real HTTP headers. Lots
            // of CFML reads result.responseHeader.status_code (e.g. Preside's
            // ElasticSearchApiWrapper success check); without these keys that
            // read is empty and the caller thinks the request failed.
            resp_headers.insert("status_code".to_string(), CfmlValue::Int(status as i64));
            resp_headers.insert("explanation".to_string(), CfmlValue::string(status_text.clone()));

            let file_content = cfhttp_file_content(resp, get_as_binary);

            let (mime, charset) = parse_content_type(&content_type);

            if throw_on_error && status >= 400 {
                return Err(CfmlError::runtime(format!("cfhttp request failed: {} {}", status, status_text)));
            }

            result_struct.insert("statusCode".to_string(), CfmlValue::string(format!("{} {}", status, status_text)));
            result_struct.insert("status_code".to_string(), CfmlValue::Int(status as i64));
            result_struct.insert("statusText".to_string(), CfmlValue::string(status_text.clone()));
            result_struct.insert("status_text".to_string(), CfmlValue::string(status_text));
            result_struct.insert("fileContent".to_string(), file_content);
            result_struct.insert("mimeType".to_string(), CfmlValue::string(mime));
            result_struct.insert("charset".to_string(), CfmlValue::string(charset));
            result_struct.insert("responseHeader".to_string(), CfmlValue::strukt(resp_headers));
            result_struct.insert("errorDetail".to_string(), CfmlValue::string(String::new()));
            result_struct.insert("HTTP_Version".to_string(), CfmlValue::string(http_version));
        }
        Err(ureq::Error::Status(code, resp)) => {
            let status_text = resp.status_text().to_string();

            if throw_on_error {
                return Err(CfmlError::runtime(format!("cfhttp request failed: {} {}", code, status_text)));
            }

            let http_version = resp.http_version().to_string();
            let content_type = resp.content_type().to_string();

            let mut resp_headers: ValueMap = ValueMap::default();
            for name in resp.headers_names() {
                if let Some(val) = resp.header(&name) {
                    resp_headers.insert(name, CfmlValue::string(val.to_string()));
                }
            }
            // Match ACF/Lucee: status also lives inside responseHeader (see the
            // success branch above).
            resp_headers.insert("status_code".to_string(), CfmlValue::Int(code as i64));
            resp_headers.insert("explanation".to_string(), CfmlValue::string(status_text.clone()));

            let file_content = cfhttp_file_content(resp, get_as_binary);
            let (mime, charset) = parse_content_type(&content_type);

            result_struct.insert("statusCode".to_string(), CfmlValue::string(format!("{} {}", code, status_text)));
            result_struct.insert("status_code".to_string(), CfmlValue::Int(code as i64));
            result_struct.insert("statusText".to_string(), CfmlValue::string(status_text.clone()));
            result_struct.insert("status_text".to_string(), CfmlValue::string(status_text));
            result_struct.insert("fileContent".to_string(), file_content);
            result_struct.insert("mimeType".to_string(), CfmlValue::string(mime));
            result_struct.insert("charset".to_string(), CfmlValue::string(charset));
            result_struct.insert("responseHeader".to_string(), CfmlValue::strukt(resp_headers));
            result_struct.insert("errorDetail".to_string(), CfmlValue::string(String::new()));
            result_struct.insert("HTTP_Version".to_string(), CfmlValue::string(http_version));
        }
        Err(ureq::Error::Transport(e)) => {
            result_struct.insert("statusCode".to_string(), CfmlValue::string("0".to_string()));
            result_struct.insert("status_code".to_string(), CfmlValue::Int(0));
            result_struct.insert("statusText".to_string(), CfmlValue::string(String::new()));
            result_struct.insert("status_text".to_string(), CfmlValue::string(String::new()));
            result_struct.insert("fileContent".to_string(), CfmlValue::string(String::new()));
            result_struct.insert("mimeType".to_string(), CfmlValue::string(String::new()));
            result_struct.insert("charset".to_string(), CfmlValue::string("UTF-8".to_string()));
            result_struct.insert("responseHeader".to_string(), CfmlValue::strukt(ValueMap::default()));
            result_struct.insert("errorDetail".to_string(), CfmlValue::string(e.to_string()));
            result_struct.insert("HTTP_Version".to_string(), CfmlValue::string(String::new()));

            if throw_on_error {
                return Err(CfmlError::runtime(format!("cfhttp connection failed: {}", e)));
            }
        }
    }

    Ok(CfmlValue::strukt(result_struct))
}

#[cfg(feature = "http")]
fn parse_content_type(ct: &str) -> (String, String) {
    let parts: Vec<&str> = ct.splitn(2, ';').collect();
    let mime = parts[0].trim().to_string();
    let charset = if parts.len() > 1 {
        parts[1]
            .split('=')
            .nth(1)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "UTF-8".to_string())
    } else {
        "UTF-8".to_string()
    };
    (mime, charset)
}

// ===============================================
// DATABASE (queryExecute)
// ===============================================

#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
pub(crate) enum DbDriver {
    Sqlite(String),
    Mysql(String),
    Postgres(String),
    Mssql(String),
}

/// Read the `datasource` option value as a connection string. Usually a plain
/// string (a logical name or a connection URL), but CFML also accepts an
/// INLINE datasource STRUCT — the ACF/Lucee `{ class:"org.sqlite.JDBC",
/// connectionString:"jdbc:sqlite::memory:" }` form. For a struct we read its
/// `connectionString` key (falling back to a `database`/`url` key) rather than
/// stringifying the whole struct — otherwise `as_string()` produced a filename
/// like `{class: org.sqlite.JDBC, connectionString: jdbc:sqlite::memory:}` that
/// SQLite then created as a literal on-disk file, losing the `:memory:` intent.
pub(crate) fn datasource_attr_string(v: &CfmlValue) -> String {
    match v {
        CfmlValue::Struct(ds) => {
            for key in ["connectionString", "url", "database"] {
                if let Some((_, cs)) = ds.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
                    let s = cs.as_string();
                    if !s.is_empty() {
                        return s;
                    }
                }
            }
            v.as_string()
        }
        _ => v.as_string(),
    }
}

#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
pub(crate) fn parse_datasource(ds: &str) -> DbDriver {
    // JDBC URL normalisation. `this.datasources` (Lucee/ACF/BoxLang style) and
    // cfconfig `connectionString`s commonly use `jdbc:<subprotocol>:<rest>`
    // forms, e.g. `jdbc:sqlite:/path/to.db`. Map them onto the native driver
    // URLs the rest of this layer understands. (sqlserver uses a different
    // `;`-delimited syntax and is left to the existing `mssql://` form.)
    if let Some(rest) = ds.strip_prefix("jdbc:") {
        if let Some(path) = rest.strip_prefix("sqlite:") {
            return DbDriver::Sqlite(path.to_string());
        }
        if rest.starts_with("postgresql://")
            || rest.starts_with("postgres://")
            || rest.starts_with("mysql://")
            || rest.starts_with("mariadb://")
        {
            return parse_datasource(rest);
        }
    }
    if ds.starts_with("mysql://") {
        DbDriver::Mysql(ds.to_string())
    } else if let Some(rest) = ds.strip_prefix("mariadb://") {
        DbDriver::Mysql(format!("mysql://{}", rest))
    } else if ds.starts_with("postgresql://") || ds.starts_with("postgres://") {
        DbDriver::Postgres(ds.to_string())
    } else if ds.starts_with("mssql://") || ds.starts_with("sqlserver://") {
        DbDriver::Mssql(ds.to_string())
    } else if ds.starts_with("sqlite://") {
        DbDriver::Sqlite(ds[9..].to_string())
    } else {
        DbDriver::Sqlite(ds.to_string()) // :memory: or file path
    }
}

/// Does `s` look like an explicit connection string (rather than a bare
/// logical datasource name)? Used to decide whether an unresolved datasource
/// is a misconfiguration or a literal connection the caller passed inline.
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn looks_like_connection_string(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.contains("://")
        || l.starts_with("jdbc:")
        || l.contains(":memory:")
        || l.contains('/')
        || l.contains('\\')
        || l.ends_with(".db")
        || l.ends_with(".sqlite")
}

/// Resolve a datasource *name* to a connection string, erroring instead of
/// silently substituting a throwaway in-memory SQLite database when the name
/// maps to no registered datasource, dynamic driver, or explicit connection
/// string. Without this guard a config typo (or an unrecognised driver key)
/// "works" against the wrong database — see GitHub #173.
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
pub(crate) fn resolve_query_datasource(name: &str) -> Result<String, CfmlError> {
    // Dynamic drivers (e.g. Cloudflare D1) own their own name resolution.
    if crate::db_driver::lookup_dynamic_datasource(name).is_some() {
        return Ok(name.to_string());
    }
    let resolved = resolve_datasource(name);
    // `resolve_datasource` returns the name unchanged when it is not registered.
    if resolved.eq_ignore_ascii_case(name) && !looks_like_connection_string(&resolved) {
        // Lucee/ACF raise a catchable `database`-typed exception for an unknown
        // datasource (NOT a generic runtime error), so `cftry { ... } cfcatch
        // type="database"` probes work. Preside's test-suite Application.cfc
        // (`_dsnExists()` → `dbinfo` wrapped in `catch( database e )`) relies on
        // exactly this to detect whether its datasource has been registered yet.
        return Err(CfmlError::database(format!(
            "datasource [{}] could not be found. Define it in this.datasources or .cfconfig.json, or pass a connection string.",
            name
        )));
    }
    Ok(resolved)
}

// -----------------------------------------------
// Datasource Registry
// -----------------------------------------------
//
// Named datasources from `.cfconfig.json` resolve to connection URLs through
// this global registry. cfquery / queryExecute consult the registry before
// falling through to bare-string parsing, so `datasource="myDSN"` works the
// same way it does in Lucee/ACF. Populated once by the CLI at startup.

use std::sync::{Mutex, OnceLock};

static DATASOURCE_REGISTRY: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

// -----------------------------------------------
// Default Mail Server
// -----------------------------------------------
//
// cfconfig's first `mailServers` entry becomes the process-wide default that
// cfmail falls back to when its tag attributes omit `server`. Populated by
// the CLI at startup; consumed inside fn_cfmail.

#[derive(Clone, Debug, Default)]
pub struct DefaultMailServer {
    pub server: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub tls: bool,
    pub ssl: bool,
    pub timeout: u32,
}

static DEFAULT_MAIL_SERVER: OnceLock<Mutex<Option<DefaultMailServer>>> = OnceLock::new();

/// Register the default SMTP server. Replaces any previous default.
pub fn set_default_mail_server(server: DefaultMailServer) {
    let m = DEFAULT_MAIL_SERVER.get_or_init(|| Mutex::new(None));
    *m.lock().unwrap() = Some(server);
}

/// Look up the registered default. Returns `None` if no cfconfig mailServers
/// entry was present at startup, in which case cfmail's pre-existing
/// "no SMTP Server defined" error fires.
pub fn default_mail_server() -> Option<DefaultMailServer> {
    DEFAULT_MAIL_SERVER
        .get()
        .and_then(|m| m.lock().unwrap().clone())
}

// -----------------------------------------------
// Security flags from cfconfig
// -----------------------------------------------
//
// CSRF can be disabled outright. serializeJSON output can be wrapped with a
// hijack-prevention prefix. Set once at startup by the CLI; reads from the
// builtins are lock-free after init.

#[derive(Clone, Debug, Default)]
pub struct SecurityFlags {
    pub csrf_enabled: bool,
    pub secure_json: bool,
    pub secure_json_prefix: String,
}

impl SecurityFlags {
    pub const fn defaults() -> Self {
        Self {
            csrf_enabled: true,
            secure_json: false,
            secure_json_prefix: String::new(),
        }
    }
}

static SECURITY_FLAGS: OnceLock<Mutex<SecurityFlags>> = OnceLock::new();

pub fn set_security_flags(flags: SecurityFlags) {
    let m = SECURITY_FLAGS.get_or_init(|| Mutex::new(SecurityFlags::defaults()));
    *m.lock().unwrap() = flags;
}

pub fn security_flags() -> SecurityFlags {
    SECURITY_FLAGS
        .get()
        .map(|m| m.lock().unwrap().clone())
        .unwrap_or_else(SecurityFlags::defaults)
}

/// Register a name → connection-URL mapping. Names are stored lowercased so
/// lookups are case-insensitive (matching CFML's general identifier handling).
/// Re-registering an existing name overwrites it.
pub fn register_datasource(name: &str, url: String) {
    let m = DATASOURCE_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    m.lock().unwrap().insert(name.to_lowercase(), url);
}

/// Mark `name` as the default datasource (used when cfquery has no
/// `datasource` attribute). Stored under the reserved key `""`.
pub fn set_default_datasource(url: String) {
    let m = DATASOURCE_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    m.lock().unwrap().insert(String::new(), url);
}

/// Per-datasource connection-acquire timeout overrides, keyed by the resolved
/// connection URL (the same key the pool builders cache on). Populated from the
/// `connectionTimeout` config key in `.cfconfig.json` / `this.datasources` so an
/// app can make an unreachable datasource fail fast — or wait longer — than the
/// default. Absent => the pool builder's built-in default applies.
static DATASOURCE_TIMEOUTS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();

/// Default connection-acquire timeout (seconds) for the networked DB pools
/// (PostgreSQL, MSSQL). Lowered from 30s so an unreachable / misconfigured
/// datasource fails a request in seconds instead of stalling it for half a
/// minute — r2d2 retries a doomed connection with backoff until this elapses.
/// Override per-datasource with the `connectionTimeout` config key.
#[cfg(any(feature = "postgres_db", feature = "mssql_db"))]
const DEFAULT_DB_CONNECTION_TIMEOUT_SECS: u64 = 5;

/// Record a per-datasource connection-acquire timeout (seconds) for `url`.
/// `0` is ignored (treated as "unset" — the pool default applies). Re-recording
/// an existing url overwrites it.
pub fn register_datasource_timeout(url: &str, secs: u32) {
    if secs == 0 {
        return;
    }
    let m = DATASOURCE_TIMEOUTS.get_or_init(|| Mutex::new(HashMap::new()));
    m.lock().unwrap().insert(url.to_string(), secs as u64);
}

/// Builtin bridge letting the VM register a per-datasource connection timeout
/// without a hard cfml-vm -> cfml-stdlib dependency (wasm builds omit stdlib).
/// Args: `[url: String, secs: Int]`. Registered as `__register_ds_timeout`.
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn fn_register_ds_timeout(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        let url = args[0].as_string();
        let secs = match &args[1] {
            CfmlValue::Int(i) => *i,
            CfmlValue::Double(d) => *d as i64,
            other => other.as_string().trim().parse().unwrap_or(0),
        };
        if secs > 0 {
            register_datasource_timeout(&url, secs as u32);
        }
    }
    Ok(CfmlValue::Null)
}

/// Resolve the connection-acquire timeout for a connection URL: the registered
/// per-datasource override if present, else the networked-pool default.
#[cfg(any(feature = "postgres_db", feature = "mssql_db"))]
fn datasource_timeout(url: &str) -> std::time::Duration {
    let secs = DATASOURCE_TIMEOUTS
        .get()
        .and_then(|m| m.lock().unwrap().get(url).copied())
        .unwrap_or(DEFAULT_DB_CONNECTION_TIMEOUT_SECS);
    std::time::Duration::from_secs(secs)
}

/// Socket-level network timeouts for pooled DB connections (GitHub #302).
///
/// These are DISTINCT from [`datasource_timeout`], which is the *pool checkout*
/// timeout — how long to wait for a free connection from the pool. Nothing here
/// previously bounded the socket itself, so a connection that had been silently
/// black-holed (NAT/pooler dropping an idle TCP session without sending a RST,
/// or a database wedged by host contention) would block a request until the OS
/// TCP timeout — minutes — with the process sitting at 0% CPU. That is the
/// latency source behind the #302 boot race, and it is reproducible locally just
/// by loading the machine the database runs on.
///
/// Defaults are chosen so they cannot break a legitimately slow query:
///
/// * **connect timeout** (default 10s) bounds only connection establishment.
/// * **keepalive** (default 30s idle) is what actually detects a black-holed
///   peer — probes fail and the socket errors out instead of hanging forever.
/// * **read/write timeout** is **off by default**, deliberately. It bounds an
///   individual socket read, and a long-running statement (a Preside schema
///   migration, a big report query) sends nothing until it finishes — so any
///   non-zero value would abort exactly those queries. Enable it only when you
///   know your workload's ceiling.
///
/// Each is overridable by env var; `0` disables that individual timeout.
#[cfg(any(feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn db_net_secs(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

/// Bounds TCP connection establishment only. Safe to default on.
#[cfg(any(feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn db_connect_timeout() -> Option<std::time::Duration> {
    let s = db_net_secs("RUSTCFML_DB_CONNECT_TIMEOUT_SECS", 10);
    (s > 0).then(|| std::time::Duration::from_secs(s))
}

/// Idle time before TCP keepalive probing starts. The main defence against a
/// silently-dropped connection; does not affect in-flight queries.
#[cfg(any(feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn db_keepalive() -> Option<std::time::Duration> {
    let s = db_net_secs("RUSTCFML_DB_KEEPALIVE_SECS", 30);
    (s > 0).then(|| std::time::Duration::from_secs(s))
}

/// Per-read/per-write socket timeout. OFF by default — see the note above about
/// long-running statements.
#[cfg(any(feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn db_socket_timeout() -> Option<std::time::Duration> {
    let s = db_net_secs("RUSTCFML_DB_SOCKET_TIMEOUT_SECS", 0);
    (s > 0).then(|| std::time::Duration::from_secs(s))
}

/// Resolve a datasource identifier to a connection URL. If `name` matches a
/// registered datasource, return its URL; otherwise return `name` unchanged
/// so callers can still pass raw `mysql://...` strings.
pub fn resolve_datasource(name: &str) -> String {
    if let Some(m) = DATASOURCE_REGISTRY.get() {
        if let Some(url) = m.lock().unwrap().get(&name.to_lowercase()) {
            return url.clone();
        }
    }
    name.to_string()
}

/// Resolve the default datasource registered via `set_default_datasource`.
/// Only called from `fn_query_execute`, which carries the same gate — without
/// it, no-DB builds (e.g. wasm32 cfml-worker) flag this as dead code.
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
pub(crate) fn default_datasource() -> Option<String> {
    DATASOURCE_REGISTRY
        .get()
        .and_then(|m| m.lock().unwrap().get("").cloned())
}

// -----------------------------------------------
// Connection Pool Manager
// -----------------------------------------------

/// Global pool manager — maps datasource URL → pool instance (type-erased)
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
static POOL_MANAGER: OnceLock<Mutex<HashMap<String, Box<dyn std::any::Any + Send>>>> = OnceLock::new();

#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn get_pool_manager() -> &'static Mutex<HashMap<String, Box<dyn std::any::Any + Send>>> {
    POOL_MANAGER.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(feature = "sqlite")]
struct SqliteConnectionManager {
    path: String,
}

#[cfg(feature = "sqlite")]
impl r2d2::ManageConnection for SqliteConnectionManager {
    type Connection = rusqlite::Connection;
    type Error = rusqlite::Error;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        rusqlite::Connection::open(&self.path)
    }

    fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
        conn.execute_batch("SELECT 1").map_err(Into::into)
    }

    fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
        false
    }
}

#[cfg(feature = "sqlite")]
fn get_sqlite_pool(path: &str) -> Result<r2d2::Pool<SqliteConnectionManager>, CfmlError> {
    let mut manager = get_pool_manager().lock().unwrap();
    let key = format!("sqlite:{}", path);
    if let Some(pool_any) = manager.get(&key) {
        if let Some(pool) = pool_any.downcast_ref::<r2d2::Pool<SqliteConnectionManager>>() {
            return Ok(pool.clone());
        }
    }
    // Ensure the parent directory of a file-backed SQLite DB exists before
    // opening: SQLite creates the file itself but NOT missing intermediate
    // directories, so a full path like `/data/app.db` where `/data` doesn't yet
    // exist fails with a cryptic "unable to open database file". Create the
    // directory chain up front so a configured path just works. In-memory and
    // other special (`:...`) targets have no filesystem parent — skip them.
    if !path.is_empty() && !path.starts_with(':') && !path.contains("mode=memory") {
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return Err(CfmlError::database(format!(
                        "queryExecute: cannot create SQLite database directory '{}': {}",
                        parent.display(),
                        e
                    )));
                }
            }
        }
    }
    let mgr = SqliteConnectionManager { path: path.to_string() };
    let pool = r2d2::Pool::builder()
        .max_size(10)
        .min_idle(Some(1))
        .connection_timeout(std::time::Duration::from_secs(30))
        .build(mgr)
        .map_err(|e| CfmlError::database(format!("queryExecute: failed to create SQLite pool: {}", e)))?;
    manager.insert(key, Box::new(pool.clone()));
    Ok(pool)
}

/// Resolve a MySQL TLS configuration from the connection URL's query string,
/// returning a sanitised URL (with the TLS-related keys removed, since the
/// `mysql` crate's `Opts::from_url` errors on parameters it doesn't recognise)
/// and the `SslOpts` to apply, if any.
///
/// Recognised keys (case-insensitive):
/// - `ssl_mode` / `ssl-mode` / `sslmode`: `disabled` | `preferred` | `required`
///   | `verify_ca` | `verify_identity` (aliases `verify-full`/`verify_full`).
/// - JDBC compatibility: `useSSL=true|false`, `requireSSL=true|false`,
///   `verifyServerCertificate=true|false`.
/// - `ssl_ca` / `sslrootcert` / `ssl-ca`: path to a root CA certificate
///   (.pem/.der) used for verify modes.
///
/// When no TLS key is present the result is `None`, preserving the historic
/// plaintext behaviour so local, non-TLS databases keep working with zero
/// config. The `mysql` crate can't do libpq-style "try TLS then fall back", so
/// any non-disabled mode requires a successful TLS handshake. Verification uses
/// the platform trust store (native-tls) unless a root cert path is supplied.
#[cfg(feature = "mysql_db")]
fn mysql_extract_ssl(url: &str) -> (String, Option<mysql::SslOpts>) {
    let (base, query) = match url.split_once('?') {
        Some((b, q)) => (b, q),
        None => return (url.to_string(), None),
    };

    let mut mode: Option<String> = None; // explicit ssl_mode wins
    let mut use_ssl: Option<bool> = None;
    let mut require_ssl: Option<bool> = None;
    let mut verify_cert: Option<bool> = None;
    let mut root_cert: Option<String> = None;
    let mut kept: Vec<&str> = Vec::new();

    for pair in query.split('&') {
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => {
                kept.push(pair);
                continue;
            }
        };
        match k.to_lowercase().as_str() {
            "ssl_mode" | "ssl-mode" | "sslmode" => mode = Some(v.trim().to_lowercase()),
            "usessl" => use_ssl = Some(v.eq_ignore_ascii_case("true") || v == "1"),
            "requiressl" => require_ssl = Some(v.eq_ignore_ascii_case("true") || v == "1"),
            "verifyservercertificate" => {
                verify_cert = Some(v.eq_ignore_ascii_case("true") || v == "1")
            }
            "ssl_ca" | "sslrootcert" | "ssl-ca" => root_cert = Some(v.to_string()),
            _ => kept.push(pair),
        }
    }

    // Resolve an effective mode. Explicit ssl_mode takes precedence; otherwise
    // derive one from the JDBC-style booleans.
    let effective = match mode.as_deref() {
        Some(m) => Some(m.to_string()),
        None => {
            if use_ssl == Some(true) || require_ssl == Some(true) {
                Some(match verify_cert {
                    Some(true) => "verify_identity".to_string(),
                    _ => "required".to_string(),
                })
            } else if use_ssl == Some(false) || require_ssl == Some(false) {
                Some("disabled".to_string())
            } else {
                None
            }
        }
    };

    let sanitized = if kept.is_empty() {
        base.to_string()
    } else {
        format!("{}?{}", base, kept.join("&"))
    };

    let ssl = match effective.as_deref() {
        None | Some("disabled") | Some("disable") => None,
        Some(m) => {
            let (accept_invalid, skip_domain) = match m {
                // encrypt only, do not authenticate the server certificate
                "preferred" | "prefer" | "required" | "require" => (true, true),
                // verify the chain but not the hostname
                "verify_ca" | "verify-ca" => (false, true),
                // verify the chain and the hostname (verify_identity / full)
                _ => (false, false),
            };
            let mut opts = mysql::SslOpts::default()
                .with_danger_accept_invalid_certs(accept_invalid)
                .with_danger_skip_domain_validation(skip_domain);
            if let Some(path) = root_cert {
                opts = opts.with_root_cert_path(Some(std::path::PathBuf::from(path)));
            }
            Some(opts)
        }
    };

    (sanitized, ssl)
}

#[cfg(feature = "mysql_db")]
fn get_mysql_pool(url: &str) -> Result<mysql::Pool, CfmlError> {
    let mut manager = get_pool_manager().lock().unwrap();
    let key = format!("mysql:{}", url);
    if let Some(pool_any) = manager.get(&key) {
        if let Some(pool) = pool_any.downcast_ref::<mysql::Pool>() {
            return Ok(pool.clone());
        }
    }
    let (sanitized, ssl) = mysql_extract_ssl(url);
    let opts = mysql::Opts::from_url(&sanitized)
        .map_err(|e| CfmlError::database(format!("queryExecute: invalid MySQL connection string: {}", e)))?;
    // Connection-per-request model (matching Lucee/ACF, which pool by request):
    //   * `reset_connection = true` (the crate default) — a connection is reset
    //     (COM_RESET_CONNECTION) when it is actually returned to the pool, wiping
    //     session/user state so it can NOT leak to the next request that reuses it.
    //   * A request HOLDS one connection per datasource for its whole duration (see
    //     REQUEST_MYSQL_CONNS / checkout_request_mysql_conn); it is only returned to
    //     the pool — and thus only reset — at the request boundary
    //     (`release_request_db_conns`). So within a request, session/user state
    //     persists across statements (Masa's `core/setup/db/mysql.sql` sets
    //     `@OLD_SQL_MODE`/`SQL_MODE` in separate cfqueries and restores them later;
    //     `SET SQL_MODE=@OLD_SQL_MODE` needs @OLD_SQL_MODE to still be set), but a
    //     transient `SET foreign_key_checks=0` / `sql_mode` does NOT outlive the
    //     request (GitHub #275 — the reset-disabled pool leaked pool-wide and across
    //     requests). `pool_min = 1` avoids the wasteful eager 10-connection burst.
    let constraints =
        mysql::PoolConstraints::new(1, 100).unwrap_or(mysql::PoolConstraints::DEFAULT);
    let pool_opts = mysql::PoolOpts::default()
        .with_reset_connection(true)
        .with_constraints(constraints);
    let builder = mysql::OptsBuilder::from_opts(opts)
        .ssl_opts(ssl)
        .pool_opts(pool_opts)
        // Per-connection prepared-statement LRU. The crate default (10) is far
        // below what one framework request uses (a Preside admin page runs
        // dozens of distinct statements), so every queryExecute thrashed the
        // cache and paid a full COM_STMT_PREPARE round-trip per query — the
        // MySQL wire writes were ~13% of warm-request CPU on a live Preside
        // profile. 256 comfortably covers a request's working set; JDBC pools
        // on Lucee ship a per-connection statement cache the same way.
        .stmt_cache_size(256)
        // Socket-level bounds — see `db_net_secs` for why keepalive is the one
        // that matters and why read/write default to off. Without these, a
        // black-holed connection hangs a request until the OS TCP timeout.
        .tcp_connect_timeout(db_connect_timeout())
        .tcp_keepalive_time_ms(db_keepalive().map(|d| d.as_millis() as u32))
        .read_timeout(db_socket_timeout())
        .write_timeout(db_socket_timeout())
        // Report rows MATCHED (not rows CHANGED) as the affected-row count, matching
        // Lucee/ACF — MySQL Connector/J negotiates CLIENT_FOUND_ROWS by default
        // (`useAffectedRows=false`), so an UPDATE whose new values equal the current
        // row still reports 1 affected row. Without this flag the MySQL server
        // reports 0 for such a no-op UPDATE, which diverges from the reference
        // engines and breaks Preside's DB session storage: `_updateSessionRecord`
        // returns `recordCount > 0`, so a same-second/unchanged session write
        // reported 0 and Preside minted a brand-new session every time (spurious
        // INSERT + `psid` cookie churn). CLIENT_FOUND_ROWS restores parity.
        .additional_capabilities(mysql::consts::CapabilityFlags::CLIENT_FOUND_ROWS);
    let pool = mysql::Pool::new(builder)
        .map_err(|e| CfmlError::database(format!("queryExecute: MySQL pool creation error: {}", e)))?;
    manager.insert(key, Box::new(pool.clone()));
    Ok(pool)
}

#[cfg(feature = "mysql_db")]
thread_local! {
    /// Request-scoped MySQL connections, keyed by connection URL. A request reuses
    /// one connection per datasource so session/user state (SET sql_mode, user
    /// variables like @OLD_SQL_MODE, foreign_key_checks, autocommit) persists across
    /// the request's statements — the Lucee/ACF connection-per-request model. The
    /// held connection is only returned to the pool — and thus only reset
    /// (COM_RESET_CONNECTION, since the pool has reset_connection=true) — at the
    /// request boundary via `release_request_db_conns`, so transient session state
    /// does NOT leak into the next request (GitHub #275).
    static REQUEST_MYSQL_CONNS: std::cell::RefCell<HashMap<String, mysql::PooledConn>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Take the request's held MySQL connection for `url`, or check a fresh one out of
/// the pool. The caller MUST return it via `return_request_mysql_conn` so it is
/// held for the rest of the request (and reset at the request boundary).
#[cfg(feature = "mysql_db")]
fn checkout_request_mysql_conn(
    url: &str,
    pool: &mysql::Pool,
) -> Result<mysql::PooledConn, CfmlError> {
    if let Some(conn) = REQUEST_MYSQL_CONNS.with(|c| c.borrow_mut().remove(url)) {
        return Ok(conn);
    }
    pool.get_conn()
        .map_err(|e| CfmlError::database(format!("queryExecute: MySQL connection error: {}", e)))
}

/// Return a MySQL connection to the request-scoped cache (held until request end).
/// Kept out of the pool for the request's duration so its session state survives
/// across statements.
#[cfg(feature = "mysql_db")]
fn return_request_mysql_conn(url: &str, conn: mysql::PooledConn) {
    REQUEST_MYSQL_CONNS.with(|c| {
        c.borrow_mut().insert(url.to_string(), conn);
    });
}

/// Release every request-scoped DB connection back to its pool. The serve loop
/// calls this at request boundaries (both before and after running a request, so a
/// prior request that died mid-flight can't leak its connection to the next one on
/// the same worker thread). Dropping each held MySQL connection returns it to the
/// pool, which — with `reset_connection = true` — fires COM_RESET_CONNECTION and
/// wipes its session/user state. Cheap no-op when nothing is held.
pub fn release_request_db_conns() {
    #[cfg(feature = "mysql_db")]
    REQUEST_MYSQL_CONNS.with(|c| {
        c.borrow_mut().clear();
    });
}

#[cfg(feature = "postgres_db")]
struct PostgresConnectionManager {
    url: String,
}

/// Per-connection prepared-statement cache. tokio-postgres' `query(&str)` awaits
/// a fresh `prepare()` (a Parse round-trip) before binding, so every
/// parameterized query costs TWO round-trips. Caching the prepared `Statement`
/// per connection makes the steady state ONE round-trip (bind/execute against
/// the already-parsed statement) — matching pgjdbc/Lucee.
#[cfg(feature = "postgres_db")]
type PgStmtCache = HashMap<String, postgres::Statement>;

/// A pooled PostgreSQL connection plus its prepared-statement cache. The cache
/// is connection-scoped (server-side prepared statements belong to one session)
/// and lives as long as the connection sits in the r2d2 pool.
#[cfg(feature = "postgres_db")]
struct PgConn {
    client: postgres::Client,
    stmt_cache: PgStmtCache,
    broken: bool,
}

/// Prepare `sql` once per connection, then hand back the cached `Statement`.
#[cfg(feature = "postgres_db")]
fn pg_prepare_cached(
    client: &mut postgres::Client,
    cache: &mut PgStmtCache,
    sql: &str,
) -> Result<postgres::Statement, postgres::Error> {
    if let Some(stmt) = cache.get(sql) {
        return Ok(stmt.clone());
    }
    let stmt = client.prepare(sql)?;
    cache.insert(sql.to_string(), stmt.clone());
    Ok(stmt)
}

/// True when the error is PostgreSQL's "cached plan must not change result type"
/// — raised when DDL alters a table a cached statement references. The fix is to
/// evict the stale statement and re-prepare (same as pgjdbc).
#[cfg(feature = "postgres_db")]
fn is_stale_cached_plan(e: &postgres::Error) -> bool {
    e.as_db_error()
        .map_or(false, |db| db.message().contains("cached plan must not change result type"))
}

/// Error type for the Postgres connection manager. TLS setup can fail with
/// non-`postgres::Error` causes (cert-store load, rustls config), so the
/// manager surfaces a single concrete error rather than `postgres::Error`.
#[cfg(feature = "postgres_db")]
#[derive(Debug)]
struct PgConnError(String);

#[cfg(feature = "postgres_db")]
impl std::fmt::Display for PgConnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(feature = "postgres_db")]
impl std::error::Error for PgConnError {}

#[cfg(feature = "postgres_db")]
impl From<postgres::Error> for PgConnError {
    fn from(e: postgres::Error) -> Self {
        PgConnError(e.to_string())
    }
}

#[cfg(feature = "postgres_db")]
impl r2d2::ManageConnection for PostgresConnectionManager {
    type Connection = PgConn;
    type Error = PgConnError;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        let client = connect_postgres(&self.url)?;
        Ok(PgConn { client, stmt_cache: HashMap::new(), broken: false })
    }

    fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
        conn.client.simple_query("SELECT 1").map(|_| ()).map_err(Into::into)
    }

    fn has_broken(&self, conn: &mut Self::Connection) -> bool {
        // The pool does NOT validate on checkout (Lucee parity — see
        // get_postgres_pool), so detect dead connections cheaply here instead:
        // r2d2 calls has_broken when a connection is returned and drops it if
        // true. `is_closed()` flips once the background connection task has
        // terminated (e.g. the server closed an idle socket), so a closed
        // connection is evicted rather than handed back out.
        conn.broken || conn.client.is_closed()
    }
}

/// rustls verifier that performs the TLS handshake (so the channel is
/// encrypted and `tls-server-end-point` channel binding still works) but does
/// NOT validate the certificate chain or hostname. This mirrors libpq /
/// pgjdbc behaviour for `sslmode=require` (and `prefer`/`allow`): the
/// connection is encrypted but the server certificate is not authenticated.
/// Signature checks are still delegated to the crypto provider so the cert
/// presented for channel binding is genuinely the peer's.
#[cfg(feature = "postgres_db")]
#[derive(Debug)]
struct PgNoCertVerify(std::sync::Arc<rustls::crypto::CryptoProvider>);

#[cfg(feature = "postgres_db")]
impl rustls::client::danger::ServerCertVerifier for PgNoCertVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Extract the effective `sslmode` from a Postgres connection URL, lower-cased.
/// Defaults to `prefer` (libpq's default) when absent — keeps local, non-TLS
/// databases working without configuration while still attempting TLS first.
#[cfg(feature = "postgres_db")]
fn pg_sslmode_from_url(url: &str) -> String {
    let query = match url.split_once('?') {
        Some((_, q)) => q,
        None => return "prefer".to_string(),
    };
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k.eq_ignore_ascii_case("sslmode") {
                return v.trim().to_lowercase();
            }
        }
    }
    "prefer".to_string()
}

/// tokio-postgres only understands `disable`/`prefer`/`require` for the
/// `sslmode` URL option and errors on libpq's `allow`/`verify-ca`/`verify-full`.
/// Rewrite the URL's `sslmode` to a token tokio-postgres accepts; the richer
/// verification semantics are applied via the rustls config we pass to
/// `connect`, not the URL. Other options (incl. `channel_binding`) are
/// preserved verbatim.
#[cfg(feature = "postgres_db")]
fn pg_sanitize_url_sslmode(url: &str) -> String {
    let (base, query) = match url.split_once('?') {
        Some((b, q)) => (b, q),
        None => return url.to_string(),
    };
    let rewritten: Vec<String> = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) if k.eq_ignore_ascii_case("sslmode") => {
                let token = match v.trim().to_lowercase().as_str() {
                    "disable" => "disable",
                    "allow" | "prefer" => "prefer",
                    // require / verify-ca / verify-full and anything else all
                    // require an encrypted channel; verification differences
                    // are handled by the rustls config.
                    _ => "require",
                };
                format!("{}={}", k, token)
            }
            _ => pair.to_string(),
        })
        .collect();
    format!("{}?{}", base, rewritten.join("&"))
}

/// Build a rustls `ClientConfig` for a Postgres TLS connection.
///
/// `verify == true` (sslmode `verify-ca`/`verify-full`) loads the platform's
/// native root certificate store and performs full chain + hostname
/// verification. `verify == false` (sslmode `require`/`prefer`/`allow`)
/// encrypts the channel without authenticating the certificate, matching libpq.
#[cfg(feature = "postgres_db")]
fn build_pg_tls_config(verify: bool) -> Result<rustls::ClientConfig, PgConnError> {
    let provider = std::sync::Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| PgConnError(format!("PostgreSQL TLS init failed: {e}")))?;

    let config = if verify {
        let mut roots = rustls::RootCertStore::empty();
        let loaded = rustls_native_certs::load_native_certs();
        for cert in loaded.certs {
            let _ = roots.add(cert);
        }
        if roots.is_empty() {
            return Err(PgConnError(format!(
                "PostgreSQL TLS: no native root certificates available for sslmode verify-ca/verify-full{}",
                loaded
                    .errors
                    .first()
                    .map(|e| format!(": {e}"))
                    .unwrap_or_default()
            )));
        }
        builder.with_root_certificates(roots).with_no_client_auth()
    } else {
        builder
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(PgNoCertVerify(provider)))
            .with_no_client_auth()
    };
    Ok(config)
}

/// Open a single PostgreSQL connection, honouring the `sslmode` URL option and
/// negotiating TLS via rustls where required. Used by the r2d2 manager.
#[cfg(feature = "postgres_db")]
fn connect_postgres(url: &str) -> Result<postgres::Client, PgConnError> {
    use std::str::FromStr;

    let mode = pg_sslmode_from_url(url);
    let sanitized = pg_sanitize_url_sslmode(url);
    let mut config = postgres::Config::from_str(&sanitized)
        .map_err(|e| PgConnError(format!("invalid PostgreSQL connection string: {e}")))?;

    // Socket-level bounds (GitHub #302) — see `db_net_secs`. Only applied when
    // the connection string didn't already specify them, so an explicit
    // `?connect_timeout=` / `?keepalives_idle=` in the DSN still wins.
    if config.get_connect_timeout().is_none() {
        if let Some(d) = db_connect_timeout() {
            config.connect_timeout(d);
        }
    }
    if let Some(d) = db_keepalive() {
        config.keepalives(true);
        config.keepalives_idle(d);
    }

    if mode == "disable" {
        config.ssl_mode(tokio_postgres::config::SslMode::Disable);
        return config.connect(postgres::NoTls).map_err(PgConnError::from);
    }

    config.ssl_mode(match mode.as_str() {
        "allow" | "prefer" => tokio_postgres::config::SslMode::Prefer,
        _ => tokio_postgres::config::SslMode::Require,
    });

    let verify = mode == "verify-ca" || mode == "verify-full";
    let tls_config = build_pg_tls_config(verify)?;
    let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls_config);
    config.connect(tls).map_err(PgConnError::from)
}

/// Maximum pooled PostgreSQL connections. Also bounds `execute_postgres`'s
/// stale-connection retry loop: draining at most this many dead connections
/// guarantees a fresh one.
#[cfg(feature = "postgres_db")]
const PG_POOL_MAX_SIZE: u32 = 10;

#[cfg(feature = "postgres_db")]
fn get_postgres_pool(url: &str) -> Result<r2d2::Pool<PostgresConnectionManager>, CfmlError> {
    let mut manager = get_pool_manager().lock().unwrap();
    let key = format!("postgres:{}", url);
    if let Some(pool_any) = manager.get(&key) {
        if let Some(pool) = pool_any.downcast_ref::<r2d2::Pool<PostgresConnectionManager>>() {
            return Ok(pool.clone());
        }
    }
    let mgr = PostgresConnectionManager { url: url.to_string() };
    let pool = r2d2::Pool::builder()
        .max_size(PG_POOL_MAX_SIZE)
        .min_idle(Some(1))
        .connection_timeout(datasource_timeout(url))
        // Do NOT ping `SELECT 1` on every checkout (Lucee parity / remote-DB
        // perf): Lucee's pool defaults `validate` off, so a cfquery is ONE
        // round-trip, not two. We rely on PostgresConnectionManager::has_broken
        // (is_closed) to evict dead connections on return instead. See PR #125.
        .test_on_check_out(false)
        .build(mgr)
        .map_err(|e| CfmlError::database(format!("queryExecute: failed to create PostgreSQL pool: {}", e)))?;
    manager.insert(key, Box::new(pool.clone()));
    Ok(pool)
}

// -----------------------------------------------
// MSSQL connection pool (tiberius)
// -----------------------------------------------

/// Maximum pooled MSSQL connections. Also bounds `execute_mssql`'s stale-connection
/// retry loop: draining at most this many dead connections guarantees a fresh one.
#[cfg(feature = "mssql_db")]
const MSSQL_POOL_MAX_SIZE: u32 = 10;

/// Concrete tiberius client type over a tokio TcpStream (the `compat` adapter
/// bridges tokio's AsyncRead/Write to the futures traits tiberius expects).
#[cfg(feature = "mssql_db")]
type MssqlClient = tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>;

/// tiberius is async and its TCP stream is bound to a runtime's reactor, so a
/// pooled connection only stays usable while that runtime is alive. We keep ONE
/// long-lived multi-threaded runtime for the whole process: connections are
/// established and queried on it, and concurrent `block_on` from multiple
/// blocking VM threads (serve mode) is safe on a multi-thread scheduler. Before
/// this, `execute_mssql` opened a fresh TCP + TLS + login handshake on EVERY
/// query (no pool at all) — many round-trips per `cfquery`. Pooling brings it in
/// line with the MySQL/PostgreSQL paths.
#[cfg(feature = "mssql_db")]
static MSSQL_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

#[cfg(feature = "mssql_db")]
fn mssql_runtime() -> &'static tokio::runtime::Runtime {
    MSSQL_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to build MSSQL tokio runtime")
    })
}

/// A pooled MSSQL connection. `broken` is set when a query fails with a
/// connection-level (not server/SQL) error so r2d2 evicts it on return — the
/// pool does NOT ping on checkout (see `get_mssql_pool`), mirroring the
/// PostgreSQL `has_broken` strategy.
#[cfg(feature = "mssql_db")]
struct MssqlConn {
    client: MssqlClient,
    broken: bool,
}

/// True when a tiberius error means the connection itself is unusable (I/O,
/// protocol, TLS, encoding, or a server-requested redirect) rather than a
/// per-statement failure (`Server` = a SQL error, `Conversion`, etc.). Only the
/// former should evict the pooled connection.
#[cfg(feature = "mssql_db")]
fn is_mssql_connection_error(e: &tiberius::error::Error) -> bool {
    use tiberius::error::Error;
    matches!(
        e,
        Error::Io { .. } | Error::Protocol(_) | Error::Encoding(_) | Error::Tls(_) | Error::Routing { .. }
    )
}

/// Error wrapper for the MSSQL connection manager (TLS/login failures surface as
/// non-`postgres`-style strings; r2d2 just needs `Error + 'static`).
#[cfg(feature = "mssql_db")]
#[derive(Debug)]
struct MssqlConnError(String);

#[cfg(feature = "mssql_db")]
impl std::fmt::Display for MssqlConnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(feature = "mssql_db")]
impl std::error::Error for MssqlConnError {}

#[cfg(feature = "mssql_db")]
struct MssqlConnectionManager {
    config: tiberius::Config,
    addr: String,
}

#[cfg(feature = "mssql_db")]
impl r2d2::ManageConnection for MssqlConnectionManager {
    type Connection = MssqlConn;
    type Error = MssqlConnError;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        use tokio_util::compat::TokioAsyncWriteCompatExt;
        let config = self.config.clone();
        let addr = self.addr.clone();
        let client = mssql_runtime().block_on(async move {
            let tcp = tokio::net::TcpStream::connect(&addr).await
                .map_err(|e| MssqlConnError(format!("MSSQL connection error: {}", e)))?;
            tcp.set_nodelay(true).ok();
            tiberius::Client::connect(config, tcp.compat_write()).await
                .map_err(|e| MssqlConnError(format!("MSSQL connection error: {}", e)))
        })?;
        Ok(MssqlConn { client, broken: false })
    }

    fn is_valid(&self, _conn: &mut Self::Connection) -> Result<(), Self::Error> {
        // Pool does not validate on checkout (test_on_check_out(false)).
        Ok(())
    }

    fn has_broken(&self, conn: &mut Self::Connection) -> bool {
        conn.broken
    }
}

/// Parse an `mssql://`/`sqlserver://` URL into a tiberius `Config` plus the
/// `host:port` to dial. Extracted from the old inline `execute_mssql` body so
/// the pool manager can re-establish connections without re-parsing per query.
#[cfg(feature = "mssql_db")]
fn mssql_config_from_url(url: &str) -> Result<(tiberius::Config, String), CfmlError> {
    use tiberius::{Config, AuthMethod};

    let clean_url = url.replace("mssql://", "").replace("sqlserver://", "");
    // Split off an optional query string before parsing the path components, so
    // the database name isn't polluted by `?trustServerCertificate=...` etc.
    let (clean_url, query) = match clean_url.split_once('?') {
        Some((b, q)) => (b.to_string(), Some(q.to_string())),
        None => (clean_url, None),
    };
    // Format: user:pass@host:port/database
    let (auth_part, host_db) = clean_url.split_once('@')
        .ok_or_else(|| CfmlError::database("queryExecute: MSSQL URL must be mssql://user:pass@host:port/database".to_string()))?;
    let (user, pass) = auth_part.split_once(':')
        .ok_or_else(|| CfmlError::database("queryExecute: MSSQL URL must include user:password".to_string()))?;
    let (host_port, database) = host_db.split_once('/')
        .unwrap_or((host_db, "master"));
    let (host, port_str) = host_port.split_once(':')
        .unwrap_or((host_port, "1433"));
    let port: u16 = port_str.parse().unwrap_or(1433);

    // TLS: tiberius (rustls feature) negotiates encryption by default. By
    // default we trust the server certificate (works with Azure SQL and any
    // managed instance out of the box). Set `trustServerCertificate=false`
    // (or `encrypt=strict`) in the URL to instead validate the certificate
    // against the platform trust store.
    let mut trust_cert = true;
    if let Some(q) = &query {
        for pair in q.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                match k.to_lowercase().as_str() {
                    "trustservercertificate" => {
                        trust_cert = !(v.eq_ignore_ascii_case("false") || v == "0");
                    }
                    "encrypt" if v.eq_ignore_ascii_case("strict") => trust_cert = false,
                    _ => {}
                }
            }
        }
    }

    let mut config = Config::new();
    config.host(host);
    config.port(port);
    config.database(database);
    config.authentication(AuthMethod::sql_server(user, pass));
    if trust_cert {
        config.trust_cert();
    }

    Ok((config, format!("{}:{}", host, port)))
}

#[cfg(feature = "mssql_db")]
fn get_mssql_pool(url: &str) -> Result<r2d2::Pool<MssqlConnectionManager>, CfmlError> {
    let mut manager = get_pool_manager().lock().unwrap();
    let key = format!("mssql:{}", url);
    if let Some(pool_any) = manager.get(&key) {
        if let Some(pool) = pool_any.downcast_ref::<r2d2::Pool<MssqlConnectionManager>>() {
            return Ok(pool.clone());
        }
    }
    let (config, addr) = mssql_config_from_url(url)?;
    let mgr = MssqlConnectionManager { config, addr };
    let pool = r2d2::Pool::builder()
        .max_size(MSSQL_POOL_MAX_SIZE)
        .min_idle(Some(1))
        .connection_timeout(datasource_timeout(url))
        // No per-checkout ping (Lucee parity / remote-DB perf): rely on
        // MssqlConnectionManager::has_broken to evict dead connections instead.
        .test_on_check_out(false)
        .build(mgr)
        .map_err(|e| CfmlError::database(format!("queryExecute: failed to create MSSQL pool: {}", e)))?;
    manager.insert(key, Box::new(pool.clone()));
    Ok(pool)
}

// -----------------------------------------------
// Structured query parameter normalization
// -----------------------------------------------

/// Unwrap a cfqueryparam-style struct (`{value, cfsqltype, null, ...}`) to the
/// effective bind value: `CfmlValue::Null` when `null=true`, otherwise the
/// inner `value`. Non-struct inputs pass through unchanged.
///
/// Lucee/ACF/BoxLang accept this struct wherever a queryExecute parameter is
/// expected — positional **and** named — and bind the inner value (or NULL).
/// Binding the stringified struct itself is never correct. The positional
/// (array) path already strips the struct in `normalize_query_params`; this
/// helper covers the named (struct-of-params) path the per-driver builders
/// hit later.
///
/// Intentionally NOT feature-gated: `pg_sql::prepare_pg_statements` is
/// always-compiled (no DB feature gate), so the helper has to be too.
pub(crate) fn cfqueryparam_unwrap(v: &CfmlValue) -> CfmlValue {
    if let CfmlValue::Struct(s) = v {
        let has_value = s.iter().any(|(k, _)| k.eq_ignore_ascii_case("value"));
        if !has_value {
            return v.clone();
        }
        let is_null = s
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("null"))
            .map(|(_, nv)| match nv {
                CfmlValue::Bool(b) => b,
                CfmlValue::String(s) => {
                    s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("yes")
                }
                _ => false,
            })
            .unwrap_or(false);
        if is_null {
            return CfmlValue::Null;
        }
        return s
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("value"))
            .map(|(_, val)| val.clone())
            .unwrap_or(CfmlValue::Null);
    }
    v.clone()
}

#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn normalize_query_params(params_arg: &CfmlValue) -> (Vec<CfmlValue>, Vec<String>) {
    // If params is an array of structs with "value" key, extract typed values
    // Returns (effective_values, type_hints)
    match params_arg {
        CfmlValue::Array(arr) if !arr.is_empty() => {
            // Check if first element is a struct with "value" key (cfqueryparam style)
            if let Some(CfmlValue::Struct(first)) = arr.first() {
                let has_value_key = first.iter().any(|(k, _)| k.eq_ignore_ascii_case("value"));
                if has_value_key {
                    let mut values = Vec::with_capacity(arr.len());
                    let mut type_hints = Vec::with_capacity(arr.len());
                    for item in arr.iter() {
                        if let CfmlValue::Struct(s) = item {
                            let value = s.iter()
                                .find(|(k, _)| k.eq_ignore_ascii_case("value"))
                                .map(|(_, v)| v.clone())
                                .unwrap_or(CfmlValue::Null);

                            let is_null = s.iter()
                                .find(|(k, _)| k.eq_ignore_ascii_case("null"))
                                .map(|(_, v)| {
                                    match v {
                                        CfmlValue::Bool(b) => b,
                                        CfmlValue::String(s) => s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("yes"),
                                        _ => false,
                                    }
                                })
                                .unwrap_or(false);

                            let cfsqltype = s.iter()
                                .find(|(k, _)| k.eq_ignore_ascii_case("cfsqltype"))
                                .map(|(_, v)| v.as_string().to_lowercase())
                                .unwrap_or_else(|| "cf_sql_varchar".to_string());

                            let is_list = s.iter()
                                .find(|(k, _)| k.eq_ignore_ascii_case("list"))
                                .map(|(_, v)| {
                                    match v {
                                        CfmlValue::Bool(b) => b,
                                        CfmlValue::String(s) => s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("yes"),
                                        _ => false,
                                    }
                                })
                                .unwrap_or(false);

                            let separator = s.iter()
                                .find(|(k, _)| k.eq_ignore_ascii_case("separator"))
                                .map(|(_, v)| v.as_string())
                                .unwrap_or_else(|| ",".to_string());

                            if is_null {
                                values.push(CfmlValue::Null);
                                type_hints.push(cfsqltype);
                            } else if is_list {
                                // Expand list value into multiple params
                                let val_str = value.as_string();
                                for part in val_str.split(&*separator) {
                                    let trimmed = part.trim();
                                    values.push(coerce_by_sqltype(trimmed, &cfsqltype));
                                    type_hints.push(cfsqltype.clone());
                                }
                            } else {
                                let coerced = coerce_by_sqltype_value(&value, &cfsqltype);
                                values.push(coerced);
                                type_hints.push(cfsqltype);
                            }
                        } else {
                            values.push(item.clone());
                            type_hints.push("cf_sql_varchar".to_string());
                        }
                    }
                    return (values, type_hints);
                }
            }
            // Plain array — pass through
            (arr.to_vec(), vec!["cf_sql_varchar".to_string(); arr.len()])
        }
        _ => (vec![], vec![]),
    }
}

/// Get how many placeholder values each structured param generates (1 for normal, N for list)
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn get_list_placeholder_counts(params_arg: &CfmlValue) -> Vec<usize> {
    match params_arg {
        CfmlValue::Array(arr) => {
            arr.iter().map(|item| {
                if let CfmlValue::Struct(s) = item {
                    let is_list = s.iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("list"))
                        .map(|(_, v)| {
                            match v {
                                CfmlValue::Bool(b) => b,
                                CfmlValue::String(s) => s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("yes"),
                                _ => false,
                            }
                        })
                        .unwrap_or(false);
                    if is_list {
                        let separator = s.iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("separator"))
                            .map(|(_, v)| v.as_string())
                            .unwrap_or_else(|| ",".to_string());
                        let value = s.iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("value"))
                            .map(|(_, v)| v.as_string())
                            .unwrap_or_default();
                        value.split(&*separator).filter(|s| !s.trim().is_empty()).count().max(1)
                    } else {
                        1
                    }
                } else {
                    1
                }
            }).collect()
        }
        _ => vec![],
    }
}

/// Expand SQL ? placeholders for list params: if param N generates 3 values, replace its ? with ?,?,?
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn expand_sql_placeholders(sql: &str, counts: &[usize]) -> String {
    let mut result = String::with_capacity(sql.len() + counts.len() * 4);
    let mut param_idx = 0;
    let bytes = sql.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] == b'?' && param_idx < counts.len() {
            let count = counts[param_idx];
            for j in 0..count {
                if j > 0 { result.push(','); }
                result.push('?');
            }
            param_idx += 1;
        } else if bytes[i] == b'\'' {
            result.push('\'');
            i += 1;
            while i < len && bytes[i] != b'\'' {
                result.push(bytes[i] as char);
                i += 1;
            }
            if i < len { result.push('\''); }
        } else {
            result.push(bytes[i] as char);
        }
        i += 1;
    }
    result
}

#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn coerce_by_sqltype(val_str: &str, sqltype: &str) -> CfmlValue {
    match sqltype {
        s if s.contains("integer") || s.contains("bigint") || s.contains("smallint") || s.contains("tinyint") => {
            val_str.parse::<i64>().map(CfmlValue::Int).unwrap_or(CfmlValue::string(val_str.to_string()))
        }
        s if s.contains("float") || s.contains("double") || s.contains("decimal") || s.contains("numeric") || s.contains("real") || s.contains("money") => {
            val_str.parse::<f64>().map(CfmlValue::Double).unwrap_or(CfmlValue::string(val_str.to_string()))
        }
        s if s.contains("bit") || s.contains("boolean") => {
            let lower = val_str.to_lowercase();
            CfmlValue::Bool(lower == "true" || lower == "yes" || lower == "1")
        }
        _ => CfmlValue::string(val_str.to_string()),
    }
}

#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn coerce_by_sqltype_value(val: &CfmlValue, sqltype: &str) -> CfmlValue {
    match sqltype {
        s if s.contains("integer") || s.contains("bigint") || s.contains("smallint") || s.contains("tinyint") => {
            match val {
                CfmlValue::Int(_) => val.clone(),
                CfmlValue::Double(d) => CfmlValue::Int(*d as i64),
                CfmlValue::String(s) => s.parse::<i64>().map(CfmlValue::Int).unwrap_or(val.clone()),
                CfmlValue::Bool(b) => CfmlValue::Int(if *b { 1 } else { 0 }),
                _ => val.clone(),
            }
        }
        s if s.contains("float") || s.contains("double") || s.contains("decimal") || s.contains("numeric") || s.contains("real") || s.contains("money") => {
            match val {
                CfmlValue::Double(_) => val.clone(),
                CfmlValue::Int(i) => CfmlValue::Double(*i as f64),
                CfmlValue::String(s) => s.parse::<f64>().map(CfmlValue::Double).unwrap_or(val.clone()),
                _ => val.clone(),
            }
        }
        s if s.contains("bit") || s.contains("boolean") => {
            match val {
                CfmlValue::Bool(_) => val.clone(),
                CfmlValue::Int(i) => CfmlValue::Bool(*i != 0),
                CfmlValue::String(s) => {
                    let lower = s.to_lowercase();
                    CfmlValue::Bool(lower == "true" || lower == "yes" || lower == "1")
                }
                _ => val.clone(),
            }
        }
        s if s == "cf_sql_null" => CfmlValue::Null,
        _ => val.clone(),
    }
}

/// Strip leading whitespace, SQL comments (`-- line` and `/* block */`), and
/// leading open-parens so the first *significant* keyword can be classified.
/// Returns the remainder starting at the first non-comment, non-whitespace,
/// non-`(` byte.
///
/// Leading `(` is peeled because a parenthesised top-level statement is always
/// row-returning — `( SELECT ... ) UNION ALL ( SELECT ... )` (Preside's
/// `selectUnion` wraps every branch in parens) or `( SELECT ... )` / `( VALUES
/// ... )`. You cannot wrap an INSERT/UPDATE/DELETE in parens at the top level,
/// so peeling can't misclassify a mutation as a query. Without this, a leading
/// `(` left an empty first keyword, the statement was treated as a mutation, and
/// the UNION ran down the execute path returning 0 rows.
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn strip_leading_sql_noise(sql: &str) -> &str {
    let mut s = sql.trim_start();
    loop {
        if let Some(rest) = s.strip_prefix("--") {
            // line comment: skip to end of line
            s = match rest.find('\n') {
                Some(i) => rest[i + 1..].trim_start(),
                None => return "",
            };
        } else if let Some(rest) = s.strip_prefix("/*") {
            // block comment: skip to closing */
            s = match rest.find("*/") {
                Some(i) => rest[i + 2..].trim_start(),
                None => return "",
            };
        } else if let Some(rest) = s.strip_prefix('(') {
            // leading open-paren of a parenthesised row-returning statement
            s = rest.trim_start();
        } else {
            return s;
        }
    }
}

/// Decide whether a SQL statement returns rows (and so must be run via the
/// query path rather than the execute path). A statement is row-returning
/// when its first significant keyword is one of:
///   - `SELECT`            — ordinary query
///   - `WITH`              — CTE that resolves to a `SELECT` (Lucee parity)
///   - `CALL` / `EXEC` / `EXECUTE` — stored-procedure invocation that may
///                            return a result set (CALL: MySQL/standard,
///                            EXEC[UTE]: SQL Server)
///   - `VALUES`            — row constructor (Postgres/SQLite)
///   - `SHOW` / `PRAGMA` / `EXPLAIN` / `DESCRIBE` / `DESC` — metadata queries
/// Leading whitespace and SQL comments are stripped first.
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn is_select_query(sql: &str) -> bool {
    let trimmed = strip_leading_sql_noise(sql);
    // First keyword = leading run of ASCII-alphabetic bytes.
    let kw_len = trimmed
        .as_bytes()
        .iter()
        .take_while(|b| b.is_ascii_alphabetic())
        .count();
    let kw = &trimmed[..kw_len];
    matches!(
        kw.to_ascii_uppercase().as_str(),
        "SELECT" | "WITH" | "CALL" | "EXEC" | "EXECUTE" | "VALUES" | "SHOW" | "PRAGMA"
            | "EXPLAIN" | "DESCRIBE" | "DESC"
    )
}

/// PostgreSQL also returns rows from DML statements with a top-level
/// `RETURNING` clause. Those must run through `query`, not `execute`; the
/// postgres crate rejects row-returning statements on the execute path.
#[cfg(feature = "postgres_db")]
fn postgres_returns_rows(sql: &str) -> bool {
    let trimmed = strip_leading_sql_noise(sql);
    let kw_len = trimmed
        .as_bytes()
        .iter()
        .take_while(|b| b.is_ascii_alphabetic())
        .count();
    let kw = trimmed[..kw_len].to_ascii_uppercase();

    if matches!(
        kw.as_str(),
        "SELECT" | "WITH" | "CALL" | "EXEC" | "EXECUTE" | "VALUES" | "SHOW" | "PRAGMA"
            | "EXPLAIN" | "DESCRIBE" | "DESC"
    ) {
        return true;
    }

    matches!(kw.as_str(), "INSERT" | "UPDATE" | "DELETE" | "MERGE")
        && sql_has_top_level_keyword(trimmed, "RETURNING")
}

/// SQL Server returns rows from DML statements carrying a top-level `OUTPUT`
/// clause (`INSERT/UPDATE/DELETE/MERGE ... OUTPUT inserted.*`). Like Postgres
/// `RETURNING`, those must run through `query`, not `execute`: tiberius'
/// `execute()` silently discards the result set, so the OUTPUT rows would be
/// lost and the caller would see a bare mutation result instead. The `OUTPUT
/// ... INTO @tbl` form does NOT stream rows back to the client, so it must be
/// treated as a plain mutation (detected by an `INTO` following `OUTPUT`).
#[cfg(feature = "mssql_db")]
fn mssql_returns_rows(sql: &str) -> bool {
    let trimmed = strip_leading_sql_noise(sql);
    let kw_len = trimmed
        .as_bytes()
        .iter()
        .take_while(|b| b.is_ascii_alphabetic())
        .count();
    let kw = trimmed[..kw_len].to_ascii_uppercase();

    if matches!(
        kw.as_str(),
        "SELECT" | "WITH" | "CALL" | "EXEC" | "EXECUTE" | "VALUES" | "SHOW" | "PRAGMA"
            | "EXPLAIN" | "DESCRIBE" | "DESC"
    ) {
        return true;
    }

    matches!(kw.as_str(), "INSERT" | "UPDATE" | "DELETE" | "MERGE")
        && sql_has_top_level_keyword(trimmed, "OUTPUT")
        && !sql_has_top_level_keyword(trimmed, "INTO")
}

/// MariaDB (10.5+) returns rows from `INSERT ... RETURNING` and
/// `DELETE ... RETURNING`. Stock MySQL has no RETURNING (the server rejects it
/// as a syntax error regardless of routing). Where the server does support it,
/// `exec_drop` silently discards the rows, so those statements must run through
/// the row-returning `exec` path instead.
#[cfg(feature = "mysql_db")]
fn mysql_returns_rows(sql: &str) -> bool {
    let trimmed = strip_leading_sql_noise(sql);
    let kw_len = trimmed
        .as_bytes()
        .iter()
        .take_while(|b| b.is_ascii_alphabetic())
        .count();
    let kw = trimmed[..kw_len].to_ascii_uppercase();

    if matches!(
        kw.as_str(),
        "SELECT" | "WITH" | "CALL" | "EXEC" | "EXECUTE" | "VALUES" | "SHOW" | "PRAGMA"
            | "EXPLAIN" | "DESCRIBE" | "DESC"
    ) {
        return true;
    }

    matches!(kw.as_str(), "INSERT" | "DELETE")
        && sql_has_top_level_keyword(trimmed, "RETURNING")
}

#[cfg(any(feature = "postgres_db", feature = "mssql_db", feature = "mysql_db"))]
fn sql_has_top_level_keyword(sql: &str, target: &str) -> bool {
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0usize;
    let mut depth = 0i32;

    while i < chars.len() {
        let c = chars[i];

        if c == '\'' || c == '"' {
            i = skip_sql_quoted(&chars, i, c);
            continue;
        }

        if c == '-' && chars.get(i + 1) == Some(&'-') {
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(chars.len());
            continue;
        }

        if c == '$' {
            if let Some(next) = skip_postgres_dollar_quote(&chars, i) {
                i = next;
                continue;
            }
        }

        match c {
            '(' => depth += 1,
            ')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ => {
                if depth == 0 && (c.is_ascii_alphabetic() || c == '_') {
                    let start = i;
                    i += 1;
                    while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                        i += 1;
                    }
                    let word: String = chars[start..i].iter().collect();
                    if word.eq_ignore_ascii_case(target) {
                        return true;
                    }
                    continue;
                }
            }
        }

        i += 1;
    }

    false
}

#[cfg(any(feature = "postgres_db", feature = "mssql_db", feature = "mysql_db"))]
fn skip_sql_quoted(chars: &[char], start: usize, quote: char) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == quote {
            if chars.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

#[cfg(any(feature = "postgres_db", feature = "mssql_db", feature = "mysql_db"))]
fn skip_postgres_dollar_quote(chars: &[char], start: usize) -> Option<usize> {
    let mut tag_end = start + 1;
    while tag_end < chars.len() && (chars[tag_end].is_ascii_alphanumeric() || chars[tag_end] == '_') {
        tag_end += 1;
    }
    if chars.get(tag_end) != Some(&'$') {
        return None;
    }

    let delimiter: Vec<char> = chars[start..=tag_end].to_vec();
    let mut i = tag_end + 1;
    while i + delimiter.len() <= chars.len() {
        if chars[i..i + delimiter.len()] == delimiter[..] {
            return Some(i + delimiter.len());
        }
        i += 1;
    }
    Some(chars.len())
}

/// Dynamic-driver-only `queryExecute` for builds that don't enable any
/// of the per-engine DB features (e.g. the Cloudflare Workers host,
/// which uses the dynamic-driver registry to plug in D1). Looks up
/// `datasource` in [`crate::db_driver::lookup_dynamic_datasource`] and
/// hands off to the registered driver. Errors out if the requested
/// datasource isn't registered.
pub fn fn_query_execute_dynamic(args: Vec<CfmlValue>) -> CfmlResult {
    let sql = get_str(&args, 0);
    if sql.is_empty() {
        return Err(CfmlError::runtime(
            "queryExecute: SQL string is required".to_string(),
        ));
    }

    let raw_params = args.get(1).cloned().unwrap_or(CfmlValue::Null);
    let options_arg = args.get(2).cloned().unwrap_or(CfmlValue::Null);

    let datasource_attr: Option<String> = match &options_arg {
        CfmlValue::Struct(opts) => opts
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("datasource"))
            .map(|(_, v)| datasource_attr_string(&v)),
        _ => None,
    };
    let ds_name = datasource_attr.unwrap_or_default();
    let driver = crate::db_driver::lookup_dynamic_datasource(&ds_name).ok_or_else(|| {
        CfmlError::runtime(format!(
            "queryExecute: datasource '{}' is not registered with the dynamic-driver registry",
            ds_name
        ))
    })?;

    let return_type = match &options_arg {
        CfmlValue::Struct(opts) => opts
            .iter()
            .find(|(k, _)| {
                k.eq_ignore_ascii_case("returntype") || k.eq_ignore_ascii_case("returnType")
            })
            .map(|(_, v)| v.as_string().to_lowercase())
            .unwrap_or_else(|| "query".to_string()),
        _ => "query".to_string(),
    };

    driver.execute(&sql, &raw_params, &return_type)
}

/// Normalize a `cfqueryparam`-style positional param array (an array of
/// `{value, cfsqltype, …}` structs) to plain bind values, and expand any
/// `list` params in the SQL (one `?` → `?,?,?`). Returns the (possibly
/// rewritten) SQL plus the normalized params. Non-struct arrays and named
/// (struct) params pass through unchanged.
///
/// Both the normal `queryExecute` path and the transaction path must run
/// this before handing params to a driver — otherwise the raw cfqueryparam
/// struct reaches the driver and gets stringified into the SQL (issue #147).
/// Idempotent: an already-normalized plain array has no `value`-keyed structs,
/// so it returns unchanged.
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn normalize_positional_params(sql: String, raw_params: &CfmlValue) -> (String, CfmlValue) {
    if let CfmlValue::Array(arr) = raw_params {
        if let Some(CfmlValue::Struct(first)) = arr.first() {
            if first.iter().any(|(k, _)| k.eq_ignore_ascii_case("value")) {
                let (values, _hints) = normalize_query_params(raw_params);
                // Check if any list params require SQL expansion
                let placeholder_counts = get_list_placeholder_counts(raw_params);
                let expanded_sql = if placeholder_counts.iter().any(|&c| c > 1) {
                    expand_sql_placeholders(&sql, &placeholder_counts)
                } else {
                    sql
                };
                return (expanded_sql, CfmlValue::array(values));
            }
        }
    }
    (sql, raw_params.clone())
}

#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
pub fn fn_query_execute(args: Vec<CfmlValue>) -> CfmlResult {
    let sql = get_str(&args, 0);
    if sql.is_empty() {
        return Err(CfmlError::database("queryExecute: SQL string is required".to_string()));
    }

    let raw_params = args.get(1).cloned().unwrap_or(CfmlValue::Null);
    let options_arg = args.get(2).cloned().unwrap_or(CfmlValue::Null);

    // Normalize structured params (cfqueryparam-style array of structs) to plain values
    // Also expand SQL for list params (single ? → multiple ?,?,?)
    let (sql, params_arg) = normalize_positional_params(sql, &raw_params);

    // Extract datasource from options. Look up against the cfconfig
    // registry first; fall back to the default datasource if the call
    // omitted one entirely; final fallback is the historical `:memory:`
    // sqlite default so existing tests keep working.
    let datasource_attr: Option<String> = match &options_arg {
        CfmlValue::Struct(opts) => opts
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("datasource"))
            .map(|(_, v)| datasource_attr_string(&v)),
        _ => None,
    };

    // Dynamic-driver fast path: if the literal datasource name is registered
    // via cfml_stdlib::db_driver, hand off without touching the URL/enum
    // pipeline. This is how cfml-worker plugs in Cloudflare D1.
    let dynamic_driver = datasource_attr
        .as_deref()
        .and_then(crate::db_driver::lookup_dynamic_datasource);

    let datasource = match &datasource_attr {
        Some(name) => resolve_query_datasource(name)?,
        None => default_datasource().unwrap_or_else(|| ":memory:".to_string()),
    };

    // Extract returnType from options. For returntype="struct", also pull
    // columnkey and bake it into the wire string as "struct:<key>" so the
    // four per-driver execute_* paths don't need new arguments.
    let return_type = match &options_arg {
        CfmlValue::Struct(opts) => {
            let rt = opts.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("returntype") || k.eq_ignore_ascii_case("returnType"))
                .map(|(_, v)| v.as_string().to_lowercase())
                .unwrap_or_else(|| "query".to_string());
            if rt == "struct" {
                // Lucee accepts both `columnKey` and `keyColumn` (Wheels' Lucee
                // engine adapter passes `keyColumn`).
                let key = opts.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("columnkey") || k.eq_ignore_ascii_case("keycolumn"))
                    .map(|(_, v)| v.as_string())
                    .unwrap_or_default();
                if key.is_empty() { rt } else { format!("struct:{}", key) }
            } else {
                rt
            }
        }
        _ => "query".to_string(),
    };

    // `timeout` (seconds) aborts a query that overruns, mirroring JDBC
    // Statement.setQueryTimeout (what Lucee's `timeout` option maps to). 0 / <=0
    // means "no timeout". Currently enforced for the MySQL/MariaDB driver via a
    // server-side KILL QUERY watchdog (see execute_mysql); other drivers accept
    // the option but do not yet enforce it (docs/known-issues.md).
    let query_timeout: Option<u32> = match &options_arg {
        CfmlValue::Struct(opts) => opts
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("timeout"))
            .and_then(|(_, v)| {
                let n = v.as_string().trim().parse::<i64>().ok()?;
                if n > 0 { Some(n as u32) } else { None }
            }),
        _ => None,
    };

    // `maxrows` caps the returned resultset (Lucee/ACF). A negative value is
    // Lucee's "no limit" sentinel. Applied post-execution so it works uniformly
    // across every driver (GitHub #251 — was QoQ-only before).
    let max_rows: Option<usize> = match &options_arg {
        CfmlValue::Struct(opts) => opts
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("maxrows"))
            .and_then(|(_, v)| {
                let n = v.as_string().trim().parse::<i64>().ok()?;
                if n >= 0 { Some(n as usize) } else { None }
            }),
        _ => None,
    };

    if let Some(driver) = dynamic_driver {
        let r = driver.execute(&sql, &params_arg, &return_type)?;
        return Ok(apply_query_maxrows(r, max_rows));
    }

    let result = match parse_datasource(&datasource) {
        #[cfg(feature = "sqlite")]
        DbDriver::Sqlite(path) => execute_sqlite(&path, &sql, &params_arg, &return_type),
        #[cfg(feature = "mysql_db")]
        DbDriver::Mysql(url) => execute_mysql(&url, &sql, &params_arg, &return_type, query_timeout),
        #[cfg(feature = "postgres_db")]
        DbDriver::Postgres(url) => execute_postgres(&url, &sql, &params_arg, &return_type),
        #[cfg(feature = "mssql_db")]
        DbDriver::Mssql(url) => execute_mssql(&url, &sql, &params_arg, &return_type),
        #[allow(unreachable_patterns)]
        _ => Err(CfmlError::runtime(format!(
            "queryExecute: database driver not available for datasource '{}'. Enable the appropriate feature (sqlite, mysql_db, postgres_db, mssql_db).",
            datasource
        ))),
    }?;
    Ok(apply_query_maxrows(result, max_rows))
}

/// Cap a query result to `max_rows` (Lucee `maxrows`), applied after execution
/// so it is driver-independent. Handles the query and array return shapes; the
/// struct (keyColumn) shape is left as-is (row order there is not meaningful).
fn apply_query_maxrows(result: CfmlValue, max_rows: Option<usize>) -> CfmlValue {
    let Some(n) = max_rows else { return result; };
    match result {
        CfmlValue::Query(q) => {
            q.with_write(|data| {
                if data.row_count() > n {
                    for col in data.data.iter_mut() {
                        std::sync::Arc::make_mut(col).truncate(n);
                    }
                }
            });
            CfmlValue::Query(q)
        }
        CfmlValue::Array(a) => {
            a.with_write(|v| v.truncate(n));
            CfmlValue::Array(a)
        }
        other => other,
    }
}

// -----------------------------------------------
// SQLite driver
// -----------------------------------------------

#[cfg(feature = "sqlite")]
fn execute_sqlite(path: &str, sql: &str, params_arg: &CfmlValue, return_type: &str) -> CfmlResult {
    use rusqlite::types::Value as SqlValue;

    let pool = get_sqlite_pool(path)?;
    let conn = pool.get()
        .map_err(|e| CfmlError::database(format!("queryExecute: failed to get SQLite connection from pool: {}", e)))?;

    // See `execute_sqlite_with_conn`: emulate MySQL `@@` system variables that
    // SQLite cannot parse, so capability-detection selects don't crash.
    let sql_owned = rewrite_mysql_system_vars(sql);
    let sql = sql_owned.as_str();
    let (exec_sql, bound_params) = build_sqlite_params(params_arg, sql)?;

    if is_select_query(sql) {
        let mut stmt = conn.prepare(&exec_sql)
            .map_err(|e| CfmlError::database(format!("queryExecute: SQL error: {}", e)))?;

        let column_count = stmt.column_count();
        let raw_columns: Vec<String> = (0..column_count)
            .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
            .collect();
        let (columns, keep) = dedup_result_columns(raw_columns);

        let rows_result: Result<Vec<ValueMap>, _> = stmt
            .query_map(rusqlite::params_from_iter(bound_params.iter()), |row| {
                let mut row_map = ValueMap::default();
                for (out_i, &src_i) in keep.iter().enumerate() {
                    let val: SqlValue = row.get_unwrap(src_i);
                    row_map.insert(columns[out_i].clone(), sqlite_to_cfml(val));
                }
                Ok(row_map)
            })
            .map_err(|e| CfmlError::database(format!("queryExecute: query error: {}", e)))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CfmlError::database(format!("queryExecute: row error: {}", e)));

        let rows = rows_result?;
        build_query_result(columns, rows, sql, return_type)
    } else {
        let affected = conn.execute(&exec_sql, rusqlite::params_from_iter(bound_params.iter()))
            .map_err(|e| CfmlError::database(format!("queryExecute: SQL error: {}", e)))?;

        let last_id = conn.last_insert_rowid();
        build_mutation_result(affected as i64, last_id, sql)
    }
}

/// Rewrite MySQL/MariaDB `@@`-prefixed session/system-variable references into
/// SQLite-valid expressions so a FROM-less capability-detection select doesn't
/// die with `unrecognized token: "@"` on the SQLite backend. Known variables
/// are emulated; any other `@@name` becomes `NULL`. The original column alias
/// is preserved because only the `@@name` token is replaced.
///
/// Only the double-at (`@@`) form is touched — a single `@name` is a valid
/// SQLite named bind parameter and is left alone. References inside
/// single-quoted string literals are skipped.
#[cfg(feature = "sqlite")]
fn rewrite_mysql_system_vars(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let len = bytes.len();
    // Fast path: nothing to do when there's no `@@` anywhere.
    if !sql.contains("@@") {
        return sql.to_string();
    }
    let mut out = String::with_capacity(len);
    let mut i = 0;
    let mut seg_start = 0; // start of unflushed verbatim text (ASCII boundary)
    while i < len {
        // Skip over single-quoted string literals so an `@@` inside one is left
        // alone; the literal stays part of the unflushed verbatim segment.
        if bytes[i] == b'\'' {
            i += 1;
            while i < len && bytes[i] != b'\'' {
                i += 1;
            }
            i += 1; // consume the closing quote (or run off the end)
            continue;
        }
        if bytes[i] == b'@' && i + 1 < len && bytes[i + 1] == b'@' {
            let mut end = i + 2;
            while end < len && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            let name = sql[i + 2..end].to_ascii_lowercase();
            // SQLite has no MySQL system variables — emulate the ones real apps
            // probe, else NULL. `sql_mode` is empty so ONLY_FULL_GROUP_BY
            // detection (ListFindNoCase) reports the relaxed/permissive mode.
            let replacement = match name.as_str() {
                "version" => "sqlite_version()",
                "sql_mode" => "''",
                _ => "NULL",
            };
            // Flush verbatim text before the token, then the replacement.
            out.push_str(&sql[seg_start..i]);
            out.push_str(replacement);
            i = end;
            seg_start = end;
            continue;
        }
        i += 1;
    }
    out.push_str(&sql[seg_start..]);
    out
}

/// Build the bound parameter values for a SQLite query, AND return the SQL to
/// actually prepare/execute. For positional/array params the SQL is unchanged.
/// For NAMED (struct) params each textual `:name` is rewritten to an anonymous
/// positional `?`, with one bound value pushed per occurrence. This is the only
/// way to handle a name that appears MORE THAN ONCE: rusqlite de-duplicates
/// repeated `:name`s in a prepared statement (so `... :x ... :x ...` has a
/// single parameter), but we push one value per textual occurrence — the
/// mismatch errored at bind time ("Wrong number of parameters passed to query.
/// Got 4, needed 3", hit by Wheels' DataChannel lastEventId poll). Rewriting to
/// `?` makes each occurrence its own positional slot bound to the same value.
/// Expand a named cfqueryparam-style value into the bind values it produces,
/// honouring `list=true` (split on `separator`, default `,`). A `null=true`
/// param yields a single NULL; a non-list param yields exactly one value.
/// Mirrors the MySQL path's expand_cfqueryparam_values; SQLite is dynamically
/// typed so no cfsqltype coercion is applied to the elements (GitHub #251).
#[cfg(feature = "sqlite")]
fn expand_sqlite_param_values(v: &CfmlValue) -> Vec<CfmlValue> {
    if let CfmlValue::Struct(s) = v {
        let has_value_key = s.iter().any(|(k, _)| k.eq_ignore_ascii_case("value"));
        if has_value_key {
            let flag = |name: &str| {
                s.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(name))
                    .map(|(_, val)| match val {
                        CfmlValue::Bool(b) => b,
                        CfmlValue::String(st) => {
                            st.eq_ignore_ascii_case("true") || st.eq_ignore_ascii_case("yes")
                        }
                        _ => false,
                    })
                    .unwrap_or(false)
            };
            if flag("null") {
                return vec![CfmlValue::Null];
            }
            let value = s
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("value"))
                .map(|(_, val)| val.clone())
                .unwrap_or(CfmlValue::Null);
            if flag("list") {
                let separator = s
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("separator"))
                    .map(|(_, val)| val.as_string())
                    .unwrap_or_else(|| ",".to_string());
                return value
                    .as_string()
                    .split(&*separator)
                    .map(|part| CfmlValue::string(part.trim().to_string()))
                    .collect();
            }
            return vec![value];
        }
    }
    vec![cfqueryparam_unwrap(v)]
}

#[cfg(feature = "sqlite")]
fn build_sqlite_params(params_arg: &CfmlValue, sql: &str) -> Result<(String, Vec<rusqlite::types::Value>), CfmlError> {
    match params_arg {
        CfmlValue::Null => Ok((sql.to_string(), vec![])),
        CfmlValue::Array(arr) => {
            Ok((sql.to_string(), arr.iter().map(|v| cfml_to_sqlite(&v)).collect()))
        }
        CfmlValue::Struct(map) => {
            let mut result = Vec::new();
            let mut out_sql = String::with_capacity(sql.len());
            let bytes = sql.as_bytes();
            let len = bytes.len();
            let mut i = 0;
            let mut seg_start = 0; // byte offset of unflushed literal text
            while i < len {
                if bytes[i] == b':' && (i == 0 || !bytes[i-1].is_ascii_alphanumeric()) {
                    let start = i + 1;
                    let mut end = start;
                    while end < len && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                        end += 1;
                    }
                    if end > start {
                        let param_name: String = String::from_utf8_lossy(&bytes[start..end]).to_string();
                        let raw = map.iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(&param_name))
                            .map(|(_, v)| v)
                            .unwrap_or(CfmlValue::Null);
                        // Named params may carry a cfqueryparam-style struct
                        // ({value, cfsqltype, null, list, ...}). Expand honouring
                        // `list=true` (split on `separator`, default `,`) so
                        // `IN (:ids)` becomes `IN (?,?,…)` — a list param was
                        // otherwise bound as ONE comma-joined literal and matched
                        // nothing (GitHub #251). Non-list params yield one value
                        // (NULL when null=true). Positional arrays are already
                        // normalized by normalize_query_params() upstream.
                        let expanded = expand_sqlite_param_values(&raw);
                        // Flush literal text before the placeholder(s).
                        // (Slice boundaries are ASCII positions → UTF-8 safe.)
                        out_sql.push_str(&sql[seg_start..i]);
                        if expanded.is_empty() {
                            // An empty list still needs a placeholder so `IN (?)`
                            // stays valid SQL; bind NULL (matches nothing).
                            out_sql.push('?');
                            result.push(rusqlite::types::Value::Null);
                        } else {
                            for (j, val) in expanded.iter().enumerate() {
                                if j > 0 {
                                    out_sql.push(',');
                                }
                                out_sql.push('?');
                                result.push(cfml_to_sqlite(val));
                            }
                        }
                        i = end;
                        seg_start = end;
                        continue;
                    }
                }
                if bytes[i] == b'\'' {
                    // Skip a single-quoted string literal so a `:` inside it is
                    // not treated as a placeholder. The literal stays part of the
                    // current unflushed segment and is copied verbatim.
                    i += 1;
                    while i < len && bytes[i] != b'\'' {
                        i += 1;
                    }
                }
                i += 1;
            }
            out_sql.push_str(&sql[seg_start..]);
            Ok((out_sql, result))
        }
        _ => Ok((sql.to_string(), vec![])),
    }
}

#[cfg(feature = "sqlite")]
fn cfml_to_sqlite(val: &CfmlValue) -> rusqlite::types::Value {
    use rusqlite::types::Value as SqlValue;
    match val {
        CfmlValue::Null => SqlValue::Null,
        CfmlValue::Bool(b) => SqlValue::Integer(if *b { 1 } else { 0 }),
        CfmlValue::Int(i) => SqlValue::Integer(*i),
        CfmlValue::Double(d) => SqlValue::Real(*d),
        CfmlValue::String(s) => SqlValue::Text((**s).clone()),
        CfmlValue::Binary(b) => SqlValue::Blob(b.clone()),
        _ => SqlValue::Text(val.as_string()),
    }
}

#[cfg(feature = "sqlite")]
fn sqlite_to_cfml(val: rusqlite::types::Value) -> CfmlValue {
    use rusqlite::types::Value as SqlValue;
    match val {
        // Lucee/ACF default (full null support OFF): a NULL column reads back as
        // an empty string in query-land, so `q.col EQ ""`, Len(q.col)=0 etc. hold.
        // (Wheels nullifies an association FK then asserts the reloaded column == "".)
        SqlValue::Null => CfmlValue::string(String::new()),
        SqlValue::Integer(i) => CfmlValue::Int(i),
        SqlValue::Real(d) => CfmlValue::Double(d),
        SqlValue::Text(s) => CfmlValue::string(s),
        SqlValue::Blob(b) => CfmlValue::Binary(b),
    }
}

// -----------------------------------------------
// MySQL driver
// -----------------------------------------------

/// Unwrap a cfqueryparam-style param AND apply its declared `cfsqltype` to the
/// bind value. Plain `cfqueryparam_unwrap` returns the raw `value` untouched, so
/// a typed string like `{value:"true", cfsqltype:"cf_sql_bit"}` (how Preside
/// binds a boolean column) reaches the driver as the literal string 'true' —
/// which MySQL rejects for an integer/bit column ("Incorrect integer value:
/// 'true'"). The positional-array path already coerces via normalize_query_params;
/// the named path must do the same. Coercion mirrors coerce_by_sqltype_value:
/// cf_sql_bit/boolean "true"/"false"/"1"/"0" -> Bool (bound as 1/0), the integer/
/// float families parse the string, everything else is left as-is.
#[cfg(feature = "mysql_db")]
fn cfqueryparam_unwrap_typed(v: &CfmlValue) -> CfmlValue {
    let unwrapped = cfqueryparam_unwrap(v);
    if matches!(unwrapped, CfmlValue::Null) {
        return unwrapped; // null=true (or a genuine NULL value) — no type coercion
    }
    if let CfmlValue::Struct(s) = v {
        if let Some(cfsqltype) = s
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("cfsqltype"))
            .map(|(_, val)| val.as_string().to_lowercase())
        {
            return coerce_by_sqltype_value(&unwrapped, &cfsqltype);
        }
    }
    unwrapped
}

/// Expand a single cfqueryparam-style value into the bound values it produces,
/// honouring `list=true` (split on `separator`, default `,`) and applying
/// `cfsqltype` coercion to each element. A non-list param yields exactly one
/// value; a `null=true` param yields a single NULL. Mirrors the list-expansion
/// the positional-array path does in `normalize_query_params`, so the named
/// (`:name`) path produces the same `IN (?,?,…)` binding instead of stuffing a
/// chr(31)-joined list into a single placeholder.
#[cfg(feature = "mysql_db")]
fn expand_cfqueryparam_values(v: &CfmlValue) -> Vec<CfmlValue> {
    if let CfmlValue::Struct(s) = v {
        let has_value_key = s.iter().any(|(k, _)| k.eq_ignore_ascii_case("value"));
        if has_value_key {
            let is_null = s.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("null"))
                .map(|(_, val)| match val {
                    CfmlValue::Bool(b) => b,
                    CfmlValue::String(st) => st.eq_ignore_ascii_case("true") || st.eq_ignore_ascii_case("yes"),
                    _ => false,
                })
                .unwrap_or(false);
            if is_null {
                return vec![CfmlValue::Null];
            }
            let value = s.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("value"))
                .map(|(_, val)| val.clone())
                .unwrap_or(CfmlValue::Null);
            let cfsqltype = s.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("cfsqltype"))
                .map(|(_, val)| val.as_string().to_lowercase())
                .unwrap_or_else(|| "cf_sql_varchar".to_string());
            let is_list = s.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("list"))
                .map(|(_, val)| match val {
                    CfmlValue::Bool(b) => b,
                    CfmlValue::String(st) => st.eq_ignore_ascii_case("true") || st.eq_ignore_ascii_case("yes"),
                    _ => false,
                })
                .unwrap_or(false);
            if is_list {
                let separator = s.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("separator"))
                    .map(|(_, val)| val.as_string())
                    .unwrap_or_else(|| ",".to_string());
                return value
                    .as_string()
                    .split(&*separator)
                    .map(|part| coerce_by_sqltype(part.trim(), &cfsqltype))
                    .collect();
            }
            return vec![coerce_by_sqltype_value(&value, &cfsqltype)];
        }
    }
    vec![cfqueryparam_unwrap_typed(v)]
}

/// Rewrite a SQL string's named `:placeholder`s to positional `?`, returning the
/// rewritten SQL and the bound values in positional order (one value pushed per
/// textual occurrence, so a repeated `:name` binds correctly). Name characters
/// are ASCII alphanumeric + `_` (case-insensitively matched against the params
/// struct), so camelCase names like `:dateCreated` are captured whole — unlike
/// the mysql crate's lowercase-only named-param parser. Single-quoted string
/// literals are skipped so a `:` inside them is not treated as a placeholder.
/// `list=true` params expand to `?,?,…` (one per element). Mirrors
/// build_sqlite_params' rewrite.
#[cfg(feature = "mysql_db")]
fn mysql_named_to_positional(sql: &str, map: &CfmlStruct) -> (String, Vec<CfmlValue>) {
    let mut result = Vec::new();
    let mut out_sql = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut seg_start = 0; // byte offset of unflushed literal text
    while i < len {
        if bytes[i] == b':' && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric()) {
            let start = i + 1;
            let mut end = start;
            while end < len && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end > start {
                let param_name: String = String::from_utf8_lossy(&bytes[start..end]).to_string();
                let raw = map
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&param_name))
                    .map(|(_, v)| v)
                    .unwrap_or(CfmlValue::Null);
                let expanded = expand_cfqueryparam_values(&raw);
                out_sql.push_str(&sql[seg_start..i]);
                if expanded.is_empty() {
                    // An empty list still needs a placeholder so `IN (?)` stays
                    // valid SQL; bind NULL (matches nothing, as `IN (NULL)`).
                    out_sql.push('?');
                    result.push(CfmlValue::Null);
                } else {
                    for (j, val) in expanded.into_iter().enumerate() {
                        if j > 0 {
                            out_sql.push(',');
                        }
                        out_sql.push('?');
                        result.push(val);
                    }
                }
                i = end;
                seg_start = end;
                continue;
            }
        }
        if bytes[i] == b'\'' {
            i += 1;
            while i < len && bytes[i] != b'\'' {
                i += 1;
            }
        }
        i += 1;
    }
    out_sql.push_str(&sql[seg_start..]);
    (out_sql, result)
}

#[cfg(feature = "mysql_db")]
/// Query-timeout watchdog for MySQL/MariaDB, mirroring JDBC
/// Statement.setQueryTimeout. On construction it starts a thread that waits
/// `secs`; if the guarded query has not finished by then it opens a fresh
/// pooled connection and issues `KILL QUERY <conn_id>`, which aborts just the
/// running statement (the connection itself stays usable, so returning it to
/// the pool is safe). Dropping the guard signals the query finished and joins
/// the thread. `fired()` reports whether the KILL was actually sent, so the
/// caller can translate the resulting "query interrupted" error into a
/// timeout error.
struct MysqlQueryTimeout {
    done_tx: std::sync::mpsc::Sender<()>,
    fired: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

#[cfg(feature = "mysql_db")]
impl MysqlQueryTimeout {
    fn start(url: String, conn_id: u32, secs: u32) -> Self {
        use std::sync::atomic::AtomicBool;
        use std::sync::mpsc::{channel, RecvTimeoutError};
        let (done_tx, rx) = channel::<()>();
        let fired = std::sync::Arc::new(AtomicBool::new(false));
        let fired_thread = fired.clone();
        let handle = std::thread::spawn(move || {
            match rx.recv_timeout(std::time::Duration::from_secs(secs as u64)) {
                // Query finished (or guard dropped) before the deadline — stand down.
                Ok(()) | Err(RecvTimeoutError::Disconnected) => {}
                // Deadline elapsed while the query was still running — KILL it.
                Err(RecvTimeoutError::Timeout) => {
                    fired_thread.store(true, std::sync::atomic::Ordering::SeqCst);
                    if let Ok(pool) = get_mysql_pool(&url) {
                        if let Ok(mut kc) = pool.get_conn() {
                            use mysql::prelude::Queryable;
                            let _ = kc.query_drop(format!("KILL QUERY {}", conn_id));
                        }
                    }
                }
            }
        });
        MysqlQueryTimeout { done_tx, fired, handle: Some(handle) }
    }

    fn fired(&self) -> bool {
        self.fired.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(feature = "mysql_db")]
impl Drop for MysqlQueryTimeout {
    fn drop(&mut self) {
        let _ = self.done_tx.send(());
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(feature = "mysql_db")]
fn execute_mysql(
    url: &str,
    sql: &str,
    params_arg: &CfmlValue,
    return_type: &str,
    timeout_secs: Option<u32>,
) -> CfmlResult {
    // Run on the request's held connection (connection-per-request), then return
    // it to the request-scoped cache so its session state persists for the rest of
    // the request and is reset only at the request boundary. Returning the
    // connection on BOTH the ok and error paths keeps it available (a query error
    // does not break the connection) and ensures it is eventually reset.
    let pool = get_mysql_pool(url)?;
    let mut conn = checkout_request_mysql_conn(url, &pool)?;
    let result = execute_mysql_on_conn(&mut conn, url, sql, params_arg, return_type, timeout_secs);
    return_request_mysql_conn(url, conn);
    result
}

#[cfg(feature = "mysql_db")]
fn execute_mysql_on_conn(
    conn: &mut mysql::PooledConn,
    url: &str,
    sql: &str,
    params_arg: &CfmlValue,
    return_type: &str,
    timeout_secs: Option<u32>,
) -> CfmlResult {
    use mysql::*;
    use mysql::prelude::*;

    // Build the SQL we actually execute plus its bound params. For NAMED (struct)
    // params we rewrite `:name` placeholders to positional `?` ourselves rather
    // than handing `:name` SQL to the mysql crate's Params::Named parser: that
    // parser only consumes lowercase identifier chars after `:`, so a camelCase
    // placeholder is TRUNCATED at its first uppercase letter (`:dateCreated` ->
    // `?` + stray `Created`, a MySQL syntax error). Preside's object SQL uses
    // camelCase property names throughout. Mirrors build_sqlite_params.
    let (final_sql, params): (std::borrow::Cow<str>, Params) = match params_arg {
        CfmlValue::Array(arr) => {
            let vals: Vec<mysql::Value> = arr.iter().map(|v| cfml_to_mysql_value(&v)).collect();
            let p = if vals.is_empty() { Params::Empty } else { Params::Positional(vals) };
            (std::borrow::Cow::Borrowed(sql), p)
        }
        // An empty params struct means "no parameters" (Preside passes `{}` to
        // placeholder-free SQL). Lucee treats empty params as no params.
        CfmlValue::Struct(map) if map.is_empty() => (std::borrow::Cow::Borrowed(sql), Params::Empty),
        CfmlValue::Struct(map) => {
            let (rewritten, vals_cfml) = mysql_named_to_positional(sql, map);
            let vals: Vec<mysql::Value> =
                vals_cfml.iter().map(|v| cfml_to_mysql_value(v)).collect();
            let p = if vals.is_empty() { Params::Empty } else { Params::Positional(vals) };
            (std::borrow::Cow::Owned(rewritten), p)
        }
        _ => (std::borrow::Cow::Borrowed(sql), Params::Empty),
    };
    let sql: &str = &final_sql;

    // Arm the query-timeout watchdog (JDBC setQueryTimeout equivalent) if a
    // positive timeout was requested. It KILLs this connection's running query
    // server-side once the deadline passes; we then translate the resulting
    // "query interrupted" error into a timeout error the caller can detect.
    let watchdog = timeout_secs.map(|secs| {
        let conn_id = conn.connection_id();
        MysqlQueryTimeout::start(url.to_string(), conn_id, secs)
    });
    // Map a query error to a timeout error when the watchdog fired (so callers
    // catching `database` errors see "timeout" in the message), else pass through.
    let map_err = |e: mysql::Error, ctx: &str| -> CfmlError {
        if let (Some(w), Some(secs)) = (watchdog.as_ref(), timeout_secs) {
            if w.fired() {
                return CfmlError::database(format!(
                    "queryExecute: MySQL query exceeded timeout of {} second(s) and was cancelled: {}",
                    secs, e
                ));
            }
        }
        CfmlError::database(format!("queryExecute: MySQL {}: {}", ctx, e))
    };

    if mysql_returns_rows(sql) {
        // Use a streaming QueryResult so the column metadata is read from the
        // result set itself — NOT inferred from the first row. A zero-row result
        // (`SELECT * FROM t WHERE 0=1`, the canonical "give me the column list"
        // idiom) still carries its columns, so `query.columnList` is populated.
        // (Deriving columns from `result.first()` returned an EMPTY column list
        // for 0 rows, so Masa's schema migrations mis-fired: `<cfif not
        // listFindNoCase(rsCheck.columnList,"comments")>` was always true and it
        // tried to rename a column that no longer existed.)
        let mut result = conn.exec_iter(sql, &params)
            .map_err(|e| map_err(e, "query error"))?;
        let cols_meta = result.columns();
        let raw_columns: Vec<String> = cols_meta
            .as_ref()
            .iter()
            .map(|c| c.name_str().to_string())
            .collect();
        let col_types: Vec<mysql::consts::ColumnType> = cols_meta
            .as_ref()
            .iter()
            .map(|c| c.column_type())
            .collect();
        let (columns, keep) = dedup_result_columns(raw_columns);

        let mut rows: Vec<ValueMap> = Vec::new();
        for row_result in result.by_ref() {
            let row = row_result.map_err(|e| map_err(e, "query error"))?;
            let mut row_map = ValueMap::default();
            for (out_i, &src_i) in keep.iter().enumerate() {
                let val: mysql::Value = row.get(src_i).unwrap_or(mysql::Value::NULL);
                row_map.insert(columns[out_i].clone(), mysql_value_to_cfml_typed(val, col_types.get(src_i).copied()));
            }
            rows.push(row_map);
        }

        build_query_result(columns, rows, sql, return_type)
    } else {
        conn.exec_drop(sql, &params)
            .map_err(|e| map_err(e, "error"))?;

        let affected = conn.affected_rows() as i64;
        let last_id = conn.last_insert_id() as i64;
        build_mutation_result(affected, last_id, sql)
    }
}

#[cfg(feature = "mysql_db")]
fn cfml_to_mysql_value(val: &CfmlValue) -> mysql::Value {
    match val {
        CfmlValue::Null => mysql::Value::NULL,
        CfmlValue::Bool(b) => mysql::Value::from(*b),
        CfmlValue::Int(i) => mysql::Value::from(*i),
        CfmlValue::Double(d) => mysql::Value::from(*d),
        CfmlValue::String(s) => mysql::Value::from(s.as_str()),
        CfmlValue::Binary(b) => mysql::Value::Bytes(b.clone()),
        _ => mysql::Value::from(val.as_string()),
    }
}

#[cfg(feature = "mysql_db")]
fn mysql_value_to_cfml(val: mysql::Value) -> CfmlValue {
    match val {
        // Lucee/ACF default (full null support OFF): a NULL column reads back as
        // an empty string in query-land, so `q.col EQ ""`, Len(q.col)=0, and
        // passing `q.col` to a required arg all behave. Matches sqlite_to_cfml.
        // (Preside's homepage has parent_page=NULL; queryRowToStruct + a
        // positional call to _isManagedPage(parentId,...) broke on `Null`.)
        mysql::Value::NULL => CfmlValue::string(String::new()),
        mysql::Value::Int(i) => CfmlValue::Int(i),
        mysql::Value::UInt(u) => CfmlValue::Int(u as i64),
        mysql::Value::Float(f) => CfmlValue::Double(f as f64),
        mysql::Value::Double(d) => CfmlValue::Double(d),
        mysql::Value::Bytes(b) => {
            match String::from_utf8(b.clone()) {
                Ok(s) => CfmlValue::string(s),
                Err(_) => CfmlValue::Binary(b),
            }
        }
        // DATE / DATETIME / TIMESTAMP all arrive as `Value::Date`; a DATE column
        // (and a DATETIME at midnight) carries all-zero time fields. Lucee/ACF
        // surface EVERY temporal column as a full datetime — a DATE becomes a
        // datetime at midnight, and a DATETIME never drops its (possibly
        // midnight) time. Always emit the full `YYYY-MM-DD HH:MM:SS` form (the
        // same canonical datetime string now()/createDateTime produce), so the
        // time component is never lost. (GH #273 — the old all-zero-time
        // heuristic collapsed a real DATETIME to a bare date.)
        mysql::Value::Date(y, mo, d, h, mi, s, _us) => {
            CfmlValue::string(format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, mo, d, h, mi, s))
        }
        // TIME columns — CFML idiom is the epoch-style 1899-12-30 prefix so
        // dateFormat/timeFormat work. `days` can carry overflow; fold into hours.
        mysql::Value::Time(neg, days, h, mi, s, _us) => {
            let total_h = (days as u64) * 24 + h as u64;
            let sign = if neg { "-" } else { "" };
            CfmlValue::string(format!("1899-12-30 {}{:02}:{:02}:{:02}", sign, total_h, mi, s))
        }
    }
}

/// Column-type-aware value conversion. Most columns need no type context, but a
/// few MySQL types are ambiguous at the `Value` level and must key off the
/// result-set metadata:
///   - BIT columns arrive as raw big-endian `Bytes` — indistinguishable from a
///     BINARY/BLOB — yet Lucee/ACF surface the numeric value (`BIT(1)` -> 0/1,
///     wider BIT -> the integer). (GH #274)
#[cfg(feature = "mysql_db")]
fn mysql_value_to_cfml_typed(
    val: mysql::Value,
    col_type: Option<mysql::consts::ColumnType>,
) -> CfmlValue {
    use mysql::consts::ColumnType;
    if matches!(col_type, Some(ColumnType::MYSQL_TYPE_BIT)) {
        if let mysql::Value::Bytes(ref b) = val {
            // Big-endian bytes -> integer (BIT is stored MSB-first).
            let mut n: i64 = 0;
            for &byte in b.iter() {
                n = (n << 8) | byte as i64;
            }
            return CfmlValue::Int(n);
        }
    }
    mysql_value_to_cfml(val)
}

// -----------------------------------------------
// PostgreSQL driver
// -----------------------------------------------

#[cfg(feature = "postgres_db")]
fn execute_postgres(url: &str, sql: &str, params_arg: &CfmlValue, return_type: &str) -> CfmlResult {
    let pool = get_postgres_pool(url)?;

    // The pool does not ping on checkout (Lucee parity / remote-DB perf), so a
    // server-closed idle connection (Neon scale-to-zero / suspend-resume,
    // failover, network drop) is handed back out and only surfaces here. When the
    // server drops MANY idle sessions at once the pool can hold several dead
    // connections, and r2d2 establishes their replacements asynchronously — so an
    // immediate retry may draw a *second* stale connection before a fresh one is
    // ready. Retry the (retry-safe) statement, discarding each broken connection
    // so r2d2 evicts it, until a live connection runs it or the attempt budget is
    // spent. Bounded by the pool size so a statement that genuinely keeps failing
    // connection-level can't loop forever.
    let mut last_err: Option<CfmlError> = None;
    for _ in 0..=PG_POOL_MAX_SIZE {
        let mut conn = pool.get()
            .map_err(|e| CfmlError::database(format!("queryExecute: PostgreSQL connection error: {}", e)))?;
        match run_postgres_on_conn(&mut conn, sql, params_arg, return_type) {
            Ok(value) => return Ok(value),
            Err(err) if err.retry_safe => {
                drop(conn);
                last_err = Some(err.error);
            }
            Err(err) => return Err(err.error),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        CfmlError::database("queryExecute: PostgreSQL connection error: exhausted pool retries".to_string())
    }))
}

#[cfg(feature = "postgres_db")]
fn run_postgres_on_conn(
    conn: &mut PgConn,
    sql: &str,
    params_arg: &CfmlValue,
    return_type: &str,
) -> Result<CfmlValue, PgRunError> {
    let result = run_postgres_statements(&mut conn.client, &mut conn.stmt_cache, sql, params_arg, return_type);
    if let Err(err) = &result {
        if err.connection_broken {
            conn.broken = true;
        }
    }
    result
}

/// Shared PostgreSQL execution for both the pooled (`execute_postgres`) and
/// transaction (`execute_postgres_with_conn`) paths.
///
/// SELECTs run as a single `query`. Non-SELECT mutations are split into one
/// parameterized statement each (see `pg_sql::prepare_pg_statements`) and run
/// in order, summing affected rows — the `postgres` crate's `execute` only
/// accepts a single command per call, so multi-statement framework mutations
/// would otherwise fail. See docs/compatibility-notes/postgres-multi-statement-mutations.md.
#[cfg(feature = "postgres_db")]
/// Format a `postgres::Error` so the server's underlying cause survives into
/// `cfcatch.message`. The top-level `Display` is a bare "db error"; the real
/// SQLSTATE message (e.g. `ERROR: function zz_x() does not exist`) and any
/// further detail live on the `source()` chain, which Lucee surfaces. Walk the
/// chain and append each cause so failures are diagnosable from CFML and logs.
#[cfg(feature = "postgres_db")]
fn format_pg_error(context: &str, e: &postgres::Error) -> String {
    use std::error::Error;
    let mut msg = format!("{}: {}", context, e);
    let mut src: Option<&(dyn Error + 'static)> = e.source();
    while let Some(s) = src {
        msg.push_str(" — ");
        msg.push_str(&s.to_string());
        src = s.source();
    }
    msg
}

#[cfg(feature = "postgres_db")]
struct PgRunError {
    error: CfmlError,
    connection_broken: bool,
    retry_safe: bool,
}

#[cfg(feature = "postgres_db")]
impl PgRunError {
    fn from_cfml(error: CfmlError) -> Self {
        Self {
            error,
            connection_broken: false,
            retry_safe: false,
        }
    }

    fn from_postgres(context: &str, e: postgres::Error, retry_safe: bool) -> Self {
        let connection_broken = pg_error_is_connection_fatal(&e);
        Self {
            error: CfmlError::database(format_pg_error(context, &e)),
            connection_broken,
            retry_safe: connection_broken && retry_safe,
        }
    }
}

/// True when a `postgres` error means the pooled connection is no longer usable.
///
/// `is_closed()` only flips once the socket is observed closed (e.g. Neon proxy
/// dropping an idle TCP connection). A server-side FATAL — `pg_terminate_backend`,
/// a smart/fast shutdown, or a failover — instead sends an error message that
/// terminates the session while `is_closed()` is still false, so it must be
/// recognised by its SqlState. We treat operator-intervention (class 57P0x) and
/// connection-exception (class 08) states as connection-fatal; ordinary SQL
/// errors (syntax, constraint, etc.) are NOT, so genuine failures still surface
/// immediately rather than being retried.
#[cfg(feature = "postgres_db")]
fn pg_error_is_connection_fatal(e: &postgres::Error) -> bool {
    if e.is_closed() {
        return true;
    }
    match e.code() {
        Some(code) => matches!(
            code.code(),
            "57P01"   // admin_shutdown — "terminating connection due to administrator command"
                | "57P02" // crash_shutdown
                | "57P03" // cannot_connect_now
                | "08000" // connection_exception
                | "08001" // sqlclient_unable_to_establish_sqlconnection
                | "08003" // connection_does_not_exist
                | "08004" // sqlserver_rejected_establishment_of_sqlconnection
                | "08006" // connection_failure
        ),
        None => false,
    }
}

#[cfg(feature = "postgres_db")]
fn run_postgres_statements(
    client: &mut postgres::Client,
    cache: &mut PgStmtCache,
    sql: &str,
    params_arg: &CfmlValue,
    return_type: &str,
) -> Result<CfmlValue, PgRunError> {
    let returns_rows = postgres_returns_rows(sql);
    let statements = crate::pg_sql::prepare_pg_statements(sql, params_arg, !returns_rows)
        .map_err(PgRunError::from_cfml)?;

    if returns_rows {
        // split=false guarantees exactly one statement.
        let stmt = &statements[0];
        let pg_params: Vec<PgParam> = stmt.params.iter().map(cfml_to_pg_param).collect();
        let param_refs: Vec<&(dyn postgres::types::ToSql + Sync)> = pg_params.iter()
            .map(|v| v as &(dyn postgres::types::ToSql + Sync))
            .collect();
        // Cached prepared statement → one round-trip in steady state. On a stale
        // cached plan (DDL changed the table), evict and re-prepare once.
        let prepared = pg_prepare_cached(client, cache, stmt.sql.as_str())
            .map_err(|e| PgRunError::from_postgres("queryExecute: PostgreSQL prepare error", e, true))?;
        let rows = match client.query(&prepared, &param_refs) {
            Ok(r) => r,
            Err(e) if is_stale_cached_plan(&e) => {
                cache.remove(stmt.sql.as_str());
                let prepared = pg_prepare_cached(client, cache, stmt.sql.as_str())
                    .map_err(|e| PgRunError::from_postgres("queryExecute: PostgreSQL prepare error", e, true))?;
                client.query(&prepared, &param_refs)
                    .map_err(|e| PgRunError::from_postgres("queryExecute: PostgreSQL query error", e, true))?
            }
            Err(e) => return Err(PgRunError::from_postgres("queryExecute: PostgreSQL query error", e, true)),
        };

        let raw_columns: Vec<String> = prepared.columns().iter().map(|c| c.name().to_string()).collect();
        let (columns, keep) = dedup_result_columns(raw_columns);

        let mut result_rows: Vec<ValueMap> = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut row_map = ValueMap::default();
            for (out_i, &src_i) in keep.iter().enumerate() {
                row_map.insert(columns[out_i].clone(), postgres_row_to_cfml(row, src_i));
            }
            result_rows.push(row_map);
        }
        build_query_result(columns, result_rows, sql, return_type).map_err(PgRunError::from_cfml)
    } else {
        let mut total: i64 = 0;
        let mut executed_any = false;
        for stmt in &statements {
            let pg_params: Vec<PgParam> = stmt.params.iter().map(cfml_to_pg_param).collect();
            let param_refs: Vec<&(dyn postgres::types::ToSql + Sync)> = pg_params.iter()
                .map(|v| v as &(dyn postgres::types::ToSql + Sync))
                .collect();
            let prepared = pg_prepare_cached(client, cache, stmt.sql.as_str())
                .map_err(|e| PgRunError::from_postgres("queryExecute: PostgreSQL prepare error", e, !executed_any))?;
            let affected = match client.execute(&prepared, &param_refs) {
                Ok(n) => n,
                Err(e) if is_stale_cached_plan(&e) => {
                    cache.remove(stmt.sql.as_str());
                    let prepared = pg_prepare_cached(client, cache, stmt.sql.as_str())
                        .map_err(|e| PgRunError::from_postgres("queryExecute: PostgreSQL prepare error", e, !executed_any))?;
                    client.execute(&prepared, &param_refs)
                        .map_err(|e| PgRunError::from_postgres("queryExecute: PostgreSQL error", e, false))?
                }
                Err(e) => return Err(PgRunError::from_postgres("queryExecute: PostgreSQL error", e, false)),
            };
            executed_any = true;
            total += affected as i64;
        }
        build_mutation_result(total, 0, sql).map_err(PgRunError::from_cfml) // PG uses RETURNING, not last_insert_id
    }
}

#[derive(Debug)]
#[cfg(feature = "postgres_db")]
enum PgParam {
    Null,
    Bool(bool),
    Int(i64),
    Double(f64),
    Text(String),
    Bytes(Vec<u8>),
}

/// Append `s` to the postgres wire buffer as raw text bytes and report a
/// present value. Used for `Type::UNKNOWN` targets, where PostgreSQL hasn't
/// resolved a concrete type yet and accepts the text representation, letting
/// the server coerce it. See docs/compatibility-notes/postgres-unknown-params.md.
#[cfg(feature = "postgres_db")]
fn write_pg_text(
    s: &str,
    out: &mut postgres::types::private::BytesMut,
) -> Result<postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
    out.extend_from_slice(s.as_bytes());
    Ok(postgres::types::IsNull::No)
}

/// Stringify an f64 the way PostgreSQL text input expects: whole numbers lose
/// their `.0` (`250.0` -> `250`) so an UNKNOWN/integer target accepts them.
#[cfg(feature = "postgres_db")]
fn format_pg_numeric_f64(d: f64) -> String {
    if d.is_finite() && d.fract() == 0.0 && d.abs() < 9.007_199_254_740_992e15 {
        format!("{}", d as i64)
    } else {
        format!("{}", d)
    }
}

#[cfg(feature = "postgres_db")]
type PgToSqlResult = Result<postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>;

/// Bind a string by parsing it to `target`'s native type. CFML values are very
/// often strings (form/URL values are untyped), so `"123"` must reach an `int4`
/// column as 4-byte binary, not raw text — the `postgres` crate sends all
/// parameters in BINARY format, so the wire bytes must match the column type.
#[cfg(feature = "postgres_db")]
fn bind_parse_err(s: &str, ty: &postgres::types::Type, e: impl std::fmt::Display) -> Box<dyn std::error::Error + Sync + Send> {
    format!("queryExecute: cannot bind \"{}\" as PostgreSQL {}: {}", s, ty.name(), e).into()
}

/// Parse a CFML time string ("HH:MM:SS", "HH:MM", with optional fractional
/// seconds) to a `NaiveTime` for binding to a PostgreSQL `time` column. Falls
/// back to the time component of a full date/time string.
#[cfg(feature = "postgres_db")]
fn parse_cfml_time(s: &str) -> Option<NaiveTime> {
    let t = s.trim();
    for fmt in &["%H:%M:%S%.f", "%H:%M:%S", "%H:%M"] {
        if let Ok(nt) = NaiveTime::parse_from_str(t, fmt) {
            return Some(nt);
        }
    }
    // "2024-03-15 10:30:45" → 10:30:45
    parse_cfml_date(t).map(|dt| dt.time())
}

/// Parse a date/time string to an absolute instant in UTC, for binding to a
/// PostgreSQL `timestamptz` column. RFC 3339 / ISO 8601 strings carrying a
/// numeric offset or 'Z' suffix ("2026-06-10T07:20:42.177+00:00", "...Z") are
/// honoured — the offset is applied to recover the true instant. A zone-less
/// value (a plain CFML datetime) carries no offset, so its wall-clock is
/// interpreted as UTC (matching the existing behaviour for a UTC server).
#[cfg(feature = "postgres_db")]
fn parse_cfml_datetime_utc(s: &str) -> Option<chrono::DateTime<Utc>> {
    let t = s.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
        return Some(dt.with_timezone(&Utc));
    }
    parse_cfml_date(t).map(|ndt| Utc.from_utc_datetime(&ndt))
}

/// Encode a CFML vector-literal string ("[1,0,0]") into pgvector's binary wire
/// format: `u16` dimension count, `u16` flags (always 0), then one big-endian
/// IEEE-754 `f32` per element. pgvector extension types carry no static OID, so
/// the `to_sql` dispatch reaches this by matching the type name "vector".
#[cfg(feature = "postgres_db")]
fn encode_pg_vector(
    s: &str,
    ty: &postgres::types::Type,
    out: &mut postgres::types::private::BytesMut,
) -> PgToSqlResult {
    let trimmed = s.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|x| x.strip_suffix(']'))
        .ok_or_else(|| bind_parse_err(s, ty, "expected a \"[..]\" vector literal"))?;
    let mut vals: Vec<f32> = Vec::new();
    for part in inner.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        vals.push(
            p.parse::<f32>()
                .map_err(|e| bind_parse_err(s, ty, format!("bad vector element \"{}\": {}", p, e)))?,
        );
    }
    if vals.len() > u16::MAX as usize {
        return Err(bind_parse_err(s, ty, "vector has too many dimensions"));
    }
    out.extend_from_slice(&(vals.len() as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // unused flags word
    for v in vals {
        out.extend_from_slice(&v.to_be_bytes());
    }
    Ok(postgres::types::IsNull::No)
}

#[cfg(feature = "postgres_db")]
impl postgres::types::ToSql for PgParam {
    fn to_sql(&self, ty: &postgres::types::Type, out: &mut postgres::types::private::BytesMut) -> PgToSqlResult {
        use postgres::types::Type;
        use std::str::FromStr;
        match self {
            PgParam::Null => Ok(postgres::types::IsNull::Yes),

            // Integers: encode at the column's exact width. `i64::to_sql` always
            // writes 8 bytes, which a 2/4-byte column rejects ("incorrect binary
            // data format"), so downcast per target type. TEXT-family and
            // UNKNOWN targets take the text form (their binary repr is utf-8;
            // UNKNOWN is parsed as cstring server-side).
            PgParam::Int(i) => match *ty {
                Type::INT2 => (*i as i16).to_sql(ty, out),
                Type::INT4 => (*i as i32).to_sql(ty, out),
                Type::INT8 => i.to_sql(ty, out),
                Type::OID => (*i as u32).to_sql(ty, out),
                Type::FLOAT4 => (*i as f32).to_sql(ty, out),
                Type::FLOAT8 => (*i as f64).to_sql(ty, out),
                Type::NUMERIC => rust_decimal::Decimal::from(*i).to_sql(ty, out),
                Type::BOOL => (*i != 0).to_sql(ty, out),
                _ => write_pg_text(&i.to_string(), out),
            },

            // Doubles: likewise width-correct; whole numbers drop their `.0` in
            // the text fallback so integer/unknown targets accept them.
            PgParam::Double(d) => match *ty {
                Type::FLOAT8 => d.to_sql(ty, out),
                Type::FLOAT4 => (*d as f32).to_sql(ty, out),
                Type::INT2 => (*d as i16).to_sql(ty, out),
                Type::INT4 => (*d as i32).to_sql(ty, out),
                Type::INT8 => (*d as i64).to_sql(ty, out),
                Type::NUMERIC => match rust_decimal::Decimal::try_from(*d) {
                    Ok(dec) => dec.to_sql(ty, out),
                    Err(_) => write_pg_text(&format_pg_numeric_f64(*d), out),
                },
                _ => write_pg_text(&format_pg_numeric_f64(*d), out),
            },

            PgParam::Bool(b) => match *ty {
                Type::BOOL => b.to_sql(ty, out),
                Type::INT2 => (*b as i16).to_sql(ty, out),
                Type::INT4 => (*b as i32).to_sql(ty, out),
                Type::INT8 => (*b as i64).to_sql(ty, out),
                _ => write_pg_text(if *b { "true" } else { "false" }, out),
            },

            // Strings: parse to the column's native type so untyped CFML strings
            // bind correctly. UUID parsing fixes the documented #52 case; the
            // numeric arms fix the common "string id -> int column" pattern.
            // TEXT-family / UNKNOWN / other fall through to raw utf-8 bytes
            // (`String::to_sql` writes the bytes regardless of `ty`).
            PgParam::Text(s) => {
                // pgvector and other extension types have no static OID, so they
                // must be matched by type NAME, not against the `Type::*` consts.
                // pgvector wire format: u16 dim, u16 flags(0), big-endian f32 each.
                if ty.name() == "vector" {
                    return encode_pg_vector(s, ty, out);
                }
                match *ty {
                Type::UUID => uuid::Uuid::parse_str(s.trim())
                    .map_err(|e| bind_parse_err(s, ty, e))?
                    .to_sql(ty, out),
                Type::INT2 => s.trim().parse::<i16>().map_err(|e| bind_parse_err(s, ty, e))?.to_sql(ty, out),
                Type::INT4 => s.trim().parse::<i32>().map_err(|e| bind_parse_err(s, ty, e))?.to_sql(ty, out),
                Type::INT8 => s.trim().parse::<i64>().map_err(|e| bind_parse_err(s, ty, e))?.to_sql(ty, out),
                Type::OID => s.trim().parse::<u32>().map_err(|e| bind_parse_err(s, ty, e))?.to_sql(ty, out),
                Type::FLOAT4 => s.trim().parse::<f32>().map_err(|e| bind_parse_err(s, ty, e))?.to_sql(ty, out),
                Type::FLOAT8 => s.trim().parse::<f64>().map_err(|e| bind_parse_err(s, ty, e))?.to_sql(ty, out),
                Type::NUMERIC => rust_decimal::Decimal::from_str(s.trim())
                    .map_err(|e| bind_parse_err(s, ty, e))?
                    .to_sql(ty, out),
                Type::BOOL => {
                    let t = s.trim();
                    let b = t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("t")
                        || t.eq_ignore_ascii_case("yes") || t == "1";
                    b.to_sql(ty, out)
                }
                // Temporal columns expect binary wire bytes (int64/int32), not
                // text — Lucee binds CFML date/time strings here. Parse the CFML
                // value and let chrono's ToSql impls write the binary form.
                Type::TIMESTAMP => parse_cfml_date(s)
                    .ok_or_else(|| bind_parse_err(s, ty, "not a recognised date/time"))?
                    .to_sql(ty, out),
                Type::TIMESTAMPTZ => {
                    // An ISO 8601 string carrying a zone offset or 'Z' is bound
                    // at the true instant it denotes; a zone-less CFML datetime
                    // is interpreted as UTC wall-clock (Lucee binds the
                    // server-tz instant — for a UTC server, identical). The
                    // server then renders it back in its session TimeZone.
                    parse_cfml_datetime_utc(s)
                        .ok_or_else(|| bind_parse_err(s, ty, "not a recognised date/time"))?
                        .to_sql(ty, out)
                }
                Type::DATE => parse_cfml_date(s)
                    .ok_or_else(|| bind_parse_err(s, ty, "not a recognised date"))?
                    .date()
                    .to_sql(ty, out),
                Type::TIME => parse_cfml_time(s)
                    .ok_or_else(|| bind_parse_err(s, ty, "not a recognised time"))?
                    .to_sql(ty, out),
                // json/jsonb want the JSON text parsed to a value; serde_json's
                // ToSql writes jsonb's required 0x01 version prefix for us (raw
                // text fails with "unsupported jsonb version number").
                Type::JSON | Type::JSONB => {
                    let v: serde_json::Value = serde_json::from_str(s.trim())
                        .map_err(|e| bind_parse_err(s, ty, e))?;
                    v.to_sql(ty, out)
                }
                _ => s.to_sql(ty, out),
                }
            },

            PgParam::Bytes(b) => b.to_sql(ty, out),
        }
    }

    fn accepts(_ty: &postgres::types::Type) -> bool {
        true
    }

    // rust-postgres sends every parameter in BINARY wire format by default,
    // whereas Lucee/pgjdbc send parameters as TEXT and let the server parse
    // them. We binary-encode the common scalar types above; for everything
    // else (arrays, inet/cidr/macaddr, interval, timetz, ranges, hstore, …)
    // `to_sql` writes the value's text form, so it MUST go out as Text format
    // for the server to parse it — otherwise the server reads the text bytes as
    // a binary payload and rejects them ("incorrect binary data format").
    // This mirrors `to_sql`'s per-(variant, type) decision exactly.
    fn encode_format(&self, ty: &postgres::types::Type) -> postgres::types::Format {
        use postgres::types::{Format, Type};
        // pgvector is binary-encoded (matched by name; no static OID).
        if ty.name() == "vector" {
            return match self {
                PgParam::Text(_) => Format::Binary,
                _ => Format::Text,
            };
        }
        let binary = match self {
            // No bytes emitted for NULL; raw bytes for bytea.
            PgParam::Null | PgParam::Bytes(_) => true,
            PgParam::Int(_) | PgParam::Double(_) | PgParam::Bool(_) => matches!(
                *ty,
                Type::INT2 | Type::INT4 | Type::INT8 | Type::OID
                    | Type::FLOAT4 | Type::FLOAT8 | Type::NUMERIC | Type::BOOL
            ),
            PgParam::Text(_) => matches!(
                *ty,
                Type::UUID | Type::INT2 | Type::INT4 | Type::INT8 | Type::OID
                    | Type::FLOAT4 | Type::FLOAT8 | Type::NUMERIC | Type::BOOL
                    | Type::TIMESTAMP | Type::TIMESTAMPTZ | Type::DATE | Type::TIME
                    | Type::JSON | Type::JSONB | Type::BYTEA
            ),
        };
        if binary { Format::Binary } else { Format::Text }
    }

    postgres::types::to_sql_checked!();
}

#[cfg(feature = "postgres_db")]
fn cfml_to_pg_param(val: &CfmlValue) -> PgParam {
    match val {
        CfmlValue::Null => PgParam::Null,
        CfmlValue::Bool(b) => PgParam::Bool(*b),
        CfmlValue::Int(i) => PgParam::Int(*i),
        CfmlValue::Double(d) => PgParam::Double(*d),
        CfmlValue::String(s) => PgParam::Text((**s).clone()),
        CfmlValue::Binary(b) => PgParam::Bytes(b.clone()),
        // A query-column proxy stands in for its first-row scalar (defensive:
        // prepare_pg_statements already flattens these).
        CfmlValue::QueryColumn(..) => cfml_to_pg_param(val.query_column_scalar()),
        _ => PgParam::Text(val.as_string()),
    }
}


// Custom FromSql wrappers for PG types that postgres-types does not provide
// a built-in impl for: TIMETZ (12B: int64 time + int32 zone) and INTERVAL
// (16B: int64 microseconds + int32 days + int32 months).
#[cfg(feature = "postgres_db")]
struct PgTimeTz { time_us: i64, zone_secs: i32 }

#[cfg(feature = "postgres_db")]
impl<'a> postgres::types::FromSql<'a> for PgTimeTz {
    fn from_sql(_ty: &postgres::types::Type, raw: &'a [u8])
        -> Result<Self, Box<dyn std::error::Error + Sync + Send>>
    {
        if raw.len() != 12 { return Err("TIMETZ wire size != 12".into()); }
        let time_us = i64::from_be_bytes(raw[0..8].try_into().unwrap());
        let zone_secs = i32::from_be_bytes(raw[8..12].try_into().unwrap());
        Ok(PgTimeTz { time_us, zone_secs })
    }
    fn accepts(ty: &postgres::types::Type) -> bool { *ty == postgres::types::Type::TIMETZ }
}

#[cfg(feature = "postgres_db")]
struct PgInterval { time_us: i64, days: i32, months: i32 }

#[cfg(feature = "postgres_db")]
impl<'a> postgres::types::FromSql<'a> for PgInterval {
    fn from_sql(_ty: &postgres::types::Type, raw: &'a [u8])
        -> Result<Self, Box<dyn std::error::Error + Sync + Send>>
    {
        if raw.len() != 16 { return Err("INTERVAL wire size != 16".into()); }
        let time_us = i64::from_be_bytes(raw[0..8].try_into().unwrap());
        let days = i32::from_be_bytes(raw[8..12].try_into().unwrap());
        let months = i32::from_be_bytes(raw[12..16].try_into().unwrap());
        Ok(PgInterval { time_us, days, months })
    }
    fn accepts(ty: &postgres::types::Type) -> bool { *ty == postgres::types::Type::INTERVAL }
}

#[cfg(feature = "postgres_db")]
fn format_pg_timetz(t: &PgTimeTz) -> String {
    let mut us = t.time_us;
    let h = us / 3_600_000_000; us %= 3_600_000_000;
    let m = us / 60_000_000;    us %= 60_000_000;
    let s = us / 1_000_000;     let frac = us % 1_000_000;
    // PG stores zone as seconds-west-of-UTC (so +02:00 → -7200). Negate for
    // human display: e.g. zone_secs = -7200 → "+02:00".
    let off_secs = -t.zone_secs;
    let sign = if off_secs >= 0 { '+' } else { '-' };
    let abs = off_secs.unsigned_abs() as i64;
    let oh = abs / 3600;
    let om = (abs % 3600) / 60;
    if frac == 0 {
        format!("1899-12-30 {:02}:{:02}:{:02}{}{:02}:{:02}", h, m, s, sign, oh, om)
    } else {
        format!("1899-12-30 {:02}:{:02}:{:02}.{:06}{}{:02}:{:02}", h, m, s, frac, sign, oh, om)
    }
}

#[cfg(feature = "postgres_db")]
fn format_pg_interval(iv: &PgInterval) -> String {
    // Mirror PG's textual rendering: "1 year 2 mons 3 days HH:MM:SS[.frac]".
    // Lucee/BoxLang pass the driver's PGInterval.toString() through, which
    // produces the same text — match that for downstream string comparisons.
    let years = iv.months / 12;
    let mons  = iv.months % 12;
    let mut parts: Vec<String> = Vec::new();
    if years != 0 { parts.push(format!("{} year{}", years, if years.abs() == 1 { "" } else { "s" })); }
    if mons  != 0 { parts.push(format!("{} mon{}",  mons,  if mons.abs()  == 1 { "" } else { "s" })); }
    if iv.days != 0 { parts.push(format!("{} day{}", iv.days, if iv.days.abs() == 1 { "" } else { "s" })); }
    if iv.time_us != 0 || parts.is_empty() {
        let neg = iv.time_us < 0;
        let mut us = iv.time_us.unsigned_abs();
        let h = us / 3_600_000_000; us %= 3_600_000_000;
        let m = us / 60_000_000;    us %= 60_000_000;
        let s = us / 1_000_000;     let frac = us % 1_000_000;
        let sign = if neg { "-" } else { "" };
        if frac == 0 {
            parts.push(format!("{}{:02}:{:02}:{:02}", sign, h, m, s));
        } else {
            let frac_trim = format!("{:06}", frac);
            let trimmed = frac_trim.trim_end_matches('0');
            parts.push(format!("{}{:02}:{:02}:{:02}.{}", sign, h, m, s, trimmed));
        }
    }
    parts.join(" ")
}

#[cfg(feature = "postgres_db")]
// Public entry: normalize a NULL column to an empty string, matching CFML/Lucee/
// ACF's default "full null support OFF" behavior (a NULL must read back as "" so
// `q.col EQ ""`, `Len(q.col)=0`, and passing q.col positionally to a required
// arg all behave). Mirrors the SQLite (`SqlValue::Null => ""`) and MySQL
// (`mysql::Value::NULL => ""`) adapters. See GH #265. Genuine read-error/type-
// mismatch fallbacks (Err arms) also collapse to "" here — indistinguishable
// from a real NULL under null-support-off, and never panics the worker.
fn postgres_row_to_cfml(row: &postgres::Row, col_idx: usize) -> CfmlValue {
    match postgres_row_to_cfml_typed(row, col_idx) {
        CfmlValue::Null => CfmlValue::string(String::new()),
        v => v,
    }
}

#[cfg(feature = "postgres_db")]
fn postgres_row_to_cfml_typed(row: &postgres::Row, col_idx: usize) -> CfmlValue {
    use postgres::types::Type;
    let col_type = row.columns()[col_idx].type_();

    // try_get never panics: a type mismatch returns Err rather than aborting
    // the worker. Each arm maps the typed value into the closest CFML form
    // (matches Lucee/BoxLang semantics: UUIDs canonical, JSON raw string,
    // numeric precision preserved as String, arrays as native CFML Array).
    fn arr<I: IntoIterator<Item = CfmlValue>>(it: I) -> CfmlValue {
        CfmlValue::array(it.into_iter().collect())
    }

    match *col_type {
        Type::BOOL => match row.try_get::<_, Option<bool>>(col_idx) {
            Ok(Some(b)) => CfmlValue::Bool(b), _ => CfmlValue::Null,
        },
        Type::INT2 => match row.try_get::<_, Option<i16>>(col_idx) {
            Ok(Some(i)) => CfmlValue::Int(i as i64), _ => CfmlValue::Null,
        },
        Type::INT4 => match row.try_get::<_, Option<i32>>(col_idx) {
            Ok(Some(i)) => CfmlValue::Int(i as i64), _ => CfmlValue::Null,
        },
        Type::INT8 => match row.try_get::<_, Option<i64>>(col_idx) {
            Ok(Some(i)) => CfmlValue::Int(i), _ => CfmlValue::Null,
        },
        Type::FLOAT4 => match row.try_get::<_, Option<f32>>(col_idx) {
            Ok(Some(f)) => CfmlValue::Double(f as f64), _ => CfmlValue::Null,
        },
        Type::FLOAT8 => match row.try_get::<_, Option<f64>>(col_idx) {
            Ok(Some(f)) => CfmlValue::Double(f), _ => CfmlValue::Null,
        },
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME =>
            match row.try_get::<_, Option<String>>(col_idx) {
                Ok(Some(s)) => CfmlValue::string(s), _ => CfmlValue::Null,
            },
        Type::BYTEA => match row.try_get::<_, Option<Vec<u8>>>(col_idx) {
            Ok(Some(b)) => CfmlValue::Binary(b), _ => CfmlValue::Null,
        },
        Type::UUID => match row.try_get::<_, Option<uuid::Uuid>>(col_idx) {
            Ok(Some(u)) => CfmlValue::string(u.hyphenated().to_string()),
            _ => CfmlValue::Null,
        },
        Type::TIMESTAMP => match row.try_get::<_, Option<NaiveDateTime>>(col_idx) {
            Ok(Some(d)) => CfmlValue::string(d.format("%Y-%m-%d %H:%M:%S").to_string()),
            _ => CfmlValue::Null,
        },
        // TIMESTAMPTZ stored in UTC; render in local TZ to match Lucee's
        // session-TZ behavior on a host with the same default TZ.
        Type::TIMESTAMPTZ => match row.try_get::<_, Option<chrono::DateTime<Utc>>>(col_idx) {
            Ok(Some(d)) => CfmlValue::string(
                d.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S").to_string()
            ),
            _ => CfmlValue::Null,
        },
        Type::DATE => match row.try_get::<_, Option<NaiveDate>>(col_idx) {
            Ok(Some(d)) => CfmlValue::string(d.format("%Y-%m-%d").to_string()),
            _ => CfmlValue::Null,
        },
        // TIME has no native date portion; CFML idiom is the epoch-style
        // 1899-12-30 prefix so dateFormat/timeFormat work.
        Type::TIME => match row.try_get::<_, Option<NaiveTime>>(col_idx) {
            Ok(Some(t)) => CfmlValue::string(format!("1899-12-30 {}", t.format("%H:%M:%S"))),
            _ => CfmlValue::Null,
        },
        // TIMETZ: postgres-types has no FromSql impl; use our 12-byte parser
        // and emit a CFML datetime with explicit ±HH:MM offset.
        Type::TIMETZ => match row.try_get::<_, Option<PgTimeTz>>(col_idx) {
            Ok(Some(t)) => CfmlValue::string(format_pg_timetz(&t)),
            _ => CfmlValue::Null,
        },
        // INTERVAL: no native CFML timespan; format like PG/Lucee's textual
        // rendering so string comparison stays meaningful.
        Type::INTERVAL => match row.try_get::<_, Option<PgInterval>>(col_idx) {
            Ok(Some(iv)) => CfmlValue::string(format_pg_interval(&iv)),
            _ => CfmlValue::Null,
        },
        Type::JSON | Type::JSONB => match row.try_get::<_, Option<serde_json::Value>>(col_idx) {
            Ok(Some(v)) => CfmlValue::string(v.to_string()),
            _ => CfmlValue::Null,
        },
        // NUMERIC as String preserves precision; CFML has no native BigDecimal.
        Type::NUMERIC => match row.try_get::<_, Option<rust_decimal::Decimal>>(col_idx) {
            Ok(Some(d)) => CfmlValue::string(d.to_string()),
            _ => CfmlValue::Null,
        },
        // Array types: BoxLang-style native CFML Array (better DX than Lucee's
        // java.sql.Array passthrough).
        Type::BOOL_ARRAY => match row.try_get::<_, Option<Vec<Option<bool>>>>(col_idx) {
            Ok(Some(v)) => arr(v.into_iter().map(|x| x.map(CfmlValue::Bool).unwrap_or(CfmlValue::Null))),
            _ => CfmlValue::Null,
        },
        Type::INT2_ARRAY => match row.try_get::<_, Option<Vec<Option<i16>>>>(col_idx) {
            Ok(Some(v)) => arr(v.into_iter().map(|x| x.map(|i| CfmlValue::Int(i as i64)).unwrap_or(CfmlValue::Null))),
            _ => CfmlValue::Null,
        },
        Type::INT4_ARRAY => match row.try_get::<_, Option<Vec<Option<i32>>>>(col_idx) {
            Ok(Some(v)) => arr(v.into_iter().map(|x| x.map(|i| CfmlValue::Int(i as i64)).unwrap_or(CfmlValue::Null))),
            _ => CfmlValue::Null,
        },
        Type::INT8_ARRAY => match row.try_get::<_, Option<Vec<Option<i64>>>>(col_idx) {
            Ok(Some(v)) => arr(v.into_iter().map(|x| x.map(CfmlValue::Int).unwrap_or(CfmlValue::Null))),
            _ => CfmlValue::Null,
        },
        Type::FLOAT4_ARRAY => match row.try_get::<_, Option<Vec<Option<f32>>>>(col_idx) {
            Ok(Some(v)) => arr(v.into_iter().map(|x| x.map(|f| CfmlValue::Double(f as f64)).unwrap_or(CfmlValue::Null))),
            _ => CfmlValue::Null,
        },
        Type::FLOAT8_ARRAY => match row.try_get::<_, Option<Vec<Option<f64>>>>(col_idx) {
            Ok(Some(v)) => arr(v.into_iter().map(|x| x.map(CfmlValue::Double).unwrap_or(CfmlValue::Null))),
            _ => CfmlValue::Null,
        },
        Type::TEXT_ARRAY | Type::VARCHAR_ARRAY | Type::BPCHAR_ARRAY | Type::NAME_ARRAY =>
            match row.try_get::<_, Option<Vec<Option<String>>>>(col_idx) {
                Ok(Some(v)) => arr(v.into_iter().map(|x| x.map(CfmlValue::string).unwrap_or(CfmlValue::Null))),
                _ => CfmlValue::Null,
            },
        Type::UUID_ARRAY => match row.try_get::<_, Option<Vec<Option<uuid::Uuid>>>>(col_idx) {
            Ok(Some(v)) => arr(v.into_iter().map(|x|
                x.map(|u| CfmlValue::string(u.hyphenated().to_string())).unwrap_or(CfmlValue::Null)
            )),
            _ => CfmlValue::Null,
        },
        // Unknown / unsupported type: try a string conversion, fall back to
        // Null on type mismatch. Never panics.
        _ => match row.try_get::<_, Option<String>>(col_idx) {
            Ok(Some(s)) => CfmlValue::string(s),
            _ => CfmlValue::Null,
        }
    }
}

// -----------------------------------------------
// MSSQL driver (tiberius)
// -----------------------------------------------

#[cfg(feature = "mssql_db")]
fn execute_mssql(url: &str, sql: &str, params_arg: &CfmlValue, return_type: &str) -> CfmlResult {
    // Normalize params before touching the pool.
    let (effective_params, _type_hints) = normalize_query_params(params_arg);

    let pool = get_mssql_pool(url)?;

    // The pool does not ping on checkout (Lucee parity / remote-DB perf), so a
    // server-closed idle connection (Azure SQL idle eviction, failover, network
    // drop, serverless scale-to-zero) is handed back out and only surfaces here.
    // When the server drops MANY idle sessions at once the pool can hold several
    // dead connections, and r2d2 establishes their replacements asynchronously —
    // so an immediate retry may draw a *second* stale connection before a fresh
    // one is ready. Retry the (retry-safe) statement, discarding each broken
    // connection so r2d2 evicts it, until a live connection runs it or the
    // attempt budget is spent. The budget is bounded by the pool size so a
    // statement that genuinely keeps failing connection-level can't loop forever.
    let mut last_err: Option<CfmlError> = None;
    for _ in 0..=MSSQL_POOL_MAX_SIZE {
        let mut conn = pool.get()
            .map_err(|e| CfmlError::database(format!("queryExecute: MSSQL connection error: {}", e)))?;
        match run_mssql_on_conn(&mut conn, sql, &effective_params, return_type) {
            Ok(value) => return Ok(value),
            // Connection-level failure on a replayable statement: drop the broken
            // connection (so r2d2 discards it) and try the next pooled connection.
            Err(err) if err.retry_safe => {
                drop(conn);
                last_err = Some(err.error);
            }
            Err(err) => return Err(err.error),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        CfmlError::database("queryExecute: MSSQL connection error: exhausted pool retries".to_string())
    }))
}

/// Run one statement against a pooled MSSQL connection, flagging the connection
/// `broken` (so r2d2 evicts it on return) when the failure is connection-level.
/// Returns the rich `MssqlRunError` so the caller can decide whether to retry.
#[cfg(feature = "mssql_db")]
fn run_mssql_on_conn(
    conn: &mut MssqlConn,
    sql: &str,
    params: &[CfmlValue],
    return_type: &str,
) -> Result<CfmlValue, MssqlRunError> {
    let result = run_mssql_statement(&mut conn.client, sql, params, return_type);
    if let Err(err) = &result {
        if err.connection_broken {
            conn.broken = true;
        }
    }
    result
}

/// A bound MSSQL parameter. We own the value so its `ColumnData` (which borrows
/// for strings/binary) can be handed to tiberius as `&dyn ToSql` across the
/// query await. Replaces the old string-interpolation path: parameters are now
/// sent as real typed bind values (sp_executesql), so values are never spliced
/// into the SQL text — no injection surface, and SQL Server can cache the plan.
#[cfg(feature = "mssql_db")]
enum MssqlParam {
    Null,
    Bool(bool),
    Int(i64),
    Double(f64),
    Str(String),
    Bytes(Vec<u8>),
}

#[cfg(feature = "mssql_db")]
impl tiberius::ToSql for MssqlParam {
    fn to_sql(&self) -> tiberius::ColumnData<'_> {
        use std::borrow::Cow;
        use tiberius::ColumnData;
        match self {
            // Untyped NULL — sp_executesql types it as nvarchar NULL; that
            // inserts/compares as NULL against any column type, matching the
            // previous literal-`NULL` behaviour.
            MssqlParam::Null => ColumnData::String(None),
            MssqlParam::Bool(b) => ColumnData::Bit(Some(*b)),
            MssqlParam::Int(n) => ColumnData::I64(Some(*n)),
            MssqlParam::Double(d) => ColumnData::F64(Some(*d)),
            MssqlParam::Str(s) => ColumnData::String(Some(Cow::Borrowed(s.as_str()))),
            MssqlParam::Bytes(b) => ColumnData::Binary(Some(Cow::Borrowed(b.as_slice()))),
        }
    }
}

/// Map CFML values to typed MSSQL bind parameters. Dates and other non-scalar
/// values fall back to their string form (SQL Server implicitly converts an
/// nvarchar like `'2026-06-15 09:00:00'` to datetime), preserving the semantics
/// the inline path had while now binding rather than splicing.
#[cfg(feature = "mssql_db")]
fn mssql_bind_params(params: &[CfmlValue]) -> Vec<MssqlParam> {
    params.iter().map(|p| match cfqueryparam_unwrap(p) {
        CfmlValue::Null => MssqlParam::Null,
        CfmlValue::Bool(b) => MssqlParam::Bool(b),
        CfmlValue::Int(n) => MssqlParam::Int(n),
        CfmlValue::Double(d) => MssqlParam::Double(d),
        CfmlValue::Binary(b) => MssqlParam::Bytes(b),
        other => MssqlParam::Str(other.as_string()),
    }).collect()
}

/// Rewrite positional `?` placeholders to tiberius/T-SQL `@P1`, `@P2`, … (1-based),
/// skipping `?` inside single-quoted string literals. List-param `?` expansion
/// already happened in `fn_query_execute`, so this is a 1:1 mapping onto the
/// bound parameter vector.
#[cfg(feature = "mssql_db")]
fn mssql_rewrite_placeholders(sql: &str) -> String {
    let mut result = String::with_capacity(sql.len() + 8);
    let mut param_idx = 0usize;
    let bytes = sql.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] == b'?' {
            param_idx += 1;
            result.push_str("@P");
            result.push_str(&param_idx.to_string());
        } else if bytes[i] == b'\'' {
            result.push('\'');
            i += 1;
            while i < len && bytes[i] != b'\'' {
                result.push(bytes[i] as char);
                i += 1;
            }
            if i < len { result.push('\''); }
        } else {
            result.push(bytes[i] as char);
        }
        i += 1;
    }
    result
}

/// A failed MSSQL statement run, carrying enough context for the caller to
/// decide whether the connection should be evicted and whether the statement is
/// safe to retry on a fresh connection. Mirrors the PostgreSQL `PgRunError`.
#[cfg(feature = "mssql_db")]
struct MssqlRunError {
    error: CfmlError,
    connection_broken: bool,
    retry_safe: bool,
}

#[cfg(feature = "mssql_db")]
impl MssqlRunError {
    fn from_cfml(error: CfmlError) -> Self {
        Self { error, connection_broken: false, retry_safe: false }
    }

    fn from_tiberius(ctx: &str, e: tiberius::error::Error, retry_safe: bool) -> Self {
        let connection_broken = is_mssql_connection_error(&e);
        Self {
            error: CfmlError::database(format!("queryExecute: MSSQL {}: {}", ctx, e)),
            connection_broken,
            // Only retry connection-level failures, and only when the caller says
            // the statement is replayable. SELECTs are side-effect free, so they
            // always replay. Mutations are NOT retried: tiberius fuses prepare
            // and execute into one sp_executesql round-trip, so a failure can't be
            // proven to predate any side effect (and a multi-statement batch may
            // be partially applied) — retrying risks double-applying a write.
            retry_safe: connection_broken && retry_safe,
        }
    }
}

/// Run one MSSQL statement (SELECT → rows, otherwise → affected count) against
/// an already-checked-out connection on the shared runtime, binding `params` as
/// real typed parameters. Shared by the pooled (`execute_mssql`) and
/// transaction (`execute_with_transaction`) paths. Connection-level failures are
/// reported via `MssqlRunError::connection_broken` so the caller can evict the
/// connection; SELECT failures are additionally flagged `retry_safe`.
#[cfg(feature = "mssql_db")]
fn run_mssql_statement(
    client: &mut MssqlClient,
    sql: &str,
    params: &[CfmlValue],
    return_type: &str,
) -> Result<CfmlValue, MssqlRunError> {
    let bound = mssql_bind_params(params);
    let rewritten = mssql_rewrite_placeholders(sql);
    let is_select = mssql_returns_rows(sql);

    mssql_runtime().block_on(async move {
        let param_refs: Vec<&dyn tiberius::ToSql> =
            bound.iter().map(|p| p as &dyn tiberius::ToSql).collect();

        if is_select {
            let stream = client.query(rewritten.as_str(), &param_refs).await
                .map_err(|e| MssqlRunError::from_tiberius("query error", e, true))?;
            let result = stream.into_first_result().await
                .map_err(|e| MssqlRunError::from_tiberius("result error", e, true))?;

            let raw_columns: Vec<String> = if let Some(first_row) = result.first() {
                first_row.columns().iter()
                    .map(|c| c.name().to_string())
                    .collect()
            } else {
                vec![]
            };
            let (columns, keep) = dedup_result_columns(raw_columns);

            let mut rows: Vec<ValueMap> = Vec::with_capacity(result.len());
            for row in &result {
                let mut row_map = ValueMap::default();
                for (out_i, &src_i) in keep.iter().enumerate() {
                    let val = mssql_column_to_cfml(row, src_i);
                    row_map.insert(columns[out_i].clone(), val);
                }
                rows.push(row_map);
            }

            build_query_result(columns, rows, sql, return_type).map_err(MssqlRunError::from_cfml)
        } else {
            // execute() returns the real rows-affected total (the previous
            // simple_query path mis-reported INSERT/UPDATE as 0 rows).
            let result = client.execute(rewritten.as_str(), &param_refs).await
                .map_err(|e| MssqlRunError::from_tiberius("error", e, false))?;
            build_mutation_result(result.total() as i64, 0, sql).map_err(MssqlRunError::from_cfml)
        }
    })
}

/// Run a transaction-control batch (BEGIN/COMMIT/ROLLBACK TRANSACTION) on the
/// shared runtime. These MUST go through `simple_query` (a raw batch) rather
/// than `execute`/sp_executesql, which would raise "transaction count after
/// EXECUTE indicates a mismatching number of BEGIN and COMMIT statements".
#[cfg(feature = "mssql_db")]
fn mssql_txn_control(client: &mut MssqlClient, broken: &mut bool, sql: &str, label: &str) -> Result<(), CfmlError> {
    mssql_runtime().block_on(async move {
        let on_err = |broken: &mut bool, e: tiberius::error::Error| {
            if is_mssql_connection_error(&e) {
                *broken = true;
            }
            CfmlError::database(format!("cftransaction: {} error: {}", label, e))
        };
        // Drain the (empty) result stream so the connection is left flushed.
        match client.simple_query(sql).await {
            Ok(s) => match s.into_results().await {
                Ok(_) => Ok(()),
                Err(e) => Err(on_err(broken, e)),
            },
            Err(e) => Err(on_err(broken, e)),
        }
    })
}

#[cfg(feature = "mssql_db")]
// Public entry: normalize a NULL column to an empty string, matching CFML/Lucee/
// ACF's default "full null support OFF" behavior (see GH #264). Mirrors the
// SQLite/MySQL/Postgres adapters. The explicit `ColumnType::Null` arm and every
// per-type `Ok(None)`/`Err` fallback in the typed converter collapse to "" here,
// so `q.col EQ ""`, `Len(q.col)=0`, and positional binding to required args behave.
fn mssql_column_to_cfml(row: &tiberius::Row, col_idx: usize) -> CfmlValue {
    match mssql_column_to_cfml_typed(row, col_idx) {
        CfmlValue::Null => CfmlValue::string(String::new()),
        v => v,
    }
}

#[cfg(feature = "mssql_db")]
fn mssql_column_to_cfml_typed(row: &tiberius::Row, col_idx: usize) -> CfmlValue {
    use tiberius::ColumnType;
    use tiberius::numeric::Numeric;
    let col_type = row.columns()[col_idx].column_type();

    // try_get returns Result<Option<T>>: a type mismatch yields Err rather than
    // panicking the worker. Each arm maps the typed value into the closest CFML
    // form (canonical UUIDs, datetime strings, precision-preserving decimals).
    match col_type {
        ColumnType::Bit | ColumnType::Bitn =>
            match row.try_get::<bool, _>(col_idx) {
                Ok(Some(b)) => CfmlValue::Bool(b), _ => CfmlValue::Null,
            },
        ColumnType::Int1 => match row.try_get::<u8, _>(col_idx) {
            Ok(Some(i)) => CfmlValue::Int(i as i64), _ => CfmlValue::Null,
        },
        ColumnType::Int2 => match row.try_get::<i16, _>(col_idx) {
            Ok(Some(i)) => CfmlValue::Int(i as i64), _ => CfmlValue::Null,
        },
        ColumnType::Int4 => match row.try_get::<i32, _>(col_idx) {
            Ok(Some(i)) => CfmlValue::Int(i as i64), _ => CfmlValue::Null,
        },
        ColumnType::Int8 => match row.try_get::<i64, _>(col_idx) {
            Ok(Some(i)) => CfmlValue::Int(i), _ => CfmlValue::Null,
        },
        // Intn is variable-width — try widest first, then narrower
        ColumnType::Intn => {
            if let Ok(Some(i)) = row.try_get::<i64, _>(col_idx) { return CfmlValue::Int(i); }
            if let Ok(Some(i)) = row.try_get::<i32, _>(col_idx) { return CfmlValue::Int(i as i64); }
            if let Ok(Some(i)) = row.try_get::<i16, _>(col_idx) { return CfmlValue::Int(i as i64); }
            if let Ok(Some(i)) = row.try_get::<u8, _>(col_idx)  { return CfmlValue::Int(i as i64); }
            CfmlValue::Null
        }
        ColumnType::Float4 => match row.try_get::<f32, _>(col_idx) {
            Ok(Some(f)) => CfmlValue::Double(f as f64), _ => CfmlValue::Null,
        },
        ColumnType::Float8 => match row.try_get::<f64, _>(col_idx) {
            Ok(Some(f)) => CfmlValue::Double(f), _ => CfmlValue::Null,
        },
        ColumnType::Floatn => {
            if let Ok(Some(f)) = row.try_get::<f64, _>(col_idx) { return CfmlValue::Double(f); }
            if let Ok(Some(f)) = row.try_get::<f32, _>(col_idx) { return CfmlValue::Double(f as f64); }
            CfmlValue::Null
        }
        // Money is fixed-point currency; Display gives a decimal string.
        ColumnType::Money | ColumnType::Money4 =>
            match row.try_get::<f64, _>(col_idx) {
                Ok(Some(f)) => CfmlValue::Double(f), _ => CfmlValue::Null,
            },
        // GUIDs canonical lowercase, no braces.
        ColumnType::Guid => match row.try_get::<uuid::Uuid, _>(col_idx) {
            Ok(Some(u)) => CfmlValue::string(u.hyphenated().to_string()),
            _ => CfmlValue::Null,
        },
        // Decimal/numeric: tiberius's Numeric Display format preserves precision.
        ColumnType::Decimaln | ColumnType::Numericn =>
            match row.try_get::<Numeric, _>(col_idx) {
                Ok(Some(n)) => CfmlValue::string(n.to_string()),
                _ => CfmlValue::Null,
            },
        // DATETIME / DATETIME2 / smalldatetime → "%Y-%m-%d %H:%M:%S".
        ColumnType::Datetime | ColumnType::Datetime2 | ColumnType::Datetime4 | ColumnType::Datetimen =>
            match row.try_get::<NaiveDateTime, _>(col_idx) {
                Ok(Some(d)) => CfmlValue::string(d.format("%Y-%m-%d %H:%M:%S").to_string()),
                _ => CfmlValue::Null,
            },
        ColumnType::Daten => match row.try_get::<NaiveDate, _>(col_idx) {
            Ok(Some(d)) => CfmlValue::string(d.format("%Y-%m-%d").to_string()),
            _ => CfmlValue::Null,
        },
        ColumnType::Timen => match row.try_get::<NaiveTime, _>(col_idx) {
            Ok(Some(t)) => CfmlValue::string(format!("1899-12-30 {}", t.format("%H:%M:%S"))),
            _ => CfmlValue::Null,
        },
        // DATETIMEOFFSET stored in UTC + offset; render in local TZ to match
        // CFML's session-TZ-style behavior, consistent with the PG TIMESTAMPTZ path.
        ColumnType::DatetimeOffsetn =>
            match row.try_get::<chrono::DateTime<chrono::FixedOffset>, _>(col_idx) {
                Ok(Some(d)) => CfmlValue::string(
                    d.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S").to_string()
                ),
                _ => CfmlValue::Null,
            },
        // Binary types → CFML Binary.
        ColumnType::BigVarBin | ColumnType::BigBinary | ColumnType::Image =>
            match row.try_get::<&[u8], _>(col_idx) {
                Ok(Some(b)) => CfmlValue::Binary(b.to_vec()),
                _ => CfmlValue::Null,
            },
        // String / character types — all map to CFML String.
        ColumnType::BigVarChar | ColumnType::BigChar | ColumnType::NVarchar
        | ColumnType::NChar | ColumnType::Text | ColumnType::NText =>
            match row.try_get::<&str, _>(col_idx) {
                Ok(Some(s)) => CfmlValue::string(s.to_string()),
                _ => CfmlValue::Null,
            },
        // XML uses a separate XmlData wrapper in tiberius; fall through to
        // try_get::<&str> after the wrapper's FromSql impl unwraps it.
        ColumnType::Xml => {
            if let Ok(Some(x)) = row.try_get::<&tiberius::xml::XmlData, _>(col_idx) {
                return CfmlValue::string(x.as_ref().to_string());
            }
            CfmlValue::Null
        }
        ColumnType::Null => CfmlValue::Null,
        // UDT / SSVariant and anything we missed — best-effort string read,
        // safe Null fallback. Never panics.
        _ => match row.try_get::<&str, _>(col_idx) {
            Ok(Some(s)) => CfmlValue::string(s.to_string()),
            _ => CfmlValue::Null,
        },
    }
}

// -----------------------------------------------
// Transaction support (public functions called by VM)
// -----------------------------------------------

/// Enum to hold driver-specific transaction connections
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
enum TransactionConn {
    #[cfg(feature = "sqlite")]
    Sqlite(r2d2::PooledConnection<SqliteConnectionManager>),
    #[cfg(feature = "mysql_db")]
    Mysql(mysql::PooledConn),
    #[cfg(feature = "postgres_db")]
    Postgres(r2d2::PooledConnection<PostgresConnectionManager>),
    #[cfg(feature = "mssql_db")]
    Mssql(r2d2::PooledConnection<MssqlConnectionManager>),
}

// ---- Public wrappers using Box<dyn Any> for VM interop ----

/// Begin a transaction — returns a type-erased connection in a Box<dyn Any>
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
pub fn txn_begin_boxed(datasource: &str) -> Result<Box<dyn std::any::Any>, CfmlError> {
    if crate::db_driver::has_dynamic_datasource(datasource) {
        return Err(crate::db_driver::dynamic_tx_unsupported(datasource));
    }
    let conn = transaction_begin(datasource)?;
    Ok(Box::new(conn))
}

/// Commit a transaction via type-erased connection
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
pub fn txn_commit_boxed(conn: &mut Box<dyn std::any::Any>) -> Result<(), CfmlError> {
    if let Some(tc) = conn.downcast_mut::<TransactionConn>() {
        transaction_commit(tc)
    } else {
        Err(CfmlError::runtime("cftransaction: invalid transaction connection".to_string()))
    }
}

/// Rollback a transaction via type-erased connection
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
pub fn txn_rollback_boxed(conn: &mut Box<dyn std::any::Any>) -> Result<(), CfmlError> {
    if let Some(tc) = conn.downcast_mut::<TransactionConn>() {
        transaction_rollback(tc)
    } else {
        Err(CfmlError::runtime("cftransaction: invalid transaction connection".to_string()))
    }
}

/// Execute a query within a transaction via type-erased connection
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
pub fn txn_execute_boxed(conn: &mut Box<dyn std::any::Any>, sql: &str, params_arg: &CfmlValue, return_type: &str) -> CfmlResult {
    if let Some(tc) = conn.downcast_mut::<TransactionConn>() {
        execute_with_transaction(tc, sql, params_arg, return_type)
    } else {
        Err(CfmlError::runtime("cftransaction: invalid transaction connection".to_string()))
    }
}

/// Begin a transaction — returns a held connection
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn transaction_begin(datasource: &str) -> Result<TransactionConn, CfmlError> {
    match parse_datasource(datasource) {
        #[cfg(feature = "sqlite")]
        DbDriver::Sqlite(path) => {
            let pool = get_sqlite_pool(&path)?;
            let conn = pool.get()
                .map_err(|e| CfmlError::database(format!("cftransaction: SQLite pool error: {}", e)))?;
            conn.execute_batch("BEGIN")
                .map_err(|e| CfmlError::database(format!("cftransaction: BEGIN error: {}", e)))?;
            Ok(TransactionConn::Sqlite(conn))
        }
        #[cfg(feature = "mysql_db")]
        DbDriver::Mysql(url) => {
            let pool = get_mysql_pool(&url)?;
            let mut conn = pool.get_conn()
                .map_err(|e| CfmlError::database(format!("cftransaction: MySQL pool error: {}", e)))?;
            use mysql::prelude::Queryable;
            conn.query_drop("BEGIN")
                .map_err(|e| CfmlError::database(format!("cftransaction: BEGIN error: {}", e)))?;
            Ok(TransactionConn::Mysql(conn))
        }
        #[cfg(feature = "postgres_db")]
        DbDriver::Postgres(url) => {
            let pool = get_postgres_pool(&url)?;
            let mut conn = pool.get()
                .map_err(|e| CfmlError::database(format!("cftransaction: PostgreSQL pool error: {}", e)))?;
            let pg = &mut *conn;
            if let Err(e) = pg.client.simple_query("BEGIN") {
                if e.is_closed() {
                    pg.broken = true;
                }
                return Err(CfmlError::database(format!("cftransaction: BEGIN error: {}", e)));
            }
            Ok(TransactionConn::Postgres(conn))
        }
        #[cfg(feature = "mssql_db")]
        DbDriver::Mssql(url) => {
            let pool = get_mssql_pool(&url)?;
            let mut conn = pool.get()
                .map_err(|e| CfmlError::database(format!("cftransaction: MSSQL pool error: {}", e)))?;
            let m = &mut *conn;
            mssql_txn_control(&mut m.client, &mut m.broken, "BEGIN TRANSACTION", "BEGIN")?;
            Ok(TransactionConn::Mssql(conn))
        }
        #[allow(unreachable_patterns)]
        _ => Err(CfmlError::runtime(format!(
            "cftransaction: unsupported datasource '{}'", datasource
        ))),
    }
}

/// Commit a transaction
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn transaction_commit(conn: &mut TransactionConn) -> Result<(), CfmlError> {
    match conn {
        #[cfg(feature = "sqlite")]
        TransactionConn::Sqlite(c) => {
            c.execute_batch("COMMIT")
                .map_err(|e| CfmlError::database(format!("cftransaction: COMMIT error: {}", e)))
        }
        #[cfg(feature = "mysql_db")]
        TransactionConn::Mysql(c) => {
            use mysql::prelude::Queryable;
            c.query_drop("COMMIT")
                .map_err(|e| CfmlError::database(format!("cftransaction: COMMIT error: {}", e)))
        }
        #[cfg(feature = "postgres_db")]
        TransactionConn::Postgres(c) => {
            let pg = &mut **c;
            match pg.client.simple_query("COMMIT") {
                Ok(_) => Ok(()),
                Err(e) => {
                    if e.is_closed() {
                        pg.broken = true;
                    }
                    Err(CfmlError::database(format!("cftransaction: COMMIT error: {}", e)))
                }
            }
        }
        #[cfg(feature = "mssql_db")]
        TransactionConn::Mssql(c) => {
            let m = &mut **c;
            mssql_txn_control(&mut m.client, &mut m.broken, "COMMIT TRANSACTION", "COMMIT")
        }
        #[allow(unreachable_patterns)]
        _ => Ok(()),
    }
}

/// Rollback a transaction
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn transaction_rollback(conn: &mut TransactionConn) -> Result<(), CfmlError> {
    match conn {
        #[cfg(feature = "sqlite")]
        TransactionConn::Sqlite(c) => {
            c.execute_batch("ROLLBACK")
                .map_err(|e| CfmlError::database(format!("cftransaction: ROLLBACK error: {}", e)))
        }
        #[cfg(feature = "mysql_db")]
        TransactionConn::Mysql(c) => {
            use mysql::prelude::Queryable;
            c.query_drop("ROLLBACK")
                .map_err(|e| CfmlError::database(format!("cftransaction: ROLLBACK error: {}", e)))
        }
        #[cfg(feature = "postgres_db")]
        TransactionConn::Postgres(c) => {
            let pg = &mut **c;
            match pg.client.simple_query("ROLLBACK") {
                Ok(_) => Ok(()),
                Err(e) => {
                    if e.is_closed() {
                        pg.broken = true;
                    }
                    Err(CfmlError::database(format!("cftransaction: ROLLBACK error: {}", e)))
                }
            }
        }
        #[cfg(feature = "mssql_db")]
        TransactionConn::Mssql(c) => {
            let m = &mut **c;
            mssql_txn_control(&mut m.client, &mut m.broken, "ROLLBACK TRANSACTION", "ROLLBACK")
        }
        #[allow(unreachable_patterns)]
        _ => Ok(()),
    }
}

/// Create a savepoint on an existing transaction conn (type-erased)
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
pub fn txn_savepoint_boxed(conn: &mut Box<dyn std::any::Any>, name: &str) -> Result<(), CfmlError> {
    if let Some(tc) = conn.downcast_mut::<TransactionConn>() {
        transaction_savepoint(tc, SavepointOp::Create, name)
    } else {
        Err(CfmlError::runtime("cftransaction: invalid transaction connection".to_string()))
    }
}

/// Release a savepoint on an existing transaction conn (type-erased)
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
pub fn txn_release_savepoint_boxed(conn: &mut Box<dyn std::any::Any>, name: &str) -> Result<(), CfmlError> {
    if let Some(tc) = conn.downcast_mut::<TransactionConn>() {
        transaction_savepoint(tc, SavepointOp::Release, name)
    } else {
        Err(CfmlError::runtime("cftransaction: invalid transaction connection".to_string()))
    }
}

/// Rollback to a savepoint on an existing transaction conn (type-erased)
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
pub fn txn_rollback_to_savepoint_boxed(conn: &mut Box<dyn std::any::Any>, name: &str) -> Result<(), CfmlError> {
    if let Some(tc) = conn.downcast_mut::<TransactionConn>() {
        transaction_savepoint(tc, SavepointOp::RollbackTo, name)
    } else {
        Err(CfmlError::runtime("cftransaction: invalid transaction connection".to_string()))
    }
}

#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
enum SavepointOp {
    Create,
    Release,
    RollbackTo,
}

/// Issue a SAVEPOINT / RELEASE / ROLLBACK TO statement on a transaction conn.
/// SQL dialects differ: ANSI engines use `SAVEPOINT`/`RELEASE SAVEPOINT`/
/// `ROLLBACK TO SAVEPOINT`; SQL Server uses `SAVE TRANSACTION` and
/// `ROLLBACK TRANSACTION <name>` and has no explicit release (a savepoint is
/// simply freed when the enclosing transaction commits), so Release is a no-op.
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn transaction_savepoint(conn: &mut TransactionConn, op: SavepointOp, name: &str) -> Result<(), CfmlError> {
    match conn {
        #[cfg(feature = "sqlite")]
        TransactionConn::Sqlite(c) => {
            let sql = match op {
                SavepointOp::Create => format!("SAVEPOINT {}", name),
                SavepointOp::Release => format!("RELEASE {}", name),
                SavepointOp::RollbackTo => format!("ROLLBACK TO {}", name),
            };
            c.execute_batch(&sql)
                .map_err(|e| CfmlError::database(format!("cftransaction: savepoint error: {}", e)))
        }
        #[cfg(feature = "mysql_db")]
        TransactionConn::Mysql(c) => {
            use mysql::prelude::Queryable;
            let sql = match op {
                SavepointOp::Create => format!("SAVEPOINT {}", name),
                SavepointOp::Release => format!("RELEASE SAVEPOINT {}", name),
                SavepointOp::RollbackTo => format!("ROLLBACK TO SAVEPOINT {}", name),
            };
            c.query_drop(&sql)
                .map_err(|e| CfmlError::database(format!("cftransaction: savepoint error: {}", e)))
        }
        #[cfg(feature = "postgres_db")]
        TransactionConn::Postgres(c) => {
            let sql = match op {
                SavepointOp::Create => format!("SAVEPOINT {}", name),
                SavepointOp::Release => format!("RELEASE SAVEPOINT {}", name),
                SavepointOp::RollbackTo => format!("ROLLBACK TO SAVEPOINT {}", name),
            };
            let pg = &mut **c;
            match pg.client.simple_query(&sql) {
                Ok(_) => Ok(()),
                Err(e) => {
                    if e.is_closed() {
                        pg.broken = true;
                    }
                    Err(CfmlError::database(format!("cftransaction: savepoint error: {}", e)))
                }
            }
        }
        #[cfg(feature = "mssql_db")]
        TransactionConn::Mssql(c) => {
            let m = &mut **c;
            match op {
                SavepointOp::Create => {
                    mssql_txn_control(&mut m.client, &mut m.broken, &format!("SAVE TRANSACTION {}", name), "SAVEPOINT")
                }
                // SQL Server frees savepoints implicitly at commit — nothing to do.
                SavepointOp::Release => Ok(()),
                SavepointOp::RollbackTo => {
                    mssql_txn_control(&mut m.client, &mut m.broken, &format!("ROLLBACK TRANSACTION {}", name), "ROLLBACK TO SAVEPOINT")
                }
            }
        }
        #[allow(unreachable_patterns)]
        _ => Ok(()),
    }
}

/// Execute a query using an existing transaction connection
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn execute_with_transaction(conn: &mut TransactionConn, sql: &str, params_arg: &CfmlValue, return_type: &str) -> CfmlResult {
    // The transaction path receives the raw cfqueryparam param array (the
    // non-transaction `fn_query_execute` does this normalization before the
    // driver sees the params). Without it the `{value, cfsqltype}` structs are
    // stringified into the SQL instead of bound (issue #147).
    let (sql, params_arg) = normalize_positional_params(sql.to_string(), params_arg);
    let sql = sql.as_str();
    let params_arg = &params_arg;
    match conn {
        #[cfg(feature = "sqlite")]
        TransactionConn::Sqlite(c) => {
            execute_sqlite_with_conn(c, sql, params_arg, return_type)
        }
        #[cfg(feature = "mysql_db")]
        TransactionConn::Mysql(c) => {
            execute_mysql_with_conn(c, sql, params_arg, return_type)
        }
        #[cfg(feature = "postgres_db")]
        TransactionConn::Postgres(c) => {
            execute_postgres_with_conn(&mut **c, sql, params_arg, return_type)
        }
        #[cfg(feature = "mssql_db")]
        TransactionConn::Mssql(c) => {
            // Transaction path: do not retry inside a transaction, but still mark
            // a closed connection as broken so the pool discards it on return.
            let (effective_params, _type_hints) = normalize_query_params(params_arg);
            run_mssql_on_conn(&mut **c, sql, &effective_params, return_type)
                .map_err(|err| err.error)
        }
        #[allow(unreachable_patterns)]
        _ => Err(CfmlError::runtime("Transaction: unsupported driver".to_string())),
    }
}

#[cfg(feature = "sqlite")]
fn execute_sqlite_with_conn(conn: &rusqlite::Connection, sql: &str, params_arg: &CfmlValue, return_type: &str) -> CfmlResult {
    use rusqlite::types::Value as SqlValue;

    // Emulate MySQL `@@` system variables SQLite can't parse (see helper).
    let sql_owned = rewrite_mysql_system_vars(sql);
    let sql = sql_owned.as_str();
    let (exec_sql, bound_params) = build_sqlite_params(params_arg, sql)?;

    if is_select_query(sql) {
        let mut stmt = conn.prepare(&exec_sql)
            .map_err(|e| CfmlError::database(format!("queryExecute: SQL error: {}", e)))?;
        let column_count = stmt.column_count();
        let raw_columns: Vec<String> = (0..column_count)
            .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
            .collect();
        let (columns, keep) = dedup_result_columns(raw_columns);
        let rows_result: Result<Vec<ValueMap>, _> = stmt
            .query_map(rusqlite::params_from_iter(bound_params.iter()), |row| {
                let mut row_map = ValueMap::default();
                for (out_i, &src_i) in keep.iter().enumerate() {
                    let val: SqlValue = row.get_unwrap(src_i);
                    row_map.insert(columns[out_i].clone(), sqlite_to_cfml(val));
                }
                Ok(row_map)
            })
            .map_err(|e| CfmlError::database(format!("queryExecute: query error: {}", e)))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CfmlError::database(format!("queryExecute: row error: {}", e)));
        let rows = rows_result?;
        build_query_result(columns, rows, sql, return_type)
    } else {
        let affected = conn.execute(&exec_sql, rusqlite::params_from_iter(bound_params.iter()))
            .map_err(|e| CfmlError::database(format!("queryExecute: SQL error: {}", e)))?;
        let last_id = conn.last_insert_rowid();
        build_mutation_result(affected as i64, last_id, sql)
    }
}

#[cfg(feature = "mysql_db")]
fn execute_mysql_with_conn(conn: &mut mysql::PooledConn, sql: &str, params_arg: &CfmlValue, return_type: &str) -> CfmlResult {
    use mysql::*;
    use mysql::prelude::*;

    // Same camelCase-safe `:name` -> positional `?` rewrite as execute_mysql
    // (see mysql_named_to_positional): the mysql crate's named-param parser
    // truncates camelCase placeholders at the first uppercase char.
    let (final_sql, params): (std::borrow::Cow<str>, Params) = match params_arg {
        CfmlValue::Array(arr) => {
            let vals: Vec<mysql::Value> = arr.iter().map(|v| cfml_to_mysql_value(&v)).collect();
            let p = if vals.is_empty() { Params::Empty } else { Params::Positional(vals) };
            (std::borrow::Cow::Borrowed(sql), p)
        }
        // An empty params struct means "no parameters" (Preside passes `{}` to
        // placeholder-free SQL). Lucee treats empty params as no params.
        CfmlValue::Struct(map) if map.is_empty() => (std::borrow::Cow::Borrowed(sql), Params::Empty),
        CfmlValue::Struct(map) => {
            let (rewritten, vals_cfml) = mysql_named_to_positional(sql, map);
            let vals: Vec<mysql::Value> =
                vals_cfml.iter().map(|v| cfml_to_mysql_value(v)).collect();
            let p = if vals.is_empty() { Params::Empty } else { Params::Positional(vals) };
            (std::borrow::Cow::Owned(rewritten), p)
        }
        _ => (std::borrow::Cow::Borrowed(sql), Params::Empty),
    };
    let sql: &str = &final_sql;

    if mysql_returns_rows(sql) {
        let result: Vec<Row> = conn.exec(sql, &params)
            .map_err(|e| CfmlError::database(format!("queryExecute: MySQL query error: {}", e)))?;
        let (raw_columns, col_types): (Vec<String>, Vec<mysql::consts::ColumnType>) =
            if let Some(first_row) = result.first() {
                let cols = first_row.columns_ref();
                (
                    cols.iter().map(|c| c.name_str().to_string()).collect(),
                    cols.iter().map(|c| c.column_type()).collect(),
                )
            } else {
                (vec![], vec![])
            };
        let (columns, keep) = dedup_result_columns(raw_columns);
        let mut rows: Vec<ValueMap> = Vec::with_capacity(result.len());
        for row in &result {
            let mut row_map = ValueMap::default();
            for (out_i, &src_i) in keep.iter().enumerate() {
                let val: mysql::Value = row.get(src_i).unwrap_or(mysql::Value::NULL);
                row_map.insert(columns[out_i].clone(), mysql_value_to_cfml_typed(val, col_types.get(src_i).copied()));
            }
            rows.push(row_map);
        }
        build_query_result(columns, rows, sql, return_type)
    } else {
        conn.exec_drop(sql, &params)
            .map_err(|e| CfmlError::database(format!("queryExecute: MySQL error: {}", e)))?;
        let affected = conn.affected_rows() as i64;
        let last_id = conn.last_insert_id() as i64;
        build_mutation_result(affected, last_id, sql)
    }
}

#[cfg(feature = "postgres_db")]
fn execute_postgres_with_conn(conn: &mut PgConn, sql: &str, params_arg: &CfmlValue, return_type: &str) -> CfmlResult {
    // Transaction path: do not retry inside a transaction, but still mark a
    // closed connection as broken so the pool discards it on return.
    run_postgres_on_conn(conn, sql, params_arg, return_type)
        .map_err(|err| err.error)
}

// -----------------------------------------------
// Shared result builders
// -----------------------------------------------

/// Lucee collapses a SQL result set's duplicate column names to the FIRST
/// occurrence — a later column whose name matches an earlier one (case-
/// insensitively) is discarded entirely, name AND data (GH #279: `select 'x' as
/// id, 1 as id` yields a single `id` column holding `'x'`). Given the driver's
/// raw positional column names this returns the surviving names paired with the
/// source column position each should read from. With no duplicates the source
/// positions are simply `0..n`, so the row-building loop is unchanged in the
/// common case; only a genuinely duplicated result pays the collapse.
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn dedup_result_columns(raw: Vec<String>) -> (Vec<String>, Vec<usize>) {
    let mut names: Vec<String> = Vec::with_capacity(raw.len());
    let mut keep: Vec<usize> = Vec::with_capacity(raw.len());
    for (i, name) in raw.into_iter().enumerate() {
        if names.iter().any(|k| k.eq_ignore_ascii_case(&name)) {
            continue; // duplicate — the first occurrence already won
        }
        names.push(name);
        keep.push(i);
    }
    (names, keep)
}

#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn build_query_result(columns: Vec<String>, rows: Vec<ValueMap>, sql: &str, return_type: &str) -> CfmlResult {
    if return_type == "array" {
        let arr: Vec<CfmlValue> = rows.into_iter()
            .map(|r| CfmlValue::strukt(r))
            .collect();
        Ok(CfmlValue::array(arr))
    } else if let Some(key) = return_type.strip_prefix("struct:") {
        // returntype="struct" columnkey="<key>" — build an ordered map keyed
        // by the value of that column. Lucee/Adobe behavior: each entry holds
        // the row struct. Schema-introspection patterns (pg_indexes, etc.)
        // rely on this.
        let key_lower = key.to_lowercase();
        let mut out: ValueMap = ValueMap::default();
        for row in rows.into_iter() {
            let key_val = row.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(&key_lower))
                .map(|(_, v)| v.as_string())
                .unwrap_or_default();
            if !key_val.is_empty() {
                out.insert(key_val, CfmlValue::strukt(row));
            }
        }
        Ok(CfmlValue::strukt(out))
    } else if return_type == "struct" {
        // returntype="struct" without columnkey — treat as a single-row map
        // (matches Lucee for single-row results); for multi-row, take the first.
        if let Some(row) = rows.into_iter().next() {
            Ok(CfmlValue::strukt(row))
        } else {
            Ok(CfmlValue::strukt(ValueMap::default()))
        }
    } else {
        Ok(CfmlValue::Query(CfmlQuery::from_parts_sql(
            columns,
            rows,
            Some(sql.to_string()),
        )))
    }
}

/// True when the statement's first keyword is INSERT (the only statement
/// kind that carries a generated key — Lucee omits `generatedKey` for
/// UPDATE/DELETE/DDL, and drivers report a stale id from the previous
/// INSERT for those).
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn sql_is_insert(sql: &str) -> bool {
    sql.trim_start()
        .get(..6)
        .map(|kw| kw.eq_ignore_ascii_case("insert"))
        .unwrap_or(false)
}

/// Metadata struct for a non-SELECT statement. Shape matches what Lucee
/// exposes through cfquery's `result=` attribute / queryExecute's `result`
/// option: {recordCount, cached, sql, executionTime [, generatedKey]} —
/// `generatedKey` only on an INSERT that actually inserted rows.
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn build_mutation_result(affected: i64, last_id: i64, sql: &str) -> CfmlResult {
    let mut result = ValueMap::default();
    result.insert("recordCount".to_string(), CfmlValue::Int(affected));
    result.insert("cached".to_string(), CfmlValue::Bool(false));
    result.insert("sql".to_string(), CfmlValue::string(sql.to_string()));
    result.insert("executionTime".to_string(), CfmlValue::Int(0));
    if sql_is_insert(sql) && affected > 0 && last_id != 0 {
        result.insert("generatedKey".to_string(), CfmlValue::Int(last_id));
    }
    Ok(CfmlValue::strukt(result))
}

// -----------------------------------------------
// HTTP/Tag infrastructure stubs (VM-intercepted)
// -----------------------------------------------

fn fn_cfdbinfo_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("cfdbinfo requires VM intercept".into()))
}

fn fn_cfheader_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__cfheader requires VM intercept".into()))
}

fn fn_cfapplication_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime(
        "__cfapplication requires VM intercept".into(),
    ))
}

fn fn_cfcontent_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__cfcontent requires VM intercept".into()))
}

fn fn_cflocation_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__cflocation requires VM intercept".into()))
}

fn fn_get_http_request_data_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("getHTTPRequestData requires VM intercept".into()))
}

fn fn_cfinvoke_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__cfinvoke requires VM intercept".into()))
}

fn fn_cfsavecontent_start_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__cfsavecontent_start requires VM intercept".into()))
}

fn fn_cfsavecontent_end_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__cfsavecontent_end requires VM intercept".into()))
}

fn fn_cfabort_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__cfabort requires VM intercept".into()))
}

fn fn_cfexit_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__cfexit requires VM intercept".into()))
}

fn fn_cfhtmlhead_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__cfhtmlhead requires VM intercept".into()))
}

fn fn_cfhtmlbody_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__cfhtmlbody requires VM intercept".into()))
}

fn fn_write_text_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__writeText requires VM intercept".into()))
}

/// Collapse whitespace runs in output (cfprocessingdirective suppressWhiteSpace).
/// Matches Lucee "smart" mode: runs containing a newline → single newline,
/// runs without a newline → single space. Preserves content inside <pre>/<code>/<textarea>.
fn fn_cfprocessingdirective_collapse(args: Vec<CfmlValue>) -> CfmlResult {
    let input = args.get(0).map(|v| v.as_string()).unwrap_or_default();
    let mut result = String::with_capacity(input.len());
    let mut in_preserve = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        // Check for <pre>, <code>, <textarea> open/close tags
        if ch == '<' {
            let mut tag_text = String::from('<');
            while let Some(&next) = chars.peek() {
                tag_text.push(next);
                chars.next();
                if next == '>' { break; }
            }
            let tag_lower = tag_text.to_lowercase();
            if tag_lower.starts_with("<pre") || tag_lower.starts_with("<code") || tag_lower.starts_with("<textarea") {
                in_preserve = true;
            } else if tag_lower.starts_with("</pre") || tag_lower.starts_with("</code") || tag_lower.starts_with("</textarea") {
                in_preserve = false;
            }
            result.push_str(&tag_text);
            continue;
        }

        if in_preserve || !ch.is_whitespace() {
            result.push(ch);
        } else {
            // Collapse whitespace run
            let mut has_newline = ch == '\n';
            while let Some(&next) = chars.peek() {
                if !next.is_whitespace() { break; }
                if next == '\n' { has_newline = true; }
                chars.next();
            }
            if has_newline {
                result.push('\n');
            } else {
                result.push(' ');
            }
        }
    }
    Ok(CfmlValue::string(result))
}

fn fn_invoke_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("invoke requires VM intercept".into()))
}

fn fn_cftransaction_start_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__cftransaction_start requires VM intercept".into()))
}

fn fn_cftransaction_commit_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__cftransaction_commit requires VM intercept".into()))
}

fn fn_cftransaction_rollback_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__cftransaction_rollback requires VM intercept".into()))
}

// -----------------------------------------------
// cfdirectory - Full builtin implementation
// -----------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
fn fn_cfdirectory(args: Vec<CfmlValue>) -> CfmlResult {
    use std::fs;
    use std::path::Path;

    let opts = match args.first() {
        Some(CfmlValue::Struct(s)) => s,
        _ => return Err(CfmlError::runtime("cfdirectory requires a struct argument".into())),
    };

    // Case-insensitive key lookup helper
    fn get_ci(s: &CfmlStruct, key: &str) -> Option<CfmlValue> {
        s.get_ci(key)
    }

    let action = get_ci(opts, "action")
        .map(|v| v.as_string().to_lowercase())
        .unwrap_or_else(|| "list".into());

    let directory = get_ci(opts, "directory")
        .map(|v| v.as_string())
        .unwrap_or_default();

    match action.as_str() {
        "list" => {
            let filter = get_ci(opts, "filter")
                .map(|v| v.as_string())
                .unwrap_or_else(|| "*".into());
            let recurse = get_ci(opts, "recurse")
                .map(|v| match v {
                    CfmlValue::Bool(b) => b,
                    CfmlValue::String(s) => {
                        let l = s.to_lowercase();
                        l == "true" || l == "yes"
                    }
                    _ => false,
                })
                .unwrap_or(false);
            // type filter (Lucee: "dir" | "file" | "all", default "all"). Wheels'
            // $folders() relies on type="dir" to list only plugin sub-directories;
            // ignoring it returned stray sibling files as folders, which then made
            // directoryList() fail on a file (ENOTDIR).
            let type_filter = get_ci(opts, "type")
                .map(|v| v.as_string().to_lowercase())
                .unwrap_or_else(|| "all".into());

            let columns = vec![
                "name".to_string(),
                "directory".to_string(),
                "type".to_string(),
                "size".to_string(),
                "datelastmodified".to_string(),
            ];
            let mut rows: Vec<ValueMap> = Vec::new();

            fn matches_glob(name: &str, pattern: &str) -> bool {
                if pattern == "*" {
                    return true;
                }
                if let Some(ext) = pattern.strip_prefix("*.") {
                    name.to_lowercase().ends_with(&format!(".{}", ext.to_lowercase()))
                } else {
                    name.to_lowercase() == pattern.to_lowercase()
                }
            }

            fn list_dir(
                dir: &Path,
                filter: &str,
                type_filter: &str,
                recurse: bool,
                rows: &mut Vec<ValueMap>,
                visited: &mut std::collections::HashSet<std::path::PathBuf>,
            ) -> Result<(), CfmlError> {
                let entries = fs::read_dir(dir).map_err(|e| {
                    CfmlError::runtime(format!("cfdirectory: cannot read directory: {}", e))
                })?;

                for entry in entries {
                    let entry = match entry {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    // Follow symlinks when classifying entries (Lucee parity): a
                    // symlink pointing at a directory must be treated as a directory
                    // so recurse descends into it. fs::metadata traverses links;
                    // DirEntry::metadata does not.
                    let metadata = match fs::metadata(entry.path()) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = metadata.is_dir();

                    let file_type = if is_dir { "Dir" } else { "File" };
                    let size = if is_dir { 0i64 } else { metadata.len() as i64 };
                    let modified = metadata
                        .modified()
                        .ok()
                        .and_then(|t| {
                            let dt: chrono::DateTime<chrono::Local> = t.into();
                            Some(dt.format("%Y-%m-%d %H:%M:%S").to_string())
                        })
                        .unwrap_or_default();

                    let should_include = match type_filter {
                        "dir" => is_dir,
                        "file" => !is_dir && matches_glob(&name, filter),
                        // "all" (default) — original behavior unchanged
                        _ => is_dir || matches_glob(&name, filter),
                    };

                    if should_include {
                        let mut row = ValueMap::default();
                        row.insert("name".into(), CfmlValue::string(name.clone()));
                        row.insert(
                            "directory".into(),
                            CfmlValue::string(dir.to_string_lossy().to_string()),
                        );
                        row.insert("type".into(), CfmlValue::string(file_type));
                        row.insert("size".into(), CfmlValue::Int(size));
                        row.insert("datelastmodified".into(), CfmlValue::string(modified));
                        rows.push(row);
                    }

                    if recurse && is_dir {
                        // Cycle protection: following directory symlinks can form
                        // loops. `visited` holds the canonical paths of the current
                        // ancestor chain only — a symlink pointing back at an
                        // ancestor is skipped, but a symlink to a sibling is still
                        // followed (Lucee lists such targets again).
                        let canon = fs::canonicalize(entry.path())
                            .unwrap_or_else(|_| entry.path());
                        if visited.insert(canon.clone()) {
                            list_dir(&entry.path(), filter, type_filter, recurse, rows, visited)?;
                            visited.remove(&canon);
                        }
                    }
                }
                Ok(())
            }

            let mut visited = std::collections::HashSet::new();
            if let Ok(canon) = fs::canonicalize(&directory) {
                visited.insert(canon);
            }
            list_dir(Path::new(&directory), &filter, &type_filter, recurse, &mut rows, &mut visited)?;

            // Apply the `sort` attribute — "col [asc|desc][, col2 [asc|desc] …]",
            // default ascending — matching Lucee/ACF. Without this the OS
            // enumeration order leaked through: Masa runs its schema migrations
            // via `<cfdirectory sort="name asc">` then a loop over the result, so
            // an unsorted listing runs migrations out of order and a later one
            // references a column an earlier-by-name one adds ("Unknown column
            // 'urltitle'"). `name`/`type`/`datelastmodified`/`directory` sort as
            // text (case-insensitive, Lucee parity); `size` sorts numerically.
            if let Some(sort_spec) = get_ci(opts, "sort").map(|v| v.as_string()) {
                let keys: Vec<(String, bool)> = sort_spec
                    .split(',')
                    .filter_map(|part| {
                        let mut it = part.split_whitespace();
                        let col = it.next()?.to_lowercase();
                        let asc = !it
                            .next()
                            .map(|d| d.eq_ignore_ascii_case("desc"))
                            .unwrap_or(false);
                        Some((col, asc))
                    })
                    .collect();
                if !keys.is_empty() {
                    let cell = |row: &ValueMap, col: &str| -> CfmlValue {
                        row.iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(col))
                            .map(|(_, v)| v.clone())
                            .unwrap_or(CfmlValue::Null)
                    };
                    rows.sort_by(|a, b| {
                        for (col, asc) in &keys {
                            let av = cell(a, col);
                            let bv = cell(b, col);
                            let ord = if col == "size" {
                                av.as_string()
                                    .parse::<i64>()
                                    .unwrap_or(0)
                                    .cmp(&bv.as_string().parse::<i64>().unwrap_or(0))
                            } else {
                                av.as_string()
                                    .to_lowercase()
                                    .cmp(&bv.as_string().to_lowercase())
                            };
                            let ord = if *asc { ord } else { ord.reverse() };
                            if ord != std::cmp::Ordering::Equal {
                                return ord;
                            }
                        }
                        std::cmp::Ordering::Equal
                    });
                }
            }

            Ok(CfmlValue::Query(CfmlQuery::from_parts(columns, rows)))
        }
        "create" => {
            fs::create_dir_all(&directory).map_err(|e| {
                CfmlError::runtime(format!("cfdirectory create failed: {}", e))
            })?;
            Ok(CfmlValue::Null)
        }
        "delete" => {
            fs::remove_dir_all(&directory).map_err(|e| {
                CfmlError::runtime(format!("cfdirectory delete failed: {}", e))
            })?;
            Ok(CfmlValue::Null)
        }
        "rename" => {
            let new_dir = get_ci(opts, "newdirectory")
                .map(|v| v.as_string())
                .unwrap_or_default();
            if new_dir.is_empty() {
                return Err(CfmlError::runtime(
                    "cfdirectory rename requires 'newdirectory' attribute".into(),
                ));
            }
            fs::rename(&directory, &new_dir).map_err(|e| {
                CfmlError::runtime(format!("cfdirectory rename failed: {}", e))
            })?;
            Ok(CfmlValue::Null)
        }
        _ => Err(CfmlError::runtime(format!(
            "cfdirectory: unsupported action '{}'",
            action
        ))),
    }
}

#[cfg(target_arch = "wasm32")]
fn fn_cfdirectory(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("cfdirectory is not supported in wasm".into()))
}

// -----------------------------------------------
// cffile - script-call form (`cffile(action=..., ...)`)
// -----------------------------------------------
// The <cffile> tag lowers to specific file BIFs, but CFML also exposes every
// built-in tag as a script-callable function. This bundles the single
// struct-of-attributes (built by the tag-call-builtin path in the VM, which
// also folds attributeCollection) and dispatches on `action`, returning the
// content for read/readBinary so the VM can deliver it to `variable="..."`.
#[cfg(not(target_arch = "wasm32"))]
fn fn_cffile(args: Vec<CfmlValue>) -> CfmlResult {
    let opts = match args.first() {
        Some(CfmlValue::Struct(s)) => s,
        _ => return Err(CfmlError::runtime("cffile requires a struct argument".into())),
    };
    let get_ci = |key: &str| opts.get_ci(key);
    let str_attr = |key: &str| get_ci(key).map(|v| v.as_string()).unwrap_or_default();
    // CFML booleanness for attribute strings ("yes"/"no"/"true"/"false").
    let bool_attr = |key: &str| match get_ci(key) {
        Some(CfmlValue::Bool(b)) => b,
        Some(v) => {
            let l = v.as_string().to_lowercase();
            l == "true" || l == "yes" || l == "1"
        }
        None => false,
    };

    let action = get_ci("action")
        .map(|v| v.as_string().to_lowercase())
        .unwrap_or_else(|| "read".into());

    match action.as_str() {
        "read" => fn_file_read(vec![CfmlValue::string(str_attr("file"))]),
        "readbinary" => fn_file_read_binary(vec![CfmlValue::string(str_attr("file"))]),
        "write" => {
            // Preserve a binary `output` as raw bytes rather than stringifying
            // it to the "<Binary>" placeholder (see fn_file_write).
            let output = get_ci("output").unwrap_or_else(|| CfmlValue::string(""));
            fn_file_write(vec![CfmlValue::string(str_attr("file")), output])
        }
        "append" => {
            // addNewLine appends a trailing newline; defaults off to match the
            // fileAppend() BIF. The tag form is delivery-equivalent. Binary
            // output is passed through untouched (no newline coercion).
            match get_ci("output") {
                Some(binary @ CfmlValue::Binary(_)) if !bool_attr("addnewline") => {
                    fn_file_append(vec![CfmlValue::string(str_attr("file")), binary])
                }
                _ => {
                    let mut data = str_attr("output");
                    if bool_attr("addnewline") {
                        data.push('\n');
                    }
                    fn_file_append(vec![
                        CfmlValue::string(str_attr("file")),
                        CfmlValue::string(data),
                    ])
                }
            }
        }
        "copy" => fn_file_copy(vec![
            CfmlValue::string(str_attr("source")),
            CfmlValue::string(str_attr("destination")),
        ]),
        "move" | "rename" => fn_file_move(vec![
            CfmlValue::string(str_attr("source")),
            CfmlValue::string(str_attr("destination")),
        ]),
        "delete" => fn_file_delete(vec![CfmlValue::string(str_attr("file"))]),
        "upload" | "uploadall" => fn_cffile_upload(args),
        _ => Err(CfmlError::runtime(format!(
            "cffile action='{}' is not implemented.",
            action
        ))),
    }
}

#[cfg(target_arch = "wasm32")]
fn fn_cffile(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("cffile is not supported in wasm".into()))
}

// ==== ENCODING HELPERS ====

pub(crate) fn base64_encode_bytes(data: &[u8]) -> String {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(alphabet[((n >> 18) & 63) as usize] as char);
        result.push(alphabet[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            result.push(alphabet[((n >> 6) & 63) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(alphabet[(n & 63) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

pub(crate) fn base64_decode_bytes(s: &str) -> Vec<u8> {
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bytes = Vec::new();
    let chars: Vec<u8> = s.bytes().filter(|&b| b != b'\n' && b != b'\r' && b != b' ').collect();
    let mut i = 0;
    while i < chars.len() {
        if i + 1 >= chars.len() { break; }
        let b0 = table.iter().position(|&c| c == chars[i]).unwrap_or(0) as u32;
        let b1 = table.iter().position(|&c| c == chars[i + 1]).unwrap_or(0) as u32;
        let b2 = if i + 2 < chars.len() && chars[i + 2] != b'=' {
            table.iter().position(|&c| c == chars[i + 2]).unwrap_or(0) as u32
        } else {
            0
        };
        let b3 = if i + 3 < chars.len() && chars[i + 3] != b'=' {
            table.iter().position(|&c| c == chars[i + 3]).unwrap_or(0) as u32
        } else {
            0
        };
        let triple = (b0 << 18) | (b1 << 12) | (b2 << 6) | b3;
        bytes.push(((triple >> 16) & 0xFF) as u8);
        if i + 2 < chars.len() && chars[i + 2] != b'=' {
            bytes.push(((triple >> 8) & 0xFF) as u8);
        }
        if i + 3 < chars.len() && chars[i + 3] != b'=' {
            bytes.push((triple & 0xFF) as u8);
        }
        i += 4;
    }
    bytes
}

fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02X}", b)).collect()
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return Err("Invalid hex string length".to_string());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| format!("Invalid hex: {}", e)))
        .collect()
}

fn uu_encode(data: &[u8]) -> String {
    let mut result = String::new();
    for chunk in data.chunks(45) {
        result.push((chunk.len() as u8 + 32) as char);
        for triple in chunk.chunks(3) {
            let b0 = triple[0] as u32;
            let b1 = triple.get(1).copied().unwrap_or(0) as u32;
            let b2 = triple.get(2).copied().unwrap_or(0) as u32;
            let n = (b0 << 16) | (b1 << 8) | b2;
            result.push((((n >> 18) & 63) as u8).wrapping_add(32) as char);
            result.push((((n >> 12) & 63) as u8).wrapping_add(32) as char);
            result.push((((n >> 6) & 63) as u8).wrapping_add(32) as char);
            result.push(((n & 63) as u8).wrapping_add(32) as char);
        }
        result.push('\n');
    }
    result
}

fn uu_decode(s: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    for line in s.lines() {
        if line.is_empty() { continue; }
        let line_bytes: Vec<u8> = line.bytes().collect();
        if line_bytes.is_empty() { continue; }
        let expected_len = (line_bytes[0].wrapping_sub(32) & 63) as usize;
        let mut i = 1;
        let mut decoded_in_line = Vec::new();
        while i + 3 < line_bytes.len() {
            let b0 = (line_bytes[i].wrapping_sub(32) & 63) as u32;
            let b1 = (line_bytes[i + 1].wrapping_sub(32) & 63) as u32;
            let b2 = (line_bytes[i + 2].wrapping_sub(32) & 63) as u32;
            let b3 = (line_bytes[i + 3].wrapping_sub(32) & 63) as u32;
            let n = (b0 << 18) | (b1 << 12) | (b2 << 6) | b3;
            decoded_in_line.push(((n >> 16) & 0xFF) as u8);
            decoded_in_line.push(((n >> 8) & 0xFF) as u8);
            decoded_in_line.push((n & 0xFF) as u8);
            i += 4;
        }
        decoded_in_line.truncate(expected_len);
        bytes.extend_from_slice(&decoded_in_line);
    }
    bytes
}

// ==== CIPHER HELPERS ====

fn cfmx_compat_encrypt(data: &[u8], key: &str) -> Vec<u8> {
    // CFMX_COMPAT is a simple XOR cipher with a key-derived seed
    let seed: u32 = key.bytes().fold(0u32, |acc, b| acc.wrapping_add(b as u32));
    let mut rng = seed;
    data.iter().map(|&b| {
        rng = rng.wrapping_mul(214013).wrapping_add(2531011);
        let keystream = ((rng >> 16) & 0xFF) as u8;
        b ^ keystream
    }).collect()
}

fn cfmx_compat_decrypt(data: &[u8], key: &str) -> Vec<u8> {
    // XOR is symmetric, so decrypt = encrypt
    cfmx_compat_encrypt(data, key)
}

fn aes_cbc_encrypt(plaintext: &[u8], key: &[u8]) -> Result<Vec<u8>, String> {
    use aes::Aes128;
    use aes::Aes192;
    use aes::Aes256;
    use cbc::Encryptor;
    use cbc::cipher::BlockEncryptMut;
    use cbc::cipher::KeyIvInit;

    let iv = vec![0u8; 16]; // Zero IV (CFML default)

    match key.len() {
        16 => {
            let encryptor = Encryptor::<Aes128>::new_from_slices(key, &iv)
                .map_err(|e| format!("AES-128 init error: {}", e))?;
            Ok(encryptor.encrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(plaintext))
        }
        24 => {
            let encryptor = Encryptor::<Aes192>::new_from_slices(key, &iv)
                .map_err(|e| format!("AES-192 init error: {}", e))?;
            Ok(encryptor.encrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(plaintext))
        }
        32 => {
            let encryptor = Encryptor::<Aes256>::new_from_slices(key, &iv)
                .map_err(|e| format!("AES-256 init error: {}", e))?;
            Ok(encryptor.encrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(plaintext))
        }
        _ => Err(format!("Invalid AES key length: {} bytes (expected 16, 24, or 32)", key.len()))
    }
}

fn aes_cbc_decrypt(ciphertext: &[u8], key: &[u8]) -> Result<Vec<u8>, String> {
    use aes::Aes128;
    use aes::Aes192;
    use aes::Aes256;
    use cbc::Decryptor;
    use cbc::cipher::BlockDecryptMut;
    use cbc::cipher::KeyIvInit;

    let iv = vec![0u8; 16];

    match key.len() {
        16 => {
            let decryptor = Decryptor::<Aes128>::new_from_slices(key, &iv)
                .map_err(|e| format!("AES-128 init error: {}", e))?;
            decryptor.decrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(ciphertext)
                .map_err(|e| format!("AES decryption error: {}", e))
        }
        24 => {
            let decryptor = Decryptor::<Aes192>::new_from_slices(key, &iv)
                .map_err(|e| format!("AES-192 init error: {}", e))?;
            decryptor.decrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(ciphertext)
                .map_err(|e| format!("AES decryption error: {}", e))
        }
        32 => {
            let decryptor = Decryptor::<Aes256>::new_from_slices(key, &iv)
                .map_err(|e| format!("AES-256 init error: {}", e))?;
            decryptor.decrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(ciphertext)
                .map_err(|e| format!("AES decryption error: {}", e))
        }
        _ => Err(format!("Invalid AES key length: {} bytes", key.len()))
    }
}

fn des_cbc_encrypt(plaintext: &[u8], key: &[u8]) -> Result<Vec<u8>, String> {
    use des::Des;
    use cbc::Encryptor;
    use cbc::cipher::BlockEncryptMut;
    use cbc::cipher::KeyIvInit;

    if key.len() != 8 {
        return Err(format!("DES key must be 8 bytes, got {}", key.len()));
    }
    let iv = [0u8; 8];
    let encryptor = Encryptor::<Des>::new_from_slices(key, &iv)
        .map_err(|e| format!("DES init error: {}", e))?;
    Ok(encryptor.encrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(plaintext))
}

fn des_cbc_decrypt(ciphertext: &[u8], key: &[u8]) -> Result<Vec<u8>, String> {
    use des::Des;
    use cbc::Decryptor;
    use cbc::cipher::BlockDecryptMut;
    use cbc::cipher::KeyIvInit;

    if key.len() != 8 {
        return Err(format!("DES key must be 8 bytes, got {}", key.len()));
    }
    let iv = [0u8; 8];
    let decryptor = Decryptor::<Des>::new_from_slices(key, &iv)
        .map_err(|e| format!("DES init error: {}", e))?;
    decryptor.decrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(ciphertext)
        .map_err(|e| format!("DES decryption error: {}", e))
}

fn desede_cbc_encrypt(plaintext: &[u8], key: &[u8]) -> Result<Vec<u8>, String> {
    use des::TdesEde3;
    use cbc::Encryptor;
    use cbc::cipher::BlockEncryptMut;
    use cbc::cipher::KeyIvInit;

    if key.len() != 24 {
        return Err(format!("DESEDE key must be 24 bytes, got {}", key.len()));
    }
    let iv = [0u8; 8];
    let encryptor = Encryptor::<TdesEde3>::new_from_slices(key, &iv)
        .map_err(|e| format!("DESEDE init error: {}", e))?;
    Ok(encryptor.encrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(plaintext))
}

fn desede_cbc_decrypt(ciphertext: &[u8], key: &[u8]) -> Result<Vec<u8>, String> {
    use des::TdesEde3;
    use cbc::Decryptor;
    use cbc::cipher::BlockDecryptMut;
    use cbc::cipher::KeyIvInit;

    if key.len() != 24 {
        return Err(format!("DESEDE key must be 24 bytes, got {}", key.len()));
    }
    let iv = [0u8; 8];
    let decryptor = Decryptor::<TdesEde3>::new_from_slices(key, &iv)
        .map_err(|e| format!("DESEDE init error: {}", e))?;
    decryptor.decrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(ciphertext)
        .map_err(|e| format!("DESEDE decryption error: {}", e))
}

fn blowfish_cbc_encrypt(plaintext: &[u8], key: &[u8]) -> Result<Vec<u8>, String> {
    use blowfish::Blowfish;
    use cbc::Encryptor;
    use cbc::cipher::BlockEncryptMut;
    use cbc::cipher::KeyIvInit;

    let iv = [0u8; 8];
    let encryptor = Encryptor::<Blowfish>::new_from_slices(key, &iv)
        .map_err(|e| format!("Blowfish init error: {}", e))?;
    Ok(encryptor.encrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(plaintext))
}

fn blowfish_cbc_decrypt(ciphertext: &[u8], key: &[u8]) -> Result<Vec<u8>, String> {
    use blowfish::Blowfish;
    use cbc::Decryptor;
    use cbc::cipher::BlockDecryptMut;
    use cbc::cipher::KeyIvInit;

    let iv = [0u8; 8];
    let decryptor = Decryptor::<Blowfish>::new_from_slices(key, &iv)
        .map_err(|e| format!("Blowfish init error: {}", e))?;
    decryptor.decrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(ciphertext)
        .map_err(|e| format!("Blowfish decryption error: {}", e))
}

// ==== SECURITY BUILTIN FUNCTIONS ====

fn fn_hmac(args: Vec<CfmlValue>) -> CfmlResult {
    use hmac::{Hmac, Mac};
    use sha2::{Sha256, Sha384, Sha512};
    use sha1::Sha1;
    use md5::Md5;

    let message = get_str(&args, 0);
    let key = get_str(&args, 1);
    let algorithm = if args.len() >= 3 {
        get_str(&args, 2).to_uppercase()
    } else {
        "HMACSHA256".to_string()
    };
    // encoding param (4th) -- we always return hex, matching CFML default

    let hex_result = match algorithm.as_str() {
        "HMACMD5" | "HMAC-MD5" => {
            let mut mac = Hmac::<Md5>::new_from_slice(key.as_bytes())
                .map_err(|e| CfmlError::runtime(format!("HMAC init error: {}", e)))?;
            mac.update(message.as_bytes());
            hex_encode(&mac.finalize().into_bytes())
        }
        "HMACSHA1" | "HMAC-SHA1" => {
            let mut mac = Hmac::<Sha1>::new_from_slice(key.as_bytes())
                .map_err(|e| CfmlError::runtime(format!("HMAC init error: {}", e)))?;
            mac.update(message.as_bytes());
            hex_encode(&mac.finalize().into_bytes())
        }
        "HMACSHA256" | "HMAC-SHA256" | "" => {
            let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes())
                .map_err(|e| CfmlError::runtime(format!("HMAC init error: {}", e)))?;
            mac.update(message.as_bytes());
            hex_encode(&mac.finalize().into_bytes())
        }
        "HMACSHA384" | "HMAC-SHA384" => {
            let mut mac = Hmac::<Sha384>::new_from_slice(key.as_bytes())
                .map_err(|e| CfmlError::runtime(format!("HMAC init error: {}", e)))?;
            mac.update(message.as_bytes());
            hex_encode(&mac.finalize().into_bytes())
        }
        "HMACSHA512" | "HMAC-SHA512" => {
            let mut mac = Hmac::<Sha512>::new_from_slice(key.as_bytes())
                .map_err(|e| CfmlError::runtime(format!("HMAC init error: {}", e)))?;
            mac.update(message.as_bytes());
            hex_encode(&mac.finalize().into_bytes())
        }
        _ => return Err(CfmlError::runtime(format!("Unsupported HMAC algorithm: {}", algorithm)))
    };

    Ok(CfmlValue::string(hex_result))
}

// ─────────────────────────────────────────────────────────────────────────────
// JWT (JSON Web Token) — JwtSign / JwtVerify / JwtDecode (Lucee crypto-extension
// names). HMAC algorithms only (HS256/HS384/HS512); RSA/ECDSA need asymmetric
// keys and are rejected with a clear error. Tokens are standard RFC 7519 JWS.
// ─────────────────────────────────────────────────────────────────────────────

/// Base64url-encode (RFC 4648 §5, no padding) — the JWT segment encoding.
fn base64url_encode(data: &[u8]) -> String {
    base64_encode_bytes(data)
        .replace('+', "-")
        .replace('/', "_")
        .trim_end_matches('=')
        .to_string()
}

/// Base64url-decode (tolerates missing padding).
fn base64url_decode(s: &str) -> Vec<u8> {
    let mut t = s.replace('-', "+").replace('_', "/");
    while t.len() % 4 != 0 {
        t.push('=');
    }
    base64_decode_bytes(&t)
}

/// Normalise a JWT `alg` to its canonical upper-case form.
fn jwt_alg_canonical(a: &str) -> String {
    a.trim().to_uppercase()
}

/// Coerce a numeric JWT claim (exp/nbf/iat) — may be Int, Double, or String.
fn jwt_claim_seconds(v: &CfmlValue) -> Option<i64> {
    match v {
        CfmlValue::Int(i) => Some(*i),
        CfmlValue::Double(d) => Some(*d as i64),
        CfmlValue::String(s) => s.trim().parse::<f64>().ok().map(|f| f as i64),
        _ => None,
    }
}

/// HMAC-sign the JWT signing input, returning the raw signature bytes.
fn jwt_hmac_sign(alg: &str, key: &str, msg: &str) -> Result<Vec<u8>, CfmlError> {
    use hmac::{Hmac, Mac};
    use sha2::{Sha256, Sha384, Sha512};
    let keyerr = |e: hmac::digest::InvalidLength| CfmlError::runtime(format!("JWT key error: {}", e));
    Ok(match alg {
        "HS256" => {
            let mut m = Hmac::<Sha256>::new_from_slice(key.as_bytes()).map_err(keyerr)?;
            m.update(msg.as_bytes());
            m.finalize().into_bytes().to_vec()
        }
        "HS384" => {
            let mut m = Hmac::<Sha384>::new_from_slice(key.as_bytes()).map_err(keyerr)?;
            m.update(msg.as_bytes());
            m.finalize().into_bytes().to_vec()
        }
        "HS512" => {
            let mut m = Hmac::<Sha512>::new_from_slice(key.as_bytes()).map_err(keyerr)?;
            m.update(msg.as_bytes());
            m.finalize().into_bytes().to_vec()
        }
        _ => return Err(CfmlError::runtime(format!(
            "JWT algorithm '{}' is not supported. RustCFML supports HS256/HS384/HS512; RSA (RS*/PS*) and ECDSA (ES*) require asymmetric keys not yet implemented.",
            alg
        ))),
    })
}

/// Constant-time verify of an HMAC JWT signature.
fn jwt_hmac_verify(alg: &str, key: &str, msg: &str, sig: &[u8]) -> Result<bool, CfmlError> {
    use hmac::{Hmac, Mac};
    use sha2::{Sha256, Sha384, Sha512};
    let keyerr = |e: hmac::digest::InvalidLength| CfmlError::runtime(format!("JWT key error: {}", e));
    Ok(match alg {
        "HS256" => {
            let mut m = Hmac::<Sha256>::new_from_slice(key.as_bytes()).map_err(keyerr)?;
            m.update(msg.as_bytes());
            m.verify_slice(sig).is_ok()
        }
        "HS384" => {
            let mut m = Hmac::<Sha384>::new_from_slice(key.as_bytes()).map_err(keyerr)?;
            m.update(msg.as_bytes());
            m.verify_slice(sig).is_ok()
        }
        "HS512" => {
            let mut m = Hmac::<Sha512>::new_from_slice(key.as_bytes()).map_err(keyerr)?;
            m.update(msg.as_bytes());
            m.verify_slice(sig).is_ok()
        }
        _ => return Err(CfmlError::runtime(format!(
            "JWT algorithm '{}' is not supported (HS256/HS384/HS512 only).",
            alg
        ))),
    })
}

/// `JwtSign(payload, key [, algorithm="HS256" [, expiresIn]])` → signed JWT string.
/// payload: a struct of claims (or a raw string). key: the HMAC secret. expiresIn:
/// optional lifetime in seconds — when given, `iat` (now) and `exp` (now+expiresIn)
/// are added to a struct payload if absent.
fn fn_jwt_sign(args: Vec<CfmlValue>) -> CfmlResult {
    let payload = args.first().cloned().unwrap_or(CfmlValue::Null);
    let key = get_str(&args, 1);
    let alg = {
        let a = get_str(&args, 2);
        if a.is_empty() { "HS256".to_string() } else { jwt_alg_canonical(&a) }
    };
    let expires_in: Option<i64> = args.get(3).and_then(jwt_claim_seconds);

    let header_json = format!("{{\"alg\":\"{}\",\"typ\":\"JWT\"}}", alg);
    let header_b64 = base64url_encode(header_json.as_bytes());

    let payload_json = match &payload {
        CfmlValue::Struct(s) => {
            let mut map = s.snapshot();
            if let Some(exp_secs) = expires_in {
                let now = chrono::Utc::now().timestamp();
                if !map.keys().any(|k| k.eq_ignore_ascii_case("iat")) {
                    map.insert("iat".to_string(), CfmlValue::Int(now));
                }
                if !map.keys().any(|k| k.eq_ignore_ascii_case("exp")) {
                    map.insert("exp".to_string(), CfmlValue::Int(now + exp_secs));
                }
            }
            let mut visited = Vec::new();
            serialize_value(&CfmlValue::strukt(map), &mut visited, false)
        }
        CfmlValue::String(s) => (**s).clone(),
        CfmlValue::Null => "{}".to_string(),
        other => {
            let mut visited = Vec::new();
            serialize_value(other, &mut visited, false)
        }
    };
    let payload_b64 = base64url_encode(payload_json.as_bytes());

    let signing_input = format!("{}.{}", header_b64, payload_b64);
    let sig = jwt_hmac_sign(&alg, &key, &signing_input)?;
    let sig_b64 = base64url_encode(&sig);
    Ok(CfmlValue::string(format!("{}.{}", signing_input, sig_b64)))
}

/// `JwtVerify(token, key [, algorithm])` → the claims struct. Throws on a bad
/// signature, an algorithm mismatch, or an expired / not-yet-valid token.
fn fn_jwt_verify(args: Vec<CfmlValue>) -> CfmlResult {
    let token = get_str(&args, 0);
    let key = get_str(&args, 1);
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(CfmlError::runtime(
            "JwtVerify: invalid JWT format (expected 3 dot-separated parts).".to_string(),
        ));
    }

    // Read the alg from the header and (defensively) reject an algorithm-substitution
    // attempt when the caller pinned an expected algorithm.
    let header_json = String::from_utf8_lossy(&base64url_decode(parts[0])).to_string();
    let header = fn_deserialize_json(vec![CfmlValue::string(header_json)])?;
    let alg_in_token = match &header {
        CfmlValue::Struct(h) => h
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("alg"))
            .map(|(_, v)| v.as_string())
            .unwrap_or_default(),
        _ => String::new(),
    };
    let expected = get_str(&args, 2);
    if !expected.is_empty() && !jwt_alg_canonical(&expected).eq_ignore_ascii_case(&alg_in_token) {
        return Err(CfmlError::runtime(format!(
            "JwtVerify: token algorithm '{}' does not match expected '{}'.",
            alg_in_token, expected
        )));
    }
    let alg = jwt_alg_canonical(&alg_in_token);

    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let sig_bytes = base64url_decode(parts[2]);
    if !jwt_hmac_verify(&alg, &key, &signing_input, &sig_bytes)? {
        return Err(CfmlError::runtime(
            "JwtVerify: signature verification failed.".to_string(),
        ));
    }

    let payload_json = String::from_utf8_lossy(&base64url_decode(parts[1])).to_string();
    let claims = fn_deserialize_json(vec![CfmlValue::string(payload_json)])?;
    if let CfmlValue::Struct(c) = &claims {
        let now = chrono::Utc::now().timestamp();
        if let Some(exp) = c.iter().find(|(k, _)| k.eq_ignore_ascii_case("exp")).and_then(|(_, v)| jwt_claim_seconds(&v)) {
            if exp < now {
                return Err(CfmlError::runtime("JwtVerify: token has expired.".to_string()));
            }
        }
        if let Some(nbf) = c.iter().find(|(k, _)| k.eq_ignore_ascii_case("nbf")).and_then(|(_, v)| jwt_claim_seconds(&v)) {
            if nbf > now {
                return Err(CfmlError::runtime("JwtVerify: token is not yet valid (nbf).".to_string()));
            }
        }
    }
    Ok(claims)
}

/// `JwtDecode(token)` → the claims struct WITHOUT verifying the signature.
/// For inspection only — never trust decoded claims without JwtVerify.
fn fn_jwt_decode(args: Vec<CfmlValue>) -> CfmlResult {
    let token = get_str(&args, 0);
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return Err(CfmlError::runtime(
            "JwtDecode: invalid JWT format (expected at least 2 dot-separated parts).".to_string(),
        ));
    }
    let payload_json = String::from_utf8_lossy(&base64url_decode(parts[1])).to_string();
    fn_deserialize_json(vec![CfmlValue::string(payload_json)])
}

#[cfg(feature = "security")]
fn fn_generate_secret_key(args: Vec<CfmlValue>) -> CfmlResult {
    use rand::RngCore;

    let algorithm = if args.is_empty() {
        "AES".to_string()
    } else {
        get_str(&args, 0).to_uppercase()
    };
    let key_size = if args.len() >= 2 { get_int(&args, 1) as usize } else { 0 };

    let num_bytes = match algorithm.as_str() {
        "AES" => {
            let bits = if key_size > 0 { key_size } else { 128 };
            match bits {
                128 | 192 | 256 => bits / 8,
                _ => return Err(CfmlError::runtime(format!("Invalid AES key size: {}. Must be 128, 192, or 256", bits)))
            }
        }
        "DES" => 8,
        "DESEDE" | "DESEDE3" => 24,
        "BLOWFISH" => {
            let bits = if key_size > 0 { key_size } else { 128 };
            bits / 8
        }
        _ => return Err(CfmlError::runtime(format!("Unsupported algorithm: {}", algorithm)))
    };

    let mut key_bytes = vec![0u8; num_bytes];
    rand::thread_rng().fill_bytes(&mut key_bytes);
    Ok(CfmlValue::string(base64_encode_bytes(&key_bytes)))
}

fn fn_encrypt(args: Vec<CfmlValue>) -> CfmlResult {
    let plaintext = get_str(&args, 0);
    let key_b64 = get_str(&args, 1);
    let algorithm = if args.len() >= 3 {
        get_str(&args, 2).to_uppercase()
    } else {
        "AES".to_string()
    };
    let encoding = if args.len() >= 4 {
        get_str(&args, 3).to_uppercase()
    } else {
        "UU".to_string()
    };

    let ciphertext = if algorithm == "CFMX_COMPAT" {
        cfmx_compat_encrypt(plaintext.as_bytes(), &key_b64)
    } else {
        let key_bytes = base64_decode_bytes(&key_b64);
        let plaintext_bytes = plaintext.as_bytes();

        match algorithm.as_str() {
            "AES" | "AES/CBC/PKCS5PADDING" | "AES/CBC/PKCS7PADDING" => {
                aes_cbc_encrypt(plaintext_bytes, &key_bytes)
                    .map_err(|e| CfmlError::runtime(e))?
            }
            "DES" | "DES/CBC/PKCS5PADDING" => {
                des_cbc_encrypt(plaintext_bytes, &key_bytes)
                    .map_err(|e| CfmlError::runtime(e))?
            }
            "DESEDE" | "DESEDE/CBC/PKCS5PADDING" => {
                desede_cbc_encrypt(plaintext_bytes, &key_bytes)
                    .map_err(|e| CfmlError::runtime(e))?
            }
            "BLOWFISH" | "BLOWFISH/CBC/PKCS5PADDING" => {
                blowfish_cbc_encrypt(plaintext_bytes, &key_bytes)
                    .map_err(|e| CfmlError::runtime(e))?
            }
            _ => return Err(CfmlError::runtime(format!("Unsupported encryption algorithm: {}", algorithm)))
        }
    };

    let encoded = match encoding.as_str() {
        "UU" => uu_encode(&ciphertext),
        "BASE64" => base64_encode_bytes(&ciphertext),
        "HEX" => hex_encode(&ciphertext),
        _ => return Err(CfmlError::runtime(format!("Unsupported encoding: {}", encoding)))
    };

    Ok(CfmlValue::string(encoded))
}

fn fn_decrypt(args: Vec<CfmlValue>) -> CfmlResult {
    let encoded_str = get_str(&args, 0);
    let key_b64 = get_str(&args, 1);
    let algorithm = if args.len() >= 3 {
        get_str(&args, 2).to_uppercase()
    } else {
        "AES".to_string()
    };
    let encoding = if args.len() >= 4 {
        get_str(&args, 3).to_uppercase()
    } else {
        "UU".to_string()
    };

    let ciphertext = match encoding.as_str() {
        "UU" => uu_decode(&encoded_str),
        "BASE64" => base64_decode_bytes(&encoded_str),
        "HEX" => hex_decode(&encoded_str).map_err(|e| CfmlError::runtime(e))?,
        _ => return Err(CfmlError::runtime(format!("Unsupported encoding: {}", encoding)))
    };

    let plaintext_bytes = if algorithm == "CFMX_COMPAT" {
        cfmx_compat_decrypt(&ciphertext, &key_b64)
    } else {
        let key_bytes = base64_decode_bytes(&key_b64);

        match algorithm.as_str() {
            "AES" | "AES/CBC/PKCS5PADDING" | "AES/CBC/PKCS7PADDING" => {
                aes_cbc_decrypt(&ciphertext, &key_bytes)
                    .map_err(|e| CfmlError::runtime(e))?
            }
            "DES" | "DES/CBC/PKCS5PADDING" => {
                des_cbc_decrypt(&ciphertext, &key_bytes)
                    .map_err(|e| CfmlError::runtime(e))?
            }
            "DESEDE" | "DESEDE/CBC/PKCS5PADDING" => {
                desede_cbc_decrypt(&ciphertext, &key_bytes)
                    .map_err(|e| CfmlError::runtime(e))?
            }
            "BLOWFISH" | "BLOWFISH/CBC/PKCS5PADDING" => {
                blowfish_cbc_decrypt(&ciphertext, &key_bytes)
                    .map_err(|e| CfmlError::runtime(e))?
            }
            _ => return Err(CfmlError::runtime(format!("Unsupported decryption algorithm: {}", algorithm)))
        }
    };

    let result = String::from_utf8(plaintext_bytes)
        .map_err(|e| CfmlError::runtime(format!("Decrypted data is not valid UTF-8: {}", e)))?;
    Ok(CfmlValue::string(result))
}

// ==== SYSTEM FUNCTIONS ====

fn fn_get_base_template_path(_args: Vec<CfmlValue>) -> CfmlResult {
    // VM-intercepted — this stub only runs if VM intercept misses
    Err(CfmlError::runtime("getBaseTemplatePath() requires VM context".to_string()))
}

fn fn_get_time_zone(_args: Vec<CfmlValue>) -> CfmlResult {
    // VM-intercepted — this stub only runs if VM intercept misses
    Err(CfmlError::runtime("getTimeZone() requires VM context".to_string()))
}

// ==== XML FUNCTIONS ====

/// CFML XML objects expose each distinct child-element tag name as a struct key
/// whose value is an array of the matching child elements (e.g. `node.fieldset`
/// returns an array of `<fieldset>` children, and `StructKeyExists(node,"fieldset")`
/// is true). Inject those keys from the element's `xmlChildren` collection.
#[cfg(feature = "xml")]
fn xml_inject_named_children(element: &mut ValueMap) {
    let mut groups: Vec<(String, Vec<CfmlValue>)> = Vec::new();
    if let Some(children) = element.get("xmlChildren").and_then(|v| v.as_cfml_array()) {
        for child in children.iter() {
            let name = match &child {
                CfmlValue::Struct(s) => s.get_ci("xmlName").map(|v| v.as_string()),
                _ => None,
            };
            if let Some(name) = name {
                if let Some(g) = groups.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(&name)) {
                    g.1.push(child.clone());
                } else {
                    groups.push((name, vec![child.clone()]));
                }
            }
        }
    }
    for (name, vals) in groups {
        // Never shadow the reserved XML-node keys.
        let lname = name.to_ascii_lowercase();
        if matches!(
            lname.as_str(),
            "xmlname" | "xmltype" | "xmltext" | "xmlchildren" | "xmlattributes"
                | "xmlcomment" | "xmlnsprefix" | "xmlnsuri" | "xmlparent" | "xmlroot"
                | "xmlcdata" | "xmlvalue"
        ) {
            continue;
        }
        element.insert(name, CfmlValue::array(vals));
    }
}

#[cfg(feature = "xml")]
fn fn_xml_parse(args: Vec<CfmlValue>) -> CfmlResult {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let xml_str = get_str(&args, 0);
    let mut reader = Reader::from_str(&xml_str);

    let mut stack: Vec<ValueMap> = Vec::new();
    let mut root: Option<ValueMap> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let mut element = ValueMap::default();
                let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                element.insert("xmlName".to_string(), CfmlValue::string(tag_name));
                element.insert("xmlType".to_string(), CfmlValue::string("ELEMENT".to_string()));
                element.insert("xmlText".to_string(), CfmlValue::string(String::new()));
                element.insert("xmlChildren".to_string(), CfmlValue::array(Vec::new()));

                let mut attrs = ValueMap::default();
                for attr in e.attributes().flatten() {
                    let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                    let val = String::from_utf8_lossy(&attr.value).to_string();
                    attrs.insert(key, CfmlValue::string(val));
                }
                element.insert("xmlAttributes".to_string(), CfmlValue::strukt(attrs));

                stack.push(element);
            }
            Ok(Event::End(_)) => {
                if let Some(mut completed) = stack.pop() {
                    xml_inject_named_children(&mut completed);
                    if let Some(parent) = stack.last_mut() {
                        if let Some(children) = parent.get_mut("xmlChildren").and_then(|v| v.as_cfml_array()) {
                            children.push(CfmlValue::strukt(completed));
                        }
                    } else {
                        root = Some(completed);
                    }
                }
            }
            Ok(Event::Empty(ref e)) => {
                let mut element = ValueMap::default();
                let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                element.insert("xmlName".to_string(), CfmlValue::string(tag_name));
                element.insert("xmlType".to_string(), CfmlValue::string("ELEMENT".to_string()));
                element.insert("xmlText".to_string(), CfmlValue::string(String::new()));
                element.insert("xmlChildren".to_string(), CfmlValue::array(Vec::new()));

                let mut attrs = ValueMap::default();
                for attr in e.attributes().flatten() {
                    let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                    let val = String::from_utf8_lossy(&attr.value).to_string();
                    attrs.insert(key, CfmlValue::string(val));
                }
                element.insert("xmlAttributes".to_string(), CfmlValue::strukt(attrs));

                if let Some(parent) = stack.last_mut() {
                    if let Some(children) = parent.get_mut("xmlChildren").and_then(|v| v.as_cfml_array()) {
                        children.push(CfmlValue::strukt(element));
                    }
                } else {
                    root = Some(element);
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or(std::borrow::Cow::Borrowed("")).to_string();
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() {
                    if let Some(current) = stack.last_mut() {
                        if let Some(CfmlValue::String(ref mut s)) = current.get_mut("xmlText") {
                            let inner = std::sync::Arc::make_mut(s);
                            if !inner.is_empty() { inner.push(' '); }
                            inner.push_str(&trimmed);
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(CfmlError::runtime(format!("XML parse error: {}", e))),
            _ => {}
        }
    }

    match root {
        Some(root_element) => {
            let mut doc = ValueMap::default();
            // On the document node, the root element is reachable by its tag name
            // as a single element (e.g. `xmlDoc.form`), not wrapped in an array.
            let root_name = root_element.get("xmlName").map(|v| v.as_string());
            let root_val = CfmlValue::strukt(root_element);
            doc.insert("xmlRoot".to_string(), root_val.clone());
            doc.insert("xmlType".to_string(), CfmlValue::string("DOCUMENT".to_string()));
            if let Some(name) = root_name {
                if !name.is_empty() {
                    doc.insert(name, root_val);
                }
            }
            Ok(CfmlValue::strukt(doc))
        }
        None => Err(CfmlError::runtime("Empty or invalid XML document".to_string()))
    }
}

#[cfg(feature = "xml")]
fn fn_xml_search(args: Vec<CfmlValue>) -> CfmlResult {
    let doc = args.get(0).cloned().unwrap_or(CfmlValue::Null);
    let path_expr = get_str(&args, 1);

    let mut results = Vec::new();

    let search_root = if let CfmlValue::Struct(ref s) = doc {
        if let Some(root) = s.get("xmlRoot") {
            root.clone()
        } else {
            doc.clone()
        }
    } else {
        doc.clone()
    };

    if path_expr.starts_with("//") {
        let tag_name = &path_expr[2..];
        xml_search_descendants(&search_root, tag_name, &mut results);
    } else {
        let parts: Vec<&str> = path_expr.trim_start_matches('/').split('/').collect();
        xml_search_path(&search_root, &parts, 0, &mut results);
    }

    Ok(CfmlValue::array(results))
}

#[cfg(feature = "xml")]
fn xml_search_descendants(node: &CfmlValue, tag_name: &str, results: &mut Vec<CfmlValue>) {
    if let CfmlValue::Struct(ref s) = node {
        if let Some(CfmlValue::String(ref name)) = s.get("xmlName") {
            if name.as_str() == tag_name || tag_name == "*" {
                results.push(node.clone());
            }
        }
        if let Some(CfmlValue::Array(ref children)) = s.get("xmlChildren") {
            for child in children.iter() {
                xml_search_descendants(&child, tag_name, results);
            }
        }
    }
}

#[cfg(feature = "xml")]
fn xml_search_path(node: &CfmlValue, parts: &[&str], depth: usize, results: &mut Vec<CfmlValue>) {
    if depth >= parts.len() {
        results.push(node.clone());
        return;
    }

    let target = parts[depth];

    if let CfmlValue::Struct(ref s) = node {
        if let Some(CfmlValue::String(ref name)) = s.get("xmlName") {
            if name.as_str() == target || target == "*" {
                if depth == parts.len() - 1 {
                    results.push(node.clone());
                } else if let Some(CfmlValue::Array(ref children)) = s.get("xmlChildren") {
                    for child in children.iter() {
                        xml_search_path(&child, parts, depth + 1, results);
                    }
                }
            }
        }
    }
}

#[cfg(feature = "xml")]
fn fn_is_xml(args: Vec<CfmlValue>) -> CfmlResult {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let s = get_str(&args, 0);
    let mut reader = Reader::from_str(&s);
    let mut found_element = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(_)) | Ok(Event::Empty(_)) => { found_element = true; }
            Ok(Event::Eof) => break,
            Err(_) => return Ok(CfmlValue::Bool(false)),
            _ => {}
        }
    }
    Ok(CfmlValue::Bool(found_element))
}

#[cfg(feature = "xml")]
fn fn_xml_transform_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("xmlTransform() is not supported (requires XSLT engine)".to_string()))
}

#[cfg(feature = "xml")]
fn fn_xml_validate_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("xmlValidate() is not supported (requires schema validation engine)".to_string()))
}

// ---- XML DOM Functions ----

#[cfg(feature = "xml")]
fn fn_xml_new(args: Vec<CfmlValue>) -> CfmlResult {
    let _case_sensitive = args.get(0).map(|v| v.is_true()).unwrap_or(false);
    // An empty doc has no xmlRoot until a root element is attached.
    // Matches Lucee: structKeyExists(xmlNew(), "xmlRoot") returns false.
    let mut doc = ValueMap::default();
    doc.insert("xmlComment".to_string(), CfmlValue::string(String::new()));
    doc.insert("__xmlDoc".to_string(), CfmlValue::Bool(true));
    let mut doc_type = ValueMap::default();
    doc_type.insert("type".to_string(), CfmlValue::string(String::new()));
    doc_type.insert("name".to_string(), CfmlValue::string(String::new()));
    doc.insert("xmlDocType".to_string(), CfmlValue::strukt(doc_type));
    doc.insert("xmlChildren".to_string(), CfmlValue::array(Vec::new()));
    Ok(CfmlValue::strukt(doc))
}

#[cfg(feature = "xml")]
fn fn_xml_elem_new(args: Vec<CfmlValue>) -> CfmlResult {
    let _doc = args.get(0); // xmlDoc (unused but accepted)
    let (namespace, child_name) = if args.len() >= 3 {
        (get_str(&args, 1), get_str(&args, 2))
    } else {
        (String::new(), get_str(&args, 1))
    };
    let mut elem = ValueMap::default();
    elem.insert("xmlName".to_string(), CfmlValue::string(child_name));
    elem.insert("xmlNsPrefix".to_string(), CfmlValue::string(String::new()));
    elem.insert("xmlNsURI".to_string(), CfmlValue::string(namespace));
    elem.insert("xmlText".to_string(), CfmlValue::string(String::new()));
    elem.insert("xmlComment".to_string(), CfmlValue::string(String::new()));
    elem.insert("xmlCData".to_string(), CfmlValue::string(String::new()));
    elem.insert("xmlAttributes".to_string(), CfmlValue::strukt(ValueMap::default()));
    elem.insert("xmlChildren".to_string(), CfmlValue::array(Vec::new()));
    Ok(CfmlValue::strukt(elem))
}

#[cfg(feature = "xml")]
fn fn_xml_child_pos(args: Vec<CfmlValue>) -> CfmlResult {
    let element = args.get(0).cloned().unwrap_or(CfmlValue::Null);
    let child_name = get_str(&args, 1).to_lowercase();
    let nth = get_int(&args, 2) as usize;
    if let CfmlValue::Struct(ref s) = element {
        if let Some(CfmlValue::Array(ref children)) = s.get("xmlChildren") {
            let mut count = 0usize;
            for (i, child) in children.iter().enumerate() {
                if let CfmlValue::Struct(ref cs) = child {
                    if let Some(CfmlValue::String(ref name)) = cs.get("xmlName") {
                        if name.to_lowercase() == child_name {
                            count += 1;
                            if count == nth {
                                return Ok(CfmlValue::Int((i + 1) as i64));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(CfmlValue::Int(-1))
}

#[cfg(feature = "xml")]
fn fn_xml_get_node_type(args: Vec<CfmlValue>) -> CfmlResult {
    let node = args.get(0).cloned().unwrap_or(CfmlValue::Null);
    if let CfmlValue::Struct(ref s) = node {
        if s.contains_key("__xmlDoc") || s.contains_key("xmlRoot") {
            return Ok(CfmlValue::string("DOCUMENT_NODE".to_string()));
        }
        if let Some(CfmlValue::String(ref t)) = s.get("xmlType") {
            let result = match t.to_uppercase().as_str() {
                "ELEMENT" => "ELEMENT_NODE",
                "TEXT" => "TEXT_NODE",
                "COMMENT" => "COMMENT_NODE",
                "CDATA" => "CDATA_SECTION_NODE",
                "ATTRIBUTE" | "ATTRIBUTE_NODE" => "ATTRIBUTE_NODE",
                _ => "ELEMENT_NODE",
            };
            return Ok(CfmlValue::string(result.to_string()));
        }
        if s.contains_key("xmlName") {
            return Ok(CfmlValue::string("ELEMENT_NODE".to_string()));
        }
    }
    Ok(CfmlValue::string("UNKNOWN_NODE".to_string()))
}

#[cfg(feature = "xml")]
fn fn_xml_has_child(args: Vec<CfmlValue>) -> CfmlResult {
    let node = args.get(0).cloned().unwrap_or(CfmlValue::Null);
    if let CfmlValue::Struct(ref s) = node {
        if let Some(CfmlValue::Array(ref children)) = s.get("xmlChildren") {
            return Ok(CfmlValue::Bool(!children.is_empty()));
        }
    }
    Ok(CfmlValue::Bool(false))
}

#[cfg(feature = "xml")]
fn fn_is_xml_doc(args: Vec<CfmlValue>) -> CfmlResult {
    let val = args.get(0).cloned().unwrap_or(CfmlValue::Null);
    if let CfmlValue::Struct(ref s) = val {
        // A struct is an XML doc if it has our __xmlDoc marker or an xmlRoot key
        // (xmlRoot is set once a root element is attached).
        return Ok(CfmlValue::Bool(
            s.contains_key("__xmlDoc") || s.contains_key("xmlRoot"),
        ));
    }
    Ok(CfmlValue::Bool(false))
}

#[cfg(feature = "xml")]
fn fn_is_xml_elem(args: Vec<CfmlValue>) -> CfmlResult {
    let val = args.get(0).cloned().unwrap_or(CfmlValue::Null);
    if let CfmlValue::Struct(ref s) = val {
        return Ok(CfmlValue::Bool(s.contains_key("xmlName") && s.contains_key("xmlChildren")));
    }
    Ok(CfmlValue::Bool(false))
}

#[cfg(feature = "xml")]
fn fn_is_xml_node(args: Vec<CfmlValue>) -> CfmlResult {
    let val = args.get(0).cloned().unwrap_or(CfmlValue::Null);
    if let CfmlValue::Struct(ref s) = val {
        return Ok(CfmlValue::Bool(
            s.contains_key("xmlName")
                || s.contains_key("xmlRoot")
                || s.contains_key("__xmlDoc"),
        ));
    }
    Ok(CfmlValue::Bool(false))
}

#[cfg(feature = "xml")]
fn fn_is_xml_root(args: Vec<CfmlValue>) -> CfmlResult {
    let val = args.get(0).cloned().unwrap_or(CfmlValue::Null);
    if let CfmlValue::Struct(ref s) = val {
        return Ok(CfmlValue::Bool(s.contains_key("xmlRoot")));
    }
    Ok(CfmlValue::Bool(false))
}

#[cfg(feature = "xml")]
fn fn_is_xml_attribute(args: Vec<CfmlValue>) -> CfmlResult {
    let val = args.get(0).cloned().unwrap_or(CfmlValue::Null);
    if let CfmlValue::Struct(ref s) = val {
        if let Some(CfmlValue::String(ref t)) = s.get("xmlType") {
            return Ok(CfmlValue::Bool(t.as_str() == "ATTRIBUTE_NODE"));
        }
    }
    Ok(CfmlValue::Bool(false))
}

#[cfg(feature = "html")]
fn fn_html_parse(args: Vec<CfmlValue>) -> CfmlResult {
    use scraper::{Html, Node};
    use ego_tree::NodeRef;

    fn walk_node(node_ref: NodeRef<Node>) -> Option<CfmlValue> {
        match node_ref.value() {
            Node::Element(el) => {
                let tag_name = el.name.local.to_string();
                let mut element = ValueMap::default();
                element.insert("xmlName".to_string(), CfmlValue::string(tag_name));
                element.insert("xmlType".to_string(), CfmlValue::string("ELEMENT".to_string()));

                let mut attrs = ValueMap::default();
                for (name, val) in el.attrs() {
                    attrs.insert(name.to_string(), CfmlValue::string(val.to_string()));
                }
                element.insert("xmlAttributes".to_string(), CfmlValue::strukt(attrs));

                let mut children = Vec::new();
                let mut text_parts = Vec::new();
                for child in node_ref.children() {
                    if let Some(child_val) = walk_node(child) {
                        if let CfmlValue::Struct(ref cs) = child_val {
                            if let Some(CfmlValue::String(ref t)) = cs.get("xmlType") {
                                if t.as_str() == "TEXT" {
                                    if let Some(CfmlValue::String(ref txt)) = cs.get("xmlText") {
                                        text_parts.push((**txt).clone());
                                    }
                                }
                            }
                        }
                        children.push(child_val);
                    }
                }
                element.insert("xmlText".to_string(), CfmlValue::string(text_parts.join("")));
                element.insert("xmlChildren".to_string(), CfmlValue::array(children));
                Some(CfmlValue::strukt(element))
            }
            Node::Text(t) => {
                let text = t.to_string();
                if text.trim().is_empty() { return None; }
                let mut element = ValueMap::default();
                element.insert("xmlName".to_string(), CfmlValue::string("#text".to_string()));
                element.insert("xmlType".to_string(), CfmlValue::string("TEXT".to_string()));
                element.insert("xmlText".to_string(), CfmlValue::string(text));
                element.insert("xmlChildren".to_string(), CfmlValue::array(Vec::new()));
                element.insert("xmlAttributes".to_string(), CfmlValue::strukt(ValueMap::default()));
                Some(CfmlValue::strukt(element))
            }
            _ => None,
        }
    }

    let html_str = get_str(&args, 0);
    let html = Html::parse_document(&html_str);

    let mut doc = ValueMap::default();
    doc.insert("xmlType".to_string(), CfmlValue::string("DOCUMENT".to_string()));

    let root_el = html.root_element();
    if let Some(root_cfml) = walk_node(*root_el) {
        doc.insert("xmlRoot".to_string(), root_cfml);
    } else {
        let mut root = ValueMap::default();
        root.insert("xmlName".to_string(), CfmlValue::string("html".to_string()));
        root.insert("xmlType".to_string(), CfmlValue::string("ELEMENT".to_string()));
        root.insert("xmlText".to_string(), CfmlValue::string(String::new()));
        root.insert("xmlChildren".to_string(), CfmlValue::array(Vec::new()));
        root.insert("xmlAttributes".to_string(), CfmlValue::strukt(ValueMap::default()));
        doc.insert("xmlRoot".to_string(), CfmlValue::strukt(root));
    }

    Ok(CfmlValue::strukt(doc))
}

// ---- Soundex ----
fn fn_soundex(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    if s.is_empty() {
        return Ok(CfmlValue::string(String::new()));
    }
    let chars: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if chars.is_empty() {
        return Ok(CfmlValue::string(String::new()));
    }
    let first = chars[0].to_ascii_uppercase();
    let soundex_code = |c: char| -> Option<char> {
        match c.to_ascii_uppercase() {
            'B' | 'F' | 'P' | 'V' => Some('1'),
            'C' | 'G' | 'J' | 'K' | 'Q' | 'S' | 'X' | 'Z' => Some('2'),
            'D' | 'T' => Some('3'),
            'L' => Some('4'),
            'M' | 'N' => Some('5'),
            'R' => Some('6'),
            _ => None, // A, E, I, O, U, H, W, Y
        }
    };
    let is_hw = |c: char| -> bool {
        matches!(c.to_ascii_uppercase(), 'H' | 'W')
    };
    let mut result = String::new();
    result.push(first);
    let first_code = soundex_code(first);
    let mut last_code = first_code;
    for &c in &chars[1..] {
        if is_hw(c) {
            // H and W are transparent — don't update last_code
            continue;
        }
        let code = soundex_code(c);
        if let Some(cd) = code {
            if code != last_code {
                result.push(cd);
                if result.len() == 4 {
                    break;
                }
            }
            last_code = code;
        } else {
            // Vowel resets the last_code so same consonant codes can appear again
            last_code = None;
        }
    }
    while result.len() < 4 {
        result.push('0');
    }
    Ok(CfmlValue::string(result))
}

// ---- Metaphone ----
fn fn_metaphone(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0).to_uppercase();
    if s.is_empty() {
        return Ok(CfmlValue::string(String::new()));
    }
    let chars: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if chars.is_empty() {
        return Ok(CfmlValue::string(String::new()));
    }
    let len = chars.len();
    let mut result = String::new();
    let mut i = 0;

    // Drop initial silent letters
    if len >= 2 {
        match (chars[0], chars[1]) {
            ('A', 'E') | ('G', 'N') | ('K', 'N') | ('P', 'N') | ('W', 'R') => i = 1,
            _ => {}
        }
    }

    while i < len {
        let c = chars[i];
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        let next = if i + 1 < len { Some(chars[i + 1]) } else { None };

        // Skip duplicate adjacent letters (except C)
        if c != 'C' && prev == Some(c) {
            i += 1;
            continue;
        }

        match c {
            'A' | 'E' | 'I' | 'O' | 'U' => {
                if i == 0 { result.push(c); }
            }
            'B' => {
                if prev != Some('M') || i == 0 {
                    result.push('B');
                }
            }
            'C' => {
                if next == Some('H') {
                    // SCH -> SK (German-origin, matches Apache Commons Metaphone)
                    if prev == Some('S') {
                        result.push('K');
                    } else {
                        result.push('X');
                    }
                    i += 1;
                } else if next == Some('I') || next == Some('E') || next == Some('Y') {
                    if next == Some('I') && i + 2 < len && chars[i + 2] == 'A' {
                        result.push('X');
                        i += 2;
                    } else {
                        result.push('S');
                    }
                } else {
                    result.push('K');
                }
            }
            'D' => {
                if next == Some('G') && i + 2 < len && matches!(chars[i + 2], 'I' | 'E' | 'Y') {
                    result.push('J');
                } else {
                    result.push('T');
                }
            }
            'F' => { result.push('F'); }
            'G' => {
                if next == Some('H') && i + 2 < len && !"AEIOU".contains(chars[i + 2]) {
                    i += 1; // silent GH
                } else if i > 0 && next == Some('N') {
                    // silent G before N (but not at start)
                } else if prev == Some('G') {
                    // skip duplicate
                } else {
                    result.push('J');
                    if next == Some('H') || next == Some('I') || next == Some('E') || next == Some('Y') {
                        // already pushed J
                    } else {
                        result.pop();
                        result.push('K');
                    }
                }
            }
            'H' => {
                if "AEIOU".contains(next.unwrap_or('X')) && (prev.is_none() || !"AEIOU".contains(prev.unwrap())) {
                    result.push('H');
                }
            }
            'J' => { result.push('J'); }
            'K' => {
                if prev != Some('C') {
                    result.push('K');
                }
            }
            'L' => { result.push('L'); }
            'M' => { result.push('M'); }
            'N' => { result.push('N'); }
            'P' => {
                if next == Some('H') {
                    result.push('F');
                    i += 1;
                } else {
                    result.push('P');
                }
            }
            'Q' => { result.push('K'); }
            'R' => { result.push('R'); }
            'S' => {
                if next == Some('H') || (next == Some('I') && i + 2 < len && (chars[i + 2] == 'O' || chars[i + 2] == 'A')) {
                    result.push('X');
                    i += 1;
                } else if next == Some('C') && i + 2 < len && matches!(chars[i + 2], 'I' | 'E' | 'Y') {
                    result.push('S');
                    i += 1; // skip C, S already pushed
                } else {
                    result.push('S');
                }
            }
            'T' => {
                if next == Some('H') {
                    result.push('0'); // theta
                    i += 1;
                } else if next == Some('I') && i + 2 < len && (chars[i + 2] == 'O' || chars[i + 2] == 'A') {
                    result.push('X');
                } else {
                    result.push('T');
                }
            }
            'V' => { result.push('F'); }
            'W' | 'Y' => {
                if "AEIOU".contains(next.unwrap_or('X')) {
                    result.push(c);
                }
            }
            'X' => {
                result.push('K');
                result.push('S');
            }
            'Z' => { result.push('S'); }
            _ => {}
        }
        i += 1;
    }
    // Deduplicate consecutive identical characters in output
    let mut deduped = String::new();
    for ch in result.chars() {
        if deduped.chars().last() != Some(ch) {
            deduped.push(ch);
        }
    }
    // Apache Commons Metaphone caps at 4 characters by default (matches Lucee)
    let truncated: String = deduped.chars().take(4).collect();
    Ok(CfmlValue::string(truncated))
}

// ---- toScript ----
fn fn_to_script(args: Vec<CfmlValue>) -> CfmlResult {
    let value = args.get(0).cloned().unwrap_or(CfmlValue::Null);
    let var_name = get_str(&args, 1);

    fn js_escape(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r")
    }

    // Lucee format: no `var` keyword, no spaces around `=`, booleans as strings.
    let script = match &value {
        CfmlValue::String(s) => format!("{}=\"{}\";", var_name, js_escape(s)),
        CfmlValue::Int(n) => format!("{}={};", var_name, n),
        CfmlValue::Double(d) => format!("{}={};", var_name, d),
        CfmlValue::Bool(b) => format!("{}=\"{}\";", var_name, if *b { "true" } else { "false" }),
        CfmlValue::Array(arr) => {
            let mut lines = vec![format!("{}=new Array();", var_name)];
            for (i, item) in arr.iter().enumerate() {
                let val_str = match &item {
                    CfmlValue::String(s) => format!("\"{}\"", js_escape(s)),
                    CfmlValue::Int(n) => n.to_string(),
                    CfmlValue::Double(d) => d.to_string(),
                    CfmlValue::Bool(b) => if *b { "true".to_string() } else { "false".to_string() },
                    _ => format!("\"{}\"", js_escape(&item.as_string())),
                };
                lines.push(format!("{}[{}]={};", var_name, i + 1, val_str));
            }
            lines.join("\n")
        }
        CfmlValue::Struct(s) => {
            let mut lines = vec![format!("{}=new Object();", var_name)];
            for (k, v) in s.iter() {
                let val_str = match v {
                    CfmlValue::String(s) => format!("\"{}\"", js_escape(&s)),
                    CfmlValue::Int(n) => n.to_string(),
                    CfmlValue::Double(d) => d.to_string(),
                    CfmlValue::Bool(b) => if b { "true".to_string() } else { "false".to_string() },
                    _ => format!("\"{}\"", js_escape(&v.as_string())),
                };
                lines.push(format!("{}.{}={};", var_name, k, val_str));
            }
            lines.join("\n")
        }
        CfmlValue::Null => format!("{}=null;", var_name),
        _ => format!("{}=\"{}\";", var_name, js_escape(&value.as_string())),
    };
    Ok(CfmlValue::string(script))
}

// ======================================================================
// NEW BUILT-IN FUNCTION IMPLEMENTATIONS
// ======================================================================

// ---- String functions ----

fn fn_uc_first(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    if s.is_empty() {
        return Ok(CfmlValue::string(s));
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap().to_uppercase().to_string();
    Ok(CfmlValue::string(first + chars.as_str()))
}

fn fn_js_string_format(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let escaped = s
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    Ok(CfmlValue::string(escaped))
}

fn fn_re_escape(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    Ok(CfmlValue::string(regex::escape(&s)))
}

fn fn_get_token(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let index = get_int(&args, 1) as usize;
    let delims = if args.len() > 2 { get_str(&args, 2) } else { " \t\n\r".to_string() };
    let tokens: Vec<&str> = s.split(|c: char| delims.contains(c))
        .filter(|t| !t.is_empty())
        .collect();
    if index >= 1 && index <= tokens.len() {
        Ok(CfmlValue::string(tokens[index - 1].to_string()))
    } else {
        Ok(CfmlValue::string(String::new()))
    }
}

fn fn_new_line(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::string("\n".to_string()))
}

// ---- Array functions ----

fn fn_array_index_exists(args: Vec<CfmlValue>) -> CfmlResult {
    match args.get(0) {
        Some(CfmlValue::Array(arr)) => {
            let idx = get_int(&args, 1) as usize;
            Ok(CfmlValue::Bool(idx >= 1 && idx <= arr.len()))
        }
        _ => Err(CfmlError::runtime("arrayIndexExists() requires an array".to_string())),
    }
}

fn fn_array_resize(args: Vec<CfmlValue>) -> CfmlResult {
    match args.get(0) {
        Some(CfmlValue::Array(arr)) => {
            let size = get_int(&args, 1) as usize;
            // In-place grow on the shared handle.
            arr.with_write(|v| {
                while v.len() < size {
                    // Lucee grows with NULLs, not empty strings — `arr[i] ?: dflt`
                    // and isNull() checks over a resized array depended on this.
                    v.push(CfmlValue::Null);
                }
            });
            Ok(CfmlValue::Array(arr.clone()))
        }
        _ => Err(CfmlError::runtime("arrayResize() requires an array".to_string())),
    }
}

fn fn_array_median(args: Vec<CfmlValue>) -> CfmlResult {
    match args.get(0) {
        Some(CfmlValue::Array(arr)) => {
            if arr.is_empty() {
                return Err(CfmlError::runtime("Cannot get median of empty array".to_string()));
            }
            let mut nums: Vec<f64> = arr.iter().map(|v| match &v {
                CfmlValue::Int(i) => *i as f64,
                CfmlValue::Double(d) => *d,
                CfmlValue::String(s) => s.parse().unwrap_or(0.0),
                _ => 0.0,
            }).collect();
            nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mid = nums.len() / 2;
            let median = if nums.len() % 2 == 0 {
                (nums[mid - 1] + nums[mid]) / 2.0
            } else {
                nums[mid]
            };
            Ok(CfmlValue::Double(median))
        }
        _ => Err(CfmlError::runtime("arrayMedian() requires an array".to_string())),
    }
}

fn fn_array_mid(args: Vec<CfmlValue>) -> CfmlResult {
    match args.get(0) {
        Some(CfmlValue::Array(arr)) => {
            let snap = arr.snapshot();
            let start = (get_int(&args, 1) as usize).saturating_sub(1);
            let count = get_int(&args, 2) as usize;
            let end = std::cmp::min(start + count, snap.len());
            if start >= snap.len() {
                return Ok(CfmlValue::array(Vec::new()));
            }
            Ok(CfmlValue::array(snap[start..end].to_vec()))
        }
        _ => Err(CfmlError::runtime("arrayMid() requires an array".to_string())),
    }
}

fn fn_array_splice(args: Vec<CfmlValue>) -> CfmlResult {
    match args.get(0) {
        Some(CfmlValue::Array(arr)) => {
            let index = (get_int(&args, 1) as usize).saturating_sub(1);
            let len = arr.len();
            let delete_count = if args.len() > 2 {
                get_int(&args, 2) as usize
            } else {
                len.saturating_sub(index)
            };
            let replacements: Vec<CfmlValue> = match args.get(3) {
                Some(CfmlValue::Array(r)) => r.snapshot(),
                _ => Vec::new(),
            };
            // Reference semantics: splice the shared array in place and return
            // the removed elements (matches Lucee/JS — the original is modified).
            let removed = arr.with_write(|v| {
                let end = std::cmp::min(index + delete_count, v.len());
                let removed: Vec<CfmlValue> = if index < v.len() {
                    v.drain(index..end).collect()
                } else {
                    Vec::new()
                };
                for (i, val) in replacements.into_iter().enumerate() {
                    let pos = std::cmp::min(index + i, v.len());
                    v.insert(pos, val);
                }
                removed
            });
            Ok(CfmlValue::array(removed))
        }
        _ => Err(CfmlError::runtime("arraySplice() requires an array".to_string())),
    }
}

fn fn_array_range(args: Vec<CfmlValue>) -> CfmlResult {
    let from = get_int(&args, 0);
    let to = get_int(&args, 1);
    let mut result = Vec::new();
    if from <= to {
        for i in from..=to {
            result.push(CfmlValue::Int(i));
        }
    } else {
        for i in (to..=from).rev() {
            result.push(CfmlValue::Int(i));
        }
    }
    Ok(CfmlValue::array(result))
}

fn fn_array_to_struct(args: Vec<CfmlValue>) -> CfmlResult {
    match args.get(0) {
        Some(CfmlValue::Array(arr)) => {
            let mut map = ValueMap::default();
            for (i, val) in arr.iter().enumerate() {
                map.insert((i + 1).to_string(), val.clone());
            }
            Ok(CfmlValue::strukt(map))
        }
        _ => Err(CfmlError::runtime("arrayToStruct() requires an array".to_string())),
    }
}

fn fn_array_delete_no_case(args: Vec<CfmlValue>) -> CfmlResult {
    match args.get(0) {
        Some(CfmlValue::Array(arr)) => {
            let target = get_str(&args, 1).to_lowercase();
            // In-place: remove the first case-insensitive match from the shared
            // array (consistent with the VM-intercepted arrayDelete path).
            let removed = arr.with_write(|v| {
                if let Some(pos) =
                    v.iter().position(|x| x.as_string().to_lowercase() == target)
                {
                    v.remove(pos);
                    true
                } else {
                    false
                }
            });
            Ok(CfmlValue::Bool(removed))
        }
        _ => Err(CfmlError::runtime("arrayDeleteNoCase() requires an array".to_string())),
    }
}

// ---- Struct functions ----

fn fn_struct_to_sorted(args: Vec<CfmlValue>) -> CfmlResult {
    match args.get(0) {
        Some(CfmlValue::Struct(s)) => {
            let mut keys: Vec<String> = s.keys();
            let sort_type = get_str(&args, 1).to_lowercase();
            if sort_type == "textnocase" {
                keys.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
            } else {
                keys.sort();
            }
            let mut result = ValueMap::default();
            for key in keys {
                if let Some(val) = s.get(&key) {
                    result.insert(key, val.clone());
                }
            }
            Ok(CfmlValue::strukt(result))
        }
        _ => Err(CfmlError::runtime("structToSorted() requires a struct".to_string())),
    }
}

fn fn_struct_is_ordered(_args: Vec<CfmlValue>) -> CfmlResult {
    // Rust HashMap is not ordered, so always return false
    Ok(CfmlValue::Bool(false))
}

fn fn_struct_is_case_sensitive(_args: Vec<CfmlValue>) -> CfmlResult {
    // CFML structs are case-insensitive by default
    Ok(CfmlValue::Bool(false))
}

fn fn_struct_to_query_string(args: Vec<CfmlValue>) -> CfmlResult {
    fn url_enc(s: &str) -> String {
        let mut result = String::new();
        for c in s.chars() {
            match c {
                'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '*' => result.push(c),
                ' ' => result.push_str("%20"),
                _ => {
                    for b in c.to_string().as_bytes() {
                        result.push_str(&format!("%{:02X}", b));
                    }
                }
            }
        }
        result
    }
    match args.get(0) {
        Some(CfmlValue::Struct(s)) => {
            let delim = if args.len() > 1 { get_str(&args, 1) } else { "&".to_string() };
            let pairs: Vec<String> = s.iter()
                .map(|(k, v)| format!("{}={}", url_enc(&k), url_enc(&v.as_string())))
                .collect();
            Ok(CfmlValue::string(pairs.join(&delim)))
        }
        _ => Err(CfmlError::runtime("structToQueryString() requires a struct".to_string())),
    }
}

// ---- Conversion functions ----

fn fn_create_time_span(args: Vec<CfmlValue>) -> CfmlResult {
    let days = get_float(&args, 0);
    let hours = get_float(&args, 1);
    let minutes = get_float(&args, 2);
    let seconds = get_float(&args, 3);
    let total_days = days + hours / 24.0 + minutes / 1440.0 + seconds / 86400.0;
    // A distinct TimeSpan value (numerically the fractional-day Double) so
    // `getClass().getName()` and the `timespan` type-check can recognise it,
    // while it still behaves as a Double in every arithmetic/coercion context.
    Ok(CfmlValue::TimeSpan(total_days))
}

fn fn_yes_no_format(args: Vec<CfmlValue>) -> CfmlResult {
    let val = args.get(0).unwrap_or(&CfmlValue::Bool(false));
    let result = match val {
        CfmlValue::Bool(b) => if *b { "Yes" } else { "No" },
        CfmlValue::Int(i) => if *i != 0 { "Yes" } else { "No" },
        CfmlValue::Double(d) => if *d != 0.0 { "Yes" } else { "No" },
        CfmlValue::String(s) => {
            let lower = s.to_lowercase();
            if lower == "yes" || lower == "true" || s.parse::<f64>().map(|n| n != 0.0).unwrap_or(false) {
                "Yes"
            } else {
                "No"
            }
        }
        _ => "No",
    };
    Ok(CfmlValue::string(result.to_string()))
}

fn fn_true_false_format(args: Vec<CfmlValue>) -> CfmlResult {
    let val = args.get(0).unwrap_or(&CfmlValue::Bool(false));
    let result = match val {
        CfmlValue::Bool(b) => *b,
        CfmlValue::Int(i) => *i != 0,
        CfmlValue::Double(d) => *d != 0.0,
        CfmlValue::String(s) => {
            let lower = s.to_lowercase();
            lower == "yes" || lower == "true" || s.parse::<f64>().map(|n| n != 0.0).unwrap_or(false)
        }
        _ => false,
    };
    Ok(CfmlValue::string(if result { "true" } else { "false" }.to_string()))
}

fn fn_null_value(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Null)
}

fn fn_increment_value(args: Vec<CfmlValue>) -> CfmlResult {
    match args.get(0) {
        Some(CfmlValue::Int(i)) => Ok(CfmlValue::Int(i + 1)),
        Some(CfmlValue::Double(d)) => Ok(CfmlValue::Double(d + 1.0)),
        Some(v) => {
            let n = v.as_string().parse::<f64>().unwrap_or(0.0);
            if n.fract() == 0.0 { Ok(CfmlValue::Int(n as i64 + 1)) }
            else { Ok(CfmlValue::Double(n + 1.0)) }
        }
        _ => Ok(CfmlValue::Int(1)),
    }
}

fn fn_decrement_value(args: Vec<CfmlValue>) -> CfmlResult {
    match args.get(0) {
        Some(CfmlValue::Int(i)) => Ok(CfmlValue::Int(i - 1)),
        Some(CfmlValue::Double(d)) => Ok(CfmlValue::Double(d - 1.0)),
        Some(v) => {
            let n = v.as_string().parse::<f64>().unwrap_or(0.0);
            if n.fract() == 0.0 { Ok(CfmlValue::Int(n as i64 - 1)) }
            else { Ok(CfmlValue::Double(n - 1.0)) }
        }
        _ => Ok(CfmlValue::Int(-1)),
    }
}

fn fn_de(args: Vec<CfmlValue>) -> CfmlResult {
    // DE() - delay evaluation. Returns the input wrapped in double quotes so
    // it can be safely passed through evaluate() without being interpreted.
    // Matches Lucee: de("hello") -> "\"hello\""
    let s = get_str(&args, 0);
    let escaped = s.replace('"', "\"\"");
    Ok(CfmlValue::string(format!("\"{}\"", escaped)))
}

fn fn_dollar_format(args: Vec<CfmlValue>) -> CfmlResult {
    let num = get_float(&args, 0);
    let abs = num.abs();
    let formatted = format!("{:.2}", abs);
    // Add comma separators to integer part
    let parts: Vec<&str> = formatted.split('.').collect();
    let int_part = parts[0];
    let dec_part = parts.get(1).unwrap_or(&"00");
    let int_with_commas = {
        let chars: Vec<char> = int_part.chars().rev().collect();
        let mut result = String::new();
        for (i, c) in chars.iter().enumerate() {
            if i > 0 && i % 3 == 0 {
                result.push(',');
            }
            result.push(*c);
        }
        result.chars().rev().collect::<String>()
    };
    if num < 0.0 {
        Ok(CfmlValue::string(format!("(${}. {})", int_with_commas, dec_part)))
    } else {
        Ok(CfmlValue::string(format!("${}.{}", int_with_commas, dec_part)))
    }
}

// ---- Query functions ----

fn fn_query_column_exists(args: Vec<CfmlValue>) -> CfmlResult {
    match args.get(0) {
        Some(CfmlValue::Query(q)) => {
            let exists = q.has_column_ci(&get_str(&args, 1));
            Ok(CfmlValue::Bool(exists))
        }
        _ => Err(CfmlError::runtime("queryColumnExists() requires a query".to_string())),
    }
}

fn fn_query_slice(args: Vec<CfmlValue>) -> CfmlResult {
    match args.get(0) {
        Some(CfmlValue::Query(q)) => {
            let offset = (get_int(&args, 1) as usize).saturating_sub(1);
            let sliced = q.with_read(|d| {
                let row_count = d.row_count();
                let length = if args.len() > 2 {
                    get_int(&args, 2) as usize
                } else {
                    row_count.saturating_sub(offset)
                };
                let end = std::cmp::min(offset + length, row_count);
                let new_data: Vec<std::sync::Arc<Vec<CfmlValue>>> = if offset < row_count {
                    d.data.iter().map(|col| std::sync::Arc::new(col[offset..end].to_vec())).collect()
                } else {
                    d.columns.iter().map(|_| std::sync::Arc::new(Vec::new())).collect()
                };
                CfmlQueryData { columns: d.columns.clone(), data: new_data, sql: None, execution_time: None, current_row: 1 }
            });
            Ok(CfmlValue::Query(CfmlQuery::from_data(sliced)))
        }
        _ => Err(CfmlError::runtime("querySlice() requires a query".to_string())),
    }
}

fn fn_query_get_result(_args: Vec<CfmlValue>) -> CfmlResult {
    // Returns metadata about last query execution
    let mut result = ValueMap::default();
    result.insert("sql".to_string(), CfmlValue::string(String::new()));
    result.insert("cached".to_string(), CfmlValue::Bool(false));
    result.insert("executionTime".to_string(), CfmlValue::Int(0));
    result.insert("recordCount".to_string(), CfmlValue::Int(0));
    Ok(CfmlValue::strukt(result))
}

fn fn_query_column_data(args: Vec<CfmlValue>) -> CfmlResult {
    match args.get(0) {
        Some(CfmlValue::Query(q)) => {
            let col = get_str(&args, 1);
            let values = q.with_read(|d| {
                d.column_data_ci(&col)
                    .cloned()
                    .unwrap_or_else(|| vec![CfmlValue::string(String::new()); d.row_count()])
            });
            Ok(CfmlValue::array(values))
        }
        _ => Err(CfmlError::runtime("queryColumnData() requires a query".to_string())),
    }
}

fn fn_query_current_row(args: Vec<CfmlValue>) -> CfmlResult {
    match args.get(0) {
        Some(CfmlValue::Query(q)) => Ok(CfmlValue::Int(q.current_row() as i64)),
        _ => Ok(CfmlValue::Int(0)),
    }
}

/// Internal helper emitted by the `<cfloop query>` / `loop query=` desugaring to
/// advance the query's 1-based cursor so `q.col` and `q.currentRow` read the
/// current row (the query itself stays a query — Lucee/ACF semantics). Not a
/// user-facing BIF. Returns Null; a non-query first arg is a no-op.
fn fn_query_move_cursor(args: Vec<CfmlValue>) -> CfmlResult {
    if let (Some(CfmlValue::Query(q)), Some(row)) = (args.get(0), args.get(1)) {
        let n = row.as_string().trim().parse::<i64>().unwrap_or(1).max(1) as usize;
        q.set_current_row(n);
    }
    Ok(CfmlValue::Null)
}

fn fn_value_list(args: Vec<CfmlValue>) -> CfmlResult {
    let arr = args.get(0).ok_or_else(|| CfmlError::runtime("valueList() requires a query column".to_string()))?;
    let delim = args.get(1).map(|v| v.as_string()).unwrap_or_else(|| ",".to_string());
    // valueList canonically iterates rows on Lucee — accept both Array and QueryColumn.
    if let Some(items) = arr.as_array_or_query_column() {
        let result: Vec<String> = items.iter().map(|v| v.as_string()).collect();
        Ok(CfmlValue::string(result.join(&delim)))
    } else {
        Ok(CfmlValue::string(arr.as_string()))
    }
}

fn fn_value_array(args: Vec<CfmlValue>) -> CfmlResult {
    // Lucee valueArray has two forms:
    //   valueArray(query, columnName) — array of that column's values
    //   valueArray(query.column)      — the dot-access already yields a column
    //                                   (Array/QueryColumn); return its values.
    let arg = args.get(0).ok_or_else(|| {
        CfmlError::runtime("valueArray() requires a query column or a query + column name".to_string())
    })?;
    if let CfmlValue::Query(q) = arg {
        let col = get_str(&args, 1);
        let values = q.with_read(|d| {
            d.column_data_ci(&col)
                .cloned()
                .unwrap_or_else(|| vec![CfmlValue::string(String::new()); d.row_count()])
        });
        return Ok(CfmlValue::array(values));
    }
    if let Some(items) = arg.as_array_or_query_column() {
        return Ok(CfmlValue::array(items));
    }
    Ok(CfmlValue::array(vec![arg.clone()]))
}

fn fn_quoted_value_list(args: Vec<CfmlValue>) -> CfmlResult {
    let arr = args.get(0).ok_or_else(|| CfmlError::runtime("quotedValueList() requires a query column".to_string()))?;
    let delim = args.get(1).map(|v| v.as_string()).unwrap_or_else(|| ",".to_string());
    if let Some(items) = arr.as_array_or_query_column() {
        let result: Vec<String> = items.iter().map(|v| format!("'{}'", v.as_string())).collect();
        Ok(CfmlValue::string(result.join(&delim)))
    } else {
        Ok(CfmlValue::string(format!("'{}'", arr.as_string())))
    }
}

// ---- List functions ----

fn fn_list_avg(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let delim = get_delimiter(&args, 1);
    let items: Vec<&str> = cfml_list_split(&list, &delim);
    if items.is_empty() {
        return Ok(CfmlValue::Int(0));
    }
    let sum: f64 = items.iter()
        .map(|s| s.trim().parse::<f64>().unwrap_or(0.0))
        .sum();
    Ok(CfmlValue::Double(sum / items.len() as f64))
}

fn fn_list_item_trim(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let delim = get_delimiter(&args, 1);
    let items: Vec<&str> = cfml_list_split(&list, &delim);
    let trimmed: Vec<String> = items.iter().map(|s| s.trim().to_string()).collect();
    Ok(CfmlValue::string(trimmed.join(&delim)))
}

fn fn_list_index_exists(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let idx = get_int(&args, 1) as usize;
    let delim = get_delimiter(&args, 2);
    let items: Vec<&str> = cfml_list_split(&list, &delim);
    Ok(CfmlValue::Bool(idx >= 1 && idx <= items.len()))
}

// ---- System functions ----

fn fn_get_file_from_path(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    // Lucee/ACF treat BOTH `/` and `\` as path separators regardless of host
    // OS, so the filename is the segment after the last separator of either
    // kind. (Using std::path::Path::file_name alone misses `\` on Unix, leaving
    // a Windows-style traversal string like `..\..\sam` intact — a path
    // sanitisation gap.)
    let file_name = path
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or("")
        .to_string();
    Ok(CfmlValue::string(file_name))
}

fn fn_get_canonical_path(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    match std::fs::canonicalize(&path) {
        Ok(p) => Ok(CfmlValue::string(p.to_string_lossy().to_string())),
        Err(_) => Ok(CfmlValue::string(path)),
    }
}

fn fn_system_output(args: Vec<CfmlValue>) -> CfmlResult {
    let msg = get_str(&args, 0);
    let add_newline = args.get(1).map(|v| match v {
        CfmlValue::Bool(b) => *b,
        _ => true,
    }).unwrap_or(true);
    if add_newline {
        eprintln!("{}", msg);
    } else {
        eprint!("{}", msg);
    }
    Ok(CfmlValue::Null)
}

/// Lucee `SystemCacheClear([cacheName])` — flushes an engine-level cache
/// (template/page, object, query, …). RustCFML has no Lucee-style template
/// page cache to invalidate (serve mode rebuilds the bytecode cache on
/// fwreinit anyway), so this is a no-op that exists so framework reload paths
/// — e.g. Preside's `Bootstrap._clearExistingApplication` — resolve it.
fn fn_system_cache_clear(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Null)
}

fn fn_get_environment_variable(args: Vec<CfmlValue>) -> CfmlResult {
    let name = get_str(&args, 0);
    match std::env::var(&name) {
        Ok(val) => Ok(CfmlValue::string(val)),
        Err(_) => Ok(CfmlValue::string(String::new())),
    }
}

fn fn_read_line(args: Vec<CfmlValue>) -> CfmlResult {
    // Optional prompt argument
    if !args.is_empty() {
        let prompt = args[0].as_string();
        eprint!("{}", prompt);
    }
    use std::io::{self, BufRead};
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).unwrap_or(0);
    // Strip trailing newline
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    Ok(CfmlValue::string(line))
}

/// `writeLog(text, type, application, file, log)`.
///
/// The VM intercepts this name so it can supply the `Context` / `Application`
/// columns from the live request; this implementation is the fallback for
/// embedders that dispatch builtins without a VM. It writes through the same
/// appenders, just with those two columns empty.
fn fn_write_log(args: Vec<CfmlValue>) -> CfmlResult {
    use cfml_common::logging;
    let text = get_str(&args, 0);
    let log_type = {
        let t = get_str(&args, 1);
        if t.is_empty() { "Information".to_string() } else { t }
    };
    let level = logging::parse_type_attr(&log_type).ok_or_else(|| {
        CfmlError::runtime(format!(
            "Invalid value for attribute type [{}]",
            log_type.to_lowercase()
        ))
    })?;
    let file = get_str(&args, 3);
    let log = get_str(&args, 4);
    // Without a VM there is no configured-logger registry to consult, so an
    // unknown `log=` name can't be distinguished — treat it as Lucee's default.
    let name = if !file.trim().is_empty() {
        file.trim().to_string()
    } else if !log.trim().is_empty() {
        log.trim().to_string()
    } else {
        "application".to_string()
    };
    logging::write_entry(&name, level, "", "", &text).map_err(CfmlError::runtime)?;
    Ok(CfmlValue::Null)
}

/// Convert CFML locale name (friendly or code) to Java locale code (e.g. "en_US").
/// Matches Lucee's behavior: setLocale("English (US)") returns "en_US".
fn cfml_locale_to_code(name: &str) -> String {
    let trimmed = name.trim();
    let lower = trimmed.to_lowercase();
    match lower.as_str() {
        "english (us)" | "english (united states)" | "en_us" | "en-us" => "en_US".to_string(),
        "english (uk)" | "english (united kingdom)" | "en_gb" | "en-gb" => "en_GB".to_string(),
        "english (australian)" | "en_au" | "en-au" => "en_AU".to_string(),
        "english (canadian)" | "en_ca" | "en-ca" => "en_CA".to_string(),
        "german (standard)" | "german" | "de_de" | "de-de" => "de_DE".to_string(),
        "french (standard)" | "french" | "fr_fr" | "fr-fr" => "fr_FR".to_string(),
        "spanish (standard)" | "spanish" | "es_es" | "es-es" => "es_ES".to_string(),
        "italian (standard)" | "italian" | "it_it" | "it-it" => "it_IT".to_string(),
        "portuguese (standard)" | "portuguese" | "pt_pt" | "pt-pt" => "pt_PT".to_string(),
        "dutch (standard)" | "dutch" | "nl_nl" | "nl-nl" => "nl_NL".to_string(),
        "japanese" | "ja_jp" | "ja-jp" => "ja_JP".to_string(),
        "chinese (china)" | "zh_cn" | "zh-cn" => "zh_CN".to_string(),
        // If already looks like a Java locale code (xx_XX), keep as-is
        _ => {
            if trimmed.contains('_') || trimmed.contains('-') {
                trimmed.replace('-', "_")
            } else {
                trimmed.to_string()
            }
        }
    }
}

/// Convert Java locale code back to Lucee's friendly lowercase name.
/// e.g. "en_US" -> "english (us)"
fn locale_code_to_friendly(code: &str) -> String {
    match code {
        "en_US" => "english (us)".to_string(),
        "en_GB" => "english (uk)".to_string(),
        "en_AU" => "english (australian)".to_string(),
        "en_CA" => "english (canadian)".to_string(),
        "de_DE" => "german (standard)".to_string(),
        "fr_FR" => "french (standard)".to_string(),
        "es_ES" => "spanish (standard)".to_string(),
        "it_IT" => "italian (standard)".to_string(),
        "pt_PT" => "portuguese (standard)".to_string(),
        "nl_NL" => "dutch (standard)".to_string(),
        "ja_JP" => "japanese".to_string(),
        "zh_CN" => "chinese (china)".to_string(),
        _ => code.to_lowercase(),
    }
}

// GH #304: both of these were inert — setLocale() computed a code and discarded
// it, getLocale() answered a hardcoded "english (us)". They are VM-intercepted now
// (the VM owns `locale` as request state, alongside `timezone`), but these
// off-VM implementations must agree rather than lie: they read and write the same
// thread-local the ls* formatters consult.
fn fn_set_locale(args: Vec<CfmlValue>) -> CfmlResult {
    let requested = get_str(&args, 0);
    let code = cfml_common::locale::canonical_locale(&requested).ok_or_else(|| {
        CfmlError::runtime(format!("setLocale(): unknown locale [{}].", requested))
    })?;
    let previous = cfml_common::locale::current_locale();
    cfml_common::locale::set_current_locale(&code);
    // Lucee returns the PREVIOUS locale (in CODE form), which is what makes
    // save-and-restore work.
    Ok(CfmlValue::string(previous))
}

fn fn_get_locale(_args: Vec<CfmlValue>) -> CfmlResult {
    // Lucee returns the lowercase friendly name by default.
    Ok(CfmlValue::string(cfml_common::locale::friendly_name(
        &cfml_common::locale::current_locale(),
    )))
}

fn fn_set_time_zone(_args: Vec<CfmlValue>) -> CfmlResult {
    // VM-intercepted (needs VM state to set the request timezone). This stub is
    // only reached if the intercept is bypassed.
    Err(CfmlError::runtime("setTimeZone() requires VM context".to_string()))
}

fn fn_set_encoding(_args: Vec<CfmlValue>) -> CfmlResult {
    // VM-intercepted (mutates the url/form scope structs in globals). This stub
    // is only reached if the intercept is bypassed — return void like the BIF.
    Ok(CfmlValue::Null)
}

fn fn_get_time_zone_info(_args: Vec<CfmlValue>) -> CfmlResult {
    // VM-intercepted (resolves the current request timezone via chrono-tz).
    Err(CfmlError::runtime("getTimeZoneInfo() requires VM context".to_string()))
}

fn fn_application_stop(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Null)
}

fn fn_get_application_metadata(_args: Vec<CfmlValue>) -> CfmlResult {
    let mut meta = ValueMap::default();
    meta.insert("name".to_string(), CfmlValue::string(String::new()));
    Ok(CfmlValue::strukt(meta))
}

fn fn_trace(args: Vec<CfmlValue>) -> CfmlResult {
    let text = get_str(&args, 0);
    eprintln!("[TRACE] {}", text);
    Ok(CfmlValue::Null)
}

/// Fallback `getDebugData()` for feature-off builds — empty struct.
fn fn_get_debug_data_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::strukt(ValueMap::default()))
}

/// Fallback `isDebugMode()` for feature-off builds — always false.
fn fn_is_debug_mode_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Bool(false))
}

/// Fallback `debugAdd()` for feature-off builds — no-op.
fn fn_debug_add_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Null)
}

/// Fallback `getRequestProfile()` for feature-off builds — empty struct.
fn fn_get_request_profile_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::strukt(ValueMap::default()))
}

/// Fallback `profileNow()` for feature-off builds — profiler unavailable.
fn fn_profile_now_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Bool(false))
}

// ---- File functions ----

fn fn_file_read_binary(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    match std::fs::read(&path) {
        Ok(bytes) => Ok(CfmlValue::Binary(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(CfmlError::file_not_found(format!("The file [{}] does not exist", path)))
        }
        Err(e) => Err(CfmlError::runtime(format!("fileReadBinary(): {}", e))),
    }
}

fn fn_file_get_mime_type(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    let ext = std::path::Path::new(&path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mime = match ext.as_str() {
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "gzip" => "application/gzip",
        "tar" => "application/x-tar",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "wav" => "audio/wav",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    };
    Ok(CfmlValue::string(mime.to_string()))
}

fn fn_directory_rename(args: Vec<CfmlValue>) -> CfmlResult {
    let old_path = get_str(&args, 0);
    let new_path = get_str(&args, 1);
    match std::fs::rename(&old_path, &new_path) {
        Ok(_) => Ok(CfmlValue::Null),
        Err(e) => Err(CfmlError::runtime(format!("directoryRename(): {}", e))),
    }
}

fn fn_directory_copy(args: Vec<CfmlValue>) -> CfmlResult {
    let src = get_str(&args, 0);
    let dst = get_str(&args, 1);
    fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            let dest_path = dst.join(entry.file_name());
            if ty.is_dir() {
                copy_dir_recursive(&entry.path(), &dest_path)?;
            } else {
                std::fs::copy(entry.path(), &dest_path)?;
            }
        }
        Ok(())
    }
    match copy_dir_recursive(std::path::Path::new(&src), std::path::Path::new(&dst)) {
        Ok(_) => Ok(CfmlValue::Null),
        Err(e) => Err(CfmlError::runtime(format!("directoryCopy(): {}", e))),
    }
}

fn fn_file_open(args: Vec<CfmlValue>) -> CfmlResult {
    // Returns the file path as a handle identifier (actual file handle management would need VM support)
    let path = get_str(&args, 0);
    let _mode = if args.len() > 1 { get_str(&args, 1) } else { "read".to_string() };
    // Return a struct representing the file handle
    let mut handle = ValueMap::default();
    handle.insert("path".to_string(), CfmlValue::string(path));
    handle.insert("isOpen".to_string(), CfmlValue::Bool(true));
    handle.insert("line".to_string(), CfmlValue::Int(0));
    Ok(CfmlValue::strukt(handle))
}

fn fn_file_close(_args: Vec<CfmlValue>) -> CfmlResult {
    // Stub - actual file handle management needs VM support
    Ok(CfmlValue::Null)
}

fn fn_file_read_line(args: Vec<CfmlValue>) -> CfmlResult {
    // Simplified: reads the Nth line from the file indicated by the handle
    match args.get(0) {
        Some(CfmlValue::Struct(handle)) => {
            let path = handle.get("path").map(|v| v.as_string()).unwrap_or_default();
            let line_num = handle.get("line").map(|v| match v {
                CfmlValue::Int(i) => i as usize,
                _ => 0,
            }).unwrap_or(0);
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let lines: Vec<&str> = content.lines().collect();
                    if line_num < lines.len() {
                        Ok(CfmlValue::string(lines[line_num].to_string()))
                    } else {
                        Ok(CfmlValue::string(String::new()))
                    }
                }
                Err(e) => Err(CfmlError::runtime(format!("fileReadLine(): {}", e))),
            }
        }
        _ => Err(CfmlError::runtime("fileReadLine() requires a file handle".to_string())),
    }
}

fn fn_file_write_line(args: Vec<CfmlValue>) -> CfmlResult {
    match args.get(0) {
        Some(CfmlValue::Struct(handle)) => {
            let path = handle.get("path").map(|v| v.as_string()).unwrap_or_default();
            let data = get_str(&args, 1);
            use std::io::Write;
            match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                Ok(mut f) => {
                    writeln!(f, "{}", data).map_err(|e| CfmlError::runtime(format!("fileWriteLine(): {}", e)))?;
                    Ok(CfmlValue::Null)
                }
                Err(e) => Err(CfmlError::runtime(format!("fileWriteLine(): {}", e))),
            }
        }
        _ => Err(CfmlError::runtime("fileWriteLine() requires a file handle".to_string())),
    }
}

fn fn_file_is_eof(args: Vec<CfmlValue>) -> CfmlResult {
    match args.get(0) {
        Some(CfmlValue::Struct(handle)) => {
            let path = handle.get("path").map(|v| v.as_string()).unwrap_or_default();
            let line_num = handle.get("line").map(|v| match v {
                CfmlValue::Int(i) => i as usize,
                _ => 0,
            }).unwrap_or(0);
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let line_count = content.lines().count();
                    Ok(CfmlValue::Bool(line_num >= line_count))
                }
                Err(_) => Ok(CfmlValue::Bool(true)),
            }
        }
        _ => Ok(CfmlValue::Bool(true)),
    }
}

// ---- VM Stub functions for tag infrastructure ----

fn fn_cflog_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("cflog requires VM-level support and was not intercepted.".to_string()))
}

fn fn_cfparam_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__cfparam requires VM-level support and was not intercepted.".to_string()))
}

fn fn_cfsetting_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("cfsetting requires VM-level support and was not intercepted.".to_string()))
}

fn fn_cflock_start_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("cflock requires VM-level support and was not intercepted.".to_string()))
}

fn fn_cflock_end_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("cflock requires VM-level support and was not intercepted.".to_string()))
}

fn fn_cfcookie_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("cfcookie requires VM-level support and was not intercepted.".to_string()))
}

// ---- File Upload functions ----

/// fileUpload(destination, formField, accept, nameConflict)
fn fn_file_upload(args: Vec<CfmlValue>) -> CfmlResult {
    let destination = get_str(&args, 0);
    let _form_field = get_str(&args, 1);
    let _accept = if args.len() > 2 { get_str(&args, 2) } else { String::new() };
    let name_conflict = if args.len() > 3 { get_str(&args, 3).to_lowercase() } else { "error".to_string() };

    // This is a stub — real implementation requires VM access to the form scope
    // to find the uploaded file's temp path. The VM intercepts this.
    let mut result = ValueMap::default();
    result.insert("serverDirectory".to_string(), CfmlValue::string(destination));
    result.insert("nameConflict".to_string(), CfmlValue::string(name_conflict));
    result.insert("fileWasSaved".to_string(), CfmlValue::Bool(false));
    Ok(CfmlValue::strukt(result))
}

/// fileUploadAll(destination, accept, nameConflict)
fn fn_file_upload_all(args: Vec<CfmlValue>) -> CfmlResult {
    let destination = get_str(&args, 0);
    let _accept = if args.len() > 1 { get_str(&args, 1) } else { String::new() };
    let _name_conflict = if args.len() > 2 { get_str(&args, 2).to_lowercase() } else { "error".to_string() };

    let mut result = ValueMap::default();
    result.insert("serverDirectory".to_string(), CfmlValue::string(destination));
    result.insert("fileWasSaved".to_string(), CfmlValue::Bool(false));
    Ok(CfmlValue::strukt(result))
}

/// __cffile_upload(destination, formField, accept, nameConflict) - generated by <cffile action="upload">
fn fn_cffile_upload(args: Vec<CfmlValue>) -> CfmlResult {
    fn_file_upload(args)
}

fn fn_cfcache_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("cfcache requires VM-level support and was not intercepted.".to_string()))
}

fn fn_cfloop_file_lines_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__cfloop_file_lines requires VM intercept".into()))
}

fn fn_cfexecute_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("__cfexecute requires VM intercept".into()))
}

// Without `smtp`, the early `return Err` makes the rest of the body
// unreachable by design — the code must still compile so the smtp build
// stays warning-free, so allow the lint rather than cfg-ing out the tail.
#[cfg_attr(not(feature = "smtp"), allow(unreachable_code))]
fn fn_cfmail(args: Vec<CfmlValue>) -> CfmlResult {
    let opts = match args.into_iter().next() {
        Some(CfmlValue::Struct(s)) => s,
        _ => return Err(CfmlError::runtime("__cfmail requires a struct argument".into())),
    };

    let get_opt = |key: &str| -> Option<String> {
        opts.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_string())
    };

    let to = get_opt("to").unwrap_or_default();
    let from = get_opt("from").unwrap_or_default();
    let subject = get_opt("subject").unwrap_or_default();
    let mail_type = get_opt("type").unwrap_or_else(|| "text".into());
    let body_text = get_opt("body").unwrap_or_default();
    #[cfg(feature = "smtp")]
    let cc = get_opt("cc");
    #[cfg(feature = "smtp")]
    let bcc = get_opt("bcc");
    #[cfg(feature = "smtp")]
    let cfg_default = default_mail_server();
    #[cfg(feature = "smtp")]
    let server = get_opt("server").or_else(|| {
        cfg_default
            .as_ref()
            .map(|d| d.server.clone())
            .filter(|s| !s.is_empty())
    });
    #[cfg(feature = "smtp")]
    let port_str = get_opt("port").or_else(|| {
        cfg_default
            .as_ref()
            .filter(|d| d.port != 0)
            .map(|d| d.port.to_string())
    });
    #[cfg(feature = "smtp")]
    let username = get_opt("username").or_else(|| {
        cfg_default
            .as_ref()
            .map(|d| d.username.clone())
            .filter(|s| !s.is_empty())
    });
    #[cfg(feature = "smtp")]
    let password = get_opt("password").or_else(|| {
        cfg_default
            .as_ref()
            .map(|d| d.password.clone())
            .filter(|s| !s.is_empty())
    });
    // <cfmail useSSL/useTLS>, falling back to the configured mailServers[] entry.
    // Both were parsed and then dropped on the floor: the transport was always
    // built with `builder_dangerous`, so a server configured with `"tls": true`
    // still sent AUTH credentials over an unencrypted connection.
    #[cfg(feature = "smtp")]
    let as_bool = |s: &str| s.eq_ignore_ascii_case("true") || s == "1" || s.eq_ignore_ascii_case("yes");
    #[cfg(feature = "smtp")]
    let use_ssl = get_opt("usessl")
        .map(|v| as_bool(&v))
        .unwrap_or_else(|| cfg_default.as_ref().map(|d| d.ssl).unwrap_or(false));
    #[cfg(feature = "smtp")]
    let use_tls = get_opt("usetls")
        .map(|v| as_bool(&v))
        .unwrap_or_else(|| cfg_default.as_ref().map(|d| d.tls).unwrap_or(false));

    // Log to stderr for debugging visibility
    eprintln!("[CFMAIL] To: {} | From: {} | Subject: {} | Type: {}", to, from, subject, mail_type);
    if !body_text.is_empty() {
        eprintln!("[CFMAIL] Body: {}", body_text);
    }

    // Lucee/ACF require a server to be configured. Fail loudly rather than
    // silently dropping mail — silent success would mislead developers into
    // thinking mail was sent.
    #[cfg(feature = "smtp")]
    {
        if server.is_none() {
            return Err(CfmlError::runtime(
                "no SMTP Server defined. Set 'server' attribute on cfmail or configure a default mail server.".to_string()
            ));
        }
    }
    #[cfg(not(feature = "smtp"))]
    {
        return Err(CfmlError::runtime(
            "cfmail requires the 'smtp' feature to be enabled in this build".to_string()
        ));
    }

    // Collect multipart bodies declared via cfmailpart (text + html
    // alternatives). Each part carries a `type` ("text"/"html") and a captured
    // `body`; a part may also wrap its attributes in an `attributeCollection`
    // struct (the form Wheels' Global.cfc $mail() emits), so look there too.
    #[cfg(feature = "smtp")]
    let mail_parts: Vec<(String, String)> = if let Some((_, CfmlValue::Array(parts))) =
        opts.iter().find(|(k, _)| k.eq_ignore_ascii_case("parts"))
    {
        parts.iter().filter_map(|p| {
            if let CfmlValue::Struct(ps) = p {
                let lookup = |key: &str| -> Option<String> {
                    ps.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))
                        .map(|(_, v)| v.as_string())
                        .or_else(|| ps.iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("attributeCollection"))
                            .and_then(|(_, v)| if let CfmlValue::Struct(ac) = v {
                                ac.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))
                                    .map(|(_, v)| v.as_string())
                            } else { None }))
                };
                Some((lookup("type").unwrap_or_else(|| "text".into()),
                      lookup("body").unwrap_or_default()))
            } else { None }
        }).collect()
    } else { Vec::new() };

    // Collect file attachments from params array
    let mut attachments: Vec<String> = Vec::new();
    if let Some((_, CfmlValue::Array(params))) = opts.iter().find(|(k, _)| k.eq_ignore_ascii_case("params")) {
        for param in params.iter() {
            if let CfmlValue::Struct(p) = param {
                if let Some(file_path) = p.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("file"))
                    .map(|(_, v)| v.as_string())
                {
                    if !file_path.is_empty() {
                        attachments.push(file_path);
                    }
                }
            }
        }
    }

    // If server is provided, attempt real SMTP delivery
    #[cfg(feature = "smtp")]
    if let Some(ref smtp_server) = server {
        use lettre::{SmtpTransport, Transport};
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::message::{header::ContentType, Message, MultiPart, SinglePart, Attachment};

        let mut email_builder = Message::builder()
            .subject(&subject);

        // Parse from address
        match from.parse::<lettre::message::Mailbox>() {
            Ok(mbox) => { email_builder = email_builder.from(mbox); }
            Err(e) => return Err(CfmlError::runtime(format!("cfmail: invalid from address '{}': {}", from, e))),
        }

        // Parse to addresses (comma-separated)
        for addr in to.split(',') {
            let addr = addr.trim();
            if !addr.is_empty() {
                match addr.parse::<lettre::message::Mailbox>() {
                    Ok(mbox) => { email_builder = email_builder.to(mbox); }
                    Err(e) => return Err(CfmlError::runtime(format!("cfmail: invalid to address '{}': {}", addr, e))),
                }
            }
        }

        // CC
        if let Some(ref cc_addrs) = cc {
            for addr in cc_addrs.split(',') {
                let addr = addr.trim();
                if !addr.is_empty() {
                    if let Ok(mbox) = addr.parse::<lettre::message::Mailbox>() {
                        email_builder = email_builder.cc(mbox);
                    }
                }
            }
        }

        // BCC
        if let Some(ref bcc_addrs) = bcc {
            for addr in bcc_addrs.split(',') {
                let addr = addr.trim();
                if !addr.is_empty() {
                    if let Ok(mbox) = addr.parse::<lettre::message::Mailbox>() {
                        email_builder = email_builder.bcc(mbox);
                    }
                }
            }
        }

        // Build a multipart/alternative from cfmailpart parts when present.
        let alternative = if mail_parts.is_empty() {
            None
        } else {
            let mut alt = MultiPart::alternative().build();
            for (ptype, pbody) in &mail_parts {
                let ct = if ptype.eq_ignore_ascii_case("html")
                    || ptype.eq_ignore_ascii_case("text/html")
                {
                    ContentType::TEXT_HTML
                } else {
                    ContentType::TEXT_PLAIN
                };
                alt = alt.singlepart(SinglePart::builder().header(ct).body(pbody.clone()));
            }
            Some(alt)
        };

        let email = if let Some(alternative) = alternative {
            // Multipart message: cfmailpart-declared text/html alternatives,
            // optionally wrapped in a mixed part alongside file attachments.
            if attachments.is_empty() {
                email_builder
                    .multipart(alternative)
                    .map_err(|e| CfmlError::runtime(format!("cfmail: failed to build email: {}", e)))?
            } else {
                let mut multipart = MultiPart::mixed().multipart(alternative);
                for file_path in &attachments {
                    let path = std::path::Path::new(file_path);
                    let filename = path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "attachment".to_string());
                    match std::fs::read(path) {
                        Ok(data) => {
                            let attachment = Attachment::new(filename)
                                .body(data, ContentType::parse("application/octet-stream").unwrap());
                            multipart = multipart.singlepart(attachment);
                            eprintln!("[CFMAIL] Attachment: {}", file_path);
                        }
                        Err(e) => {
                            eprintln!("[CFMAIL] Warning: could not read attachment '{}': {}", file_path, e);
                        }
                    }
                }
                email_builder
                    .multipart(multipart)
                    .map_err(|e| CfmlError::runtime(format!("cfmail: failed to build email: {}", e)))?
            }
        } else if attachments.is_empty() {
            // Simple message (no attachments)
            let content_type = if mail_type.eq_ignore_ascii_case("html") {
                ContentType::TEXT_HTML
            } else {
                ContentType::TEXT_PLAIN
            };
            email_builder
                .header(content_type)
                .body(body_text.clone())
                .map_err(|e| CfmlError::runtime(format!("cfmail: failed to build email: {}", e)))?
        } else {
            // Multipart message with attachments
            let body_part = if mail_type.eq_ignore_ascii_case("html") {
                SinglePart::builder()
                    .header(ContentType::TEXT_HTML)
                    .body(body_text.clone())
            } else {
                SinglePart::builder()
                    .header(ContentType::TEXT_PLAIN)
                    .body(body_text.clone())
            };

            let mut multipart = MultiPart::mixed().singlepart(body_part);

            for file_path in &attachments {
                let path = std::path::Path::new(file_path);
                let filename = path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "attachment".to_string());
                match std::fs::read(path) {
                    Ok(data) => {
                        let attachment = Attachment::new(filename)
                            .body(data, ContentType::parse("application/octet-stream").unwrap());
                        multipart = multipart.singlepart(attachment);
                        eprintln!("[CFMAIL] Attachment: {}", file_path);
                    }
                    Err(e) => {
                        eprintln!("[CFMAIL] Warning: could not read attachment '{}': {}", file_path, e);
                    }
                }
            }

            email_builder
                .multipart(multipart)
                .map_err(|e| CfmlError::runtime(format!("cfmail: failed to build email: {}", e)))?
        };

        let port: u16 = port_str.as_deref()
            .and_then(|p| p.parse().ok())
            .unwrap_or(25);

        // Encryption. `useSSL` = implicit TLS (SMTPS, the whole connection is
        // wrapped, conventionally port 465). `useTLS` = STARTTLS (plaintext
        // connect, then upgrade, conventionally port 587). Neither was applied
        // before — every message went over `builder_dangerous`, so AUTH LOGIN
        // credentials were readable on the wire even with tls/ssl configured.
        // If TLS was explicitly asked for and cannot be established, fail: a
        // silent downgrade to plaintext is the exact bug being fixed here.
        let mut transport_builder = if use_ssl {
            SmtpTransport::relay(smtp_server).map_err(|e| {
                CfmlError::runtime(format!(
                    "cfmail: could not establish an implicit-TLS (useSSL) connection to {}: {}",
                    smtp_server, e
                ))
            })?
        } else if use_tls {
            SmtpTransport::starttls_relay(smtp_server).map_err(|e| {
                CfmlError::runtime(format!(
                    "cfmail: could not establish a STARTTLS (useTLS) connection to {}: {}",
                    smtp_server, e
                ))
            })?
        } else {
            SmtpTransport::builder_dangerous(smtp_server)
        }
        .port(port);

        if let (Some(ref user), Some(ref pass)) = (&username, &password) {
            transport_builder = transport_builder.credentials(
                Credentials::new(user.clone(), pass.clone())
            );
        }

        let mailer = transport_builder.build();
        match mailer.send(&email) {
            Ok(_) => eprintln!(
                "[CFMAIL] Sent successfully via {} ({})",
                smtp_server,
                if use_ssl {
                    "implicit TLS"
                } else if use_tls {
                    "STARTTLS"
                } else {
                    "no encryption"
                }
            ),
            Err(e) => return Err(CfmlError::runtime(format!("cfmail: SMTP send failed: {}", e))),
        }
    }

    Ok(CfmlValue::Null)
}

/// Stub for cache functions — VM intercepts these
fn fn_cache_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("Cache function requires VM-level support and was not intercepted.".to_string()))
}

/// Stub for session/auth functions — VM intercepts these
fn fn_session_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("Session/auth function requires VM-level support and was not intercepted.".to_string()))
}

/// Stub for cfthread functions — VM intercepts these
fn fn_cfthread_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("cfthread function requires VM-level support and was not intercepted.".to_string()))
}

/// Stub for async-kernel functions (runAsync, _schedule) — VM intercepts these
fn fn_async_stub(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("Async function requires VM-level support and was not intercepted.".to_string()))
}

// ===============================================
// STRUCT METADATA FUNCTIONS
// ===============================================

fn fn_struct_get_metadata(args: Vec<CfmlValue>) -> CfmlResult {
    if args.is_empty() {
        return Err(CfmlError::runtime("structGetMetadata requires a struct argument".to_string()));
    }
    // All RustCFML structs are unordered and case-insensitive
    let mut meta = ValueMap::default();
    meta.insert("ordered".to_string(), CfmlValue::Bool(false));
    meta.insert("casesensitive".to_string(), CfmlValue::Bool(false));
    Ok(CfmlValue::strukt(meta))
}

fn fn_struct_set_metadata(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("structSetMetadata() is not implemented. Ordered/case-sensitive struct metadata is not yet supported.".to_string()))
}

// ===============================================
// FILE ATTRIBUTE FUNCTIONS
// ===============================================

fn fn_file_set_access_mode(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    let mode_str = get_str(&args, 1);
    if path.is_empty() || mode_str.is_empty() {
        return Err(CfmlError::runtime("fileSetAccessMode requires path and mode arguments".to_string()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = u32::from_str_radix(&mode_str, 8)
            .map_err(|_| CfmlError::runtime(format!("Invalid mode '{}': expected octal like '644'", mode_str)))?;
        let perms = std::fs::Permissions::from_mode(mode);
        std::fs::set_permissions(&path, perms)
            .map_err(|e| CfmlError::runtime(format!("fileSetAccessMode failed: {}", e)))?;
    }
    #[cfg(not(unix))]
    {
        let _ = mode_str;
        // On non-Unix platforms, this is a no-op
    }
    Ok(CfmlValue::Null)
}

fn fn_file_set_attribute(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    let attribute = get_str(&args, 1).to_lowercase();
    if path.is_empty() {
        return Err(CfmlError::runtime("fileSetAttribute requires path and attribute arguments".to_string()));
    }
    let metadata = std::fs::metadata(&path)
        .map_err(|e| CfmlError::runtime(format!("fileSetAttribute failed: {}", e)))?;
    let mut perms = metadata.permissions();
    match attribute.as_str() {
        "readonly" => perms.set_readonly(true),
        "normal" => perms.set_readonly(false),
        _ => {} // Ignore unsupported attributes
    }
    std::fs::set_permissions(&path, perms)
        .map_err(|e| CfmlError::runtime(format!("fileSetAttribute failed: {}", e)))?;
    Ok(CfmlValue::Null)
}

fn fn_file_set_last_modified(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    if path.is_empty() {
        return Err(CfmlError::runtime("fileSetLastModified requires a path argument".to_string()));
    }
    // Parse the date argument — try to parse as date string, or use current time
    let modified_time = if let Some(date_val) = args.get(1) {
        let date_str = date_val.as_string();
        if let Some(dt) = parse_cfml_date(&date_str) {
            let secs = dt.and_utc().timestamp();
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64)
        } else {
            cfml_common::clock::now_system_time()
        }
    } else {
        cfml_common::clock::now_system_time()
    };
    let file = std::fs::File::open(&path)
        .map_err(|e| CfmlError::runtime(format!("fileSetLastModified failed: {}", e)))?;
    file.set_modified(modified_time)
        .map_err(|e| CfmlError::runtime(format!("fileSetLastModified failed: {}", e)))?;
    Ok(CfmlValue::Null)
}

// ============================================================
// Locale (ls*) Functions
// ============================================================

fn fn_ls_date_format(args: Vec<CfmlValue>) -> CfmlResult {
    let pass_args = if args.len() >= 2 {
        vec![args[0].clone(), args[1].clone()]
    } else {
        vec![args.first().cloned().unwrap_or(CfmlValue::string(String::new()))]
    };
    fn_date_format(pass_args)
}

fn fn_ls_time_format(args: Vec<CfmlValue>) -> CfmlResult {
    let pass_args = if args.len() >= 2 {
        vec![args[0].clone(), args[1].clone()]
    } else {
        vec![args.first().cloned().unwrap_or(CfmlValue::string(String::new()))]
    };
    fn_time_format(pass_args)
}

fn fn_ls_date_time_format(args: Vec<CfmlValue>) -> CfmlResult {
    let pass_args = if args.len() >= 2 {
        vec![args[0].clone(), args[1].clone()]
    } else {
        vec![args.first().cloned().unwrap_or(CfmlValue::string(String::new()))]
    };
    fn_date_time_format(pass_args)
}

/// Resolve the locale an `ls*` call should format in: the explicit argument at
/// `idx` when supplied, otherwise the request's active locale (GH #304).
///
/// An explicit locale that names nothing we recognise is an ERROR — silently
/// falling back to `en_US` is exactly the failure mode this function exists to
/// remove, since the caller would see plausible US-formatted output and never
/// learn their locale was dropped.
fn ls_locale_arg(args: &[CfmlValue], idx: usize) -> Result<String, CfmlError> {
    match args.get(idx) {
        Some(v) if !v.as_string().trim().is_empty() => {
            let requested = v.as_string();
            cfml_common::locale::canonical_locale(&requested).ok_or_else(|| {
                CfmlError::runtime(format!("Unknown locale [{}].", requested))
            })
        }
        _ => Ok(cfml_common::locale::current_locale()),
    }
}

/// Group the integer part of `digits` with `sep`, or leave it alone when the
/// locale does not group.
fn group_digits(digits: &str, sep: char) -> String {
    if sep == '\u{0}' {
        return digits.to_string();
    }
    let grouped = add_thousands_separator(digits);
    if sep == ',' {
        grouped
    } else {
        grouped.replace(',', &sep.to_string())
    }
}

fn fn_ls_currency_format(args: Vec<CfmlValue>) -> CfmlResult {
    let n = get_float(&args, 0);
    let currency_type = if args.len() > 1 && !get_str(&args, 1).trim().is_empty() {
        get_str(&args, 1).to_lowercase()
    } else {
        "local".to_string()
    };
    // Third argument is the locale — it used to be read by nobody, so
    // `lsCurrencyFormat(1234.5, "local", "de_DE")` returned "$1,234.50".
    let locale = ls_locale_arg(&args, 2)?;
    let fmt = cfml_common::locale::number_format_for(&locale);

    let negative = n < 0.0;
    // Round HALF-UP like Java's currency formatter. Rust's `{:.N}` rounds half to
    // EVEN, so a 0-decimal locale rendered ¥1234.5 as ¥1,234 where Lucee gives
    // ¥1,235.
    let scale = 10f64.powi(fmt.currency_decimals as i32);
    let rounded = (n.abs() * scale + 0.5).floor() / scale;
    let formatted_num = format!("{:.*}", fmt.currency_decimals, rounded);
    let (int_part, frac_part) = match formatted_num.split_once('.') {
        Some((i, f)) => (i, f),
        None => (formatted_num.as_str(), ""),
    };
    let mut amount = group_digits(int_part, fmt.grouping);
    if !frac_part.is_empty() {
        amount.push(fmt.decimal);
        amount.push_str(frac_part);
    }

    // Negative handling differs by form, and the difference is not cosmetic —
    // verified byte-for-byte against Lucee 7.0.4:
    //   local          -> the parens wrap symbol AND amount:  ($50.00) / (50,00 €)
    //   international  -> the ISO code stays OUTSIDE:         USD (50.00)
    //   none           -> a leading minus:                    -50.00
    // Separators are plain ASCII spaces, not NBSP, in every form.
    Ok(CfmlValue::string(match currency_type.as_str() {
        // The ISO code always LEADS, even for locales that trail their symbol
        // (de_DE renders "1.234,50 €" but "EUR 1.234,50").
        "international" => {
            if negative {
                format!("{} ({})", fmt.currency_code, amount)
            } else {
                format!("{} {}", fmt.currency_code, amount)
            }
        }
        "none" => {
            if negative {
                format!("-{}", amount)
            } else {
                amount
            }
        }
        _ => {
            let gap = if fmt.space_before_symbol { " " } else { "" };
            let body = if fmt.symbol_after {
                format!("{}{}{}", amount, gap, fmt.currency_symbol)
            } else {
                format!("{}{}{}", fmt.currency_symbol, gap, amount)
            };
            if negative {
                format!("({})", body)
            } else {
                body
            }
        }
    }))
}

fn fn_ls_euro_currency_format(args: Vec<CfmlValue>) -> CfmlResult {
    // In Lucee, lsEuroCurrencyFormat uses the CURRENT locale's currency, not EUR.
    // In en_US locale, this returns USD formatting. Delegate to lsCurrencyFormat.
    fn_ls_currency_format(args)
}

fn fn_ls_is_date(args: Vec<CfmlValue>) -> CfmlResult {
    fn_is_date(vec![args.first().cloned().unwrap_or(CfmlValue::string(String::new()))])
}

fn fn_ls_is_numeric(args: Vec<CfmlValue>) -> CfmlResult {
    fn_is_numeric(vec![args.first().cloned().unwrap_or(CfmlValue::string(String::new()))])
}

fn fn_ls_is_currency(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0).trim().to_string();
    if s.is_empty() {
        return Ok(CfmlValue::Bool(false));
    }
    let stripped: String = s.chars()
        .filter(|c| *c != '$' && *c != '\u{20AC}' && *c != '\u{00A3}' && *c != '\u{00A5}'
            && *c != ',' && *c != ' ')
        .collect();
    let is_currency = !stripped.is_empty() && stripped.trim_start_matches('-').parse::<f64>().is_ok();
    let has_digit = s.chars().any(|c| c.is_ascii_digit());
    Ok(CfmlValue::Bool(is_currency && has_digit))
}

fn fn_ls_parse_currency(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0).trim().to_string();
    let stripped: String = s.chars()
        .filter(|c| *c != '$' && *c != '\u{20AC}' && *c != '\u{00A3}' && *c != '\u{00A5}'
            && *c != ',' && *c != ' ')
        .collect();
    let cleaned = if stripped.len() >= 3 {
        let prefix = &stripped[..3];
        if prefix.chars().all(|c| c.is_ascii_uppercase()) && stripped[3..].starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '.') {
            stripped[3..].to_string()
        } else {
            stripped
        }
    } else {
        stripped
    };
    match cleaned.parse::<f64>() {
        Ok(n) => Ok(CfmlValue::Double(n)),
        Err(_) => Err(CfmlError::runtime(format!("Cannot parse currency: {}", s))),
    }
}

fn fn_ls_parse_date_time(args: Vec<CfmlValue>) -> CfmlResult {
    fn_parse_date_time(vec![args.first().cloned().unwrap_or(CfmlValue::string(String::new()))])
}

fn fn_ls_number_format(args: Vec<CfmlValue>) -> CfmlResult {
    let pass_args = if args.len() >= 2 && !get_str(&args, 1).trim().is_empty() {
        vec![args[0].clone(), args[1].clone()]
    } else {
        vec![args.first().cloned().unwrap_or(CfmlValue::string(String::new()))]
    };
    let formatted = fn_number_format(pass_args)?.as_string();

    // GH #304: the locale argument (and the request locale) were dropped entirely,
    // so this was numberFormat() under another name. numberFormat produces en_US
    // punctuation, so re-punctuate for locales that differ. Swap through a
    // placeholder — a naive two-step replace would clobber the separators it had
    // just written when the locale's decimal is the other locale's grouping char
    // (de_DE: "1,234.5" -> "1.234,5").
    let locale = ls_locale_arg(&args, 2)?;
    let fmt = cfml_common::locale::number_format_for(&locale);
    if fmt.decimal == '.' && fmt.grouping == ',' {
        return Ok(CfmlValue::string(formatted));
    }
    let swapped: String = formatted
        .chars()
        .map(|c| match c {
            ',' => fmt.grouping,
            '.' => fmt.decimal,
            other => other,
        })
        .collect();
    Ok(CfmlValue::string(swapped))
}

fn fn_ls_week(args: Vec<CfmlValue>) -> CfmlResult {
    fn_week(vec![args.first().cloned().unwrap_or(CfmlValue::string(String::new()))])
}

fn fn_ls_day_of_week(args: Vec<CfmlValue>) -> CfmlResult {
    fn_day_of_week(vec![args.first().cloned().unwrap_or(CfmlValue::string(String::new()))])
}

// ---- Exception functions ----

fn fn_exception_key_exists(args: Vec<CfmlValue>) -> CfmlResult {
    if let (Some(CfmlValue::Struct(s)), Some(key)) = (args.get(0), args.get(1)) {
        let key_str = key.as_string().to_lowercase();
        let exists = s.keys().iter().any(|k| k.eq_ignore_ascii_case(&key_str));
        Ok(CfmlValue::Bool(exists))
    } else {
        Ok(CfmlValue::Bool(false))
    }
}

// ===============================================
// PASSWORD HASHING / CSRF FUNCTIONS
// ===============================================

/// generatePBKDFKey(algorithm, passphrase, salt, iterations, keySize)
/// Generates a derived key using PBKDF2.
/// algorithm: "PBKDF2WithHmacSHA1", "PBKDF2WithHmacSHA256", "PBKDF2WithHmacSHA512"
/// keySize is in bits (e.g. 128, 256). Returns base64-encoded derived key (matches Lucee).
#[cfg(feature = "security")]
fn fn_generate_pbkdf_key(args: Vec<CfmlValue>) -> CfmlResult {
    use pbkdf2::pbkdf2_hmac;
    use sha2::{Sha256, Sha512};
    use sha1::Sha1;

    if args.len() < 5 {
        return Err(CfmlError::runtime(
            "generatePBKDFKey requires 5 arguments: algorithm, passphrase, salt, iterations, keySize".to_string()
        ));
    }

    let algorithm = get_str(&args, 0).to_uppercase();
    let passphrase = get_str(&args, 1);
    let salt = get_str(&args, 2);
    let iterations = get_int(&args, 3) as u32;
    let key_size_bits = get_int(&args, 4) as usize;
    let key_size_bytes = key_size_bits / 8;

    if iterations == 0 {
        return Err(CfmlError::runtime("generatePBKDFKey: iterations must be greater than 0".to_string()));
    }
    if key_size_bytes == 0 {
        return Err(CfmlError::runtime("generatePBKDFKey: keySize must be greater than 0".to_string()));
    }

    let mut derived_key = vec![0u8; key_size_bytes];

    match algorithm.as_str() {
        "PBKDF2WITHHMACSHA1" | "PBKDF2WITHSHA1" => {
            pbkdf2_hmac::<Sha1>(
                passphrase.as_bytes(),
                salt.as_bytes(),
                iterations,
                &mut derived_key,
            );
        }
        "PBKDF2WITHHMACSHA256" | "PBKDF2WITHSHA256" => {
            pbkdf2_hmac::<Sha256>(
                passphrase.as_bytes(),
                salt.as_bytes(),
                iterations,
                &mut derived_key,
            );
        }
        "PBKDF2WITHHMACSHA512" | "PBKDF2WITHSHA512" => {
            pbkdf2_hmac::<Sha512>(
                passphrase.as_bytes(),
                salt.as_bytes(),
                iterations,
                &mut derived_key,
            );
        }
        _ => {
            return Err(CfmlError::runtime(format!(
                "generatePBKDFKey: unsupported algorithm '{}'. Supported: PBKDF2WithHmacSHA1, PBKDF2WithHmacSHA256, PBKDF2WithHmacSHA512",
                algorithm
            )));
        }
    }

    Ok(CfmlValue::string(base64_encode_bytes(&derived_key)))
}

/// generateBCryptHash(password [, rounds])
/// Generate a bcrypt hash. Rounds default to 10. Returns the bcrypt hash string.
#[cfg(feature = "security")]
fn fn_generate_bcrypt_hash(args: Vec<CfmlValue>) -> CfmlResult {
    if args.is_empty() {
        return Err(CfmlError::runtime("generateBCryptHash requires at least 1 argument: password".to_string()));
    }

    let password = get_str(&args, 0);
    let rounds = if args.len() >= 2 { get_int(&args, 1) as u32 } else { 10 };

    if rounds < 4 || rounds > 31 {
        return Err(CfmlError::runtime(format!(
            "generateBCryptHash: rounds must be between 4 and 31, got {}", rounds
        )));
    }

    let hash = bcrypt::hash(password.as_bytes(), rounds)
        .map_err(|e| CfmlError::runtime(format!("generateBCryptHash error: {}", e)))?;

    Ok(CfmlValue::string(hash))
}

/// verifyBCryptHash(password, hash)
/// Verify a password against a bcrypt hash. Returns boolean.
#[cfg(feature = "security")]
fn fn_verify_bcrypt_hash(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() < 2 {
        return Err(CfmlError::runtime("verifyBCryptHash requires 2 arguments: password, hash".to_string()));
    }

    let password = get_str(&args, 0);
    let hash = get_str(&args, 1);

    let result = bcrypt::verify(password.as_bytes(), &hash)
        .unwrap_or(false);

    Ok(CfmlValue::Bool(result))
}

/// BCryptHash( input, cost=10 ) — Lucee crypto-extension BIF. Generates a salted
/// bcrypt hash (each call differs); `cost` is the work factor 4-31. Modern name
/// for the deprecated GenerateBCryptHash().
#[cfg(feature = "security")]
fn fn_bcrypt_hash(args: Vec<CfmlValue>) -> CfmlResult {
    if args.is_empty() {
        return Err(CfmlError::runtime(
            "BCryptHash requires at least 1 argument: input".to_string(),
        ));
    }
    let input = get_str(&args, 0);
    let cost = if args.len() >= 2 { get_int(&args, 1) as u32 } else { 10 };
    if !(4..=31).contains(&cost) {
        return Err(CfmlError::runtime(format!(
            "BCryptHash: cost must be between 4 and 31, got {}",
            cost
        )));
    }
    // Emit the `$2a$` variant (jBCrypt-compatible) so hashes are interchangeable
    // with the org.mindrot.jbcrypt.BCrypt shim and reference engines.
    let parts = bcrypt::hash_with_result(input.as_bytes(), cost)
        .map_err(|e| CfmlError::runtime(format!("BCryptHash error: {}", e)))?;
    Ok(CfmlValue::string(parts.format_for_version(bcrypt::Version::TwoA)))
}

/// BCryptVerify( input, hash, throwOnError=false ) — Lucee crypto-extension BIF.
/// The cost factor is encoded in the hash. Returns false on a malformed hash
/// unless `throwOnError` is true. Modern name for the deprecated VerifyBCryptHash().
#[cfg(feature = "security")]
fn fn_bcrypt_verify(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() < 2 {
        return Err(CfmlError::runtime(
            "BCryptVerify requires 2 arguments: input, hash".to_string(),
        ));
    }
    let input = get_str(&args, 0);
    let hash = get_str(&args, 1);
    let throw_on_error = args.get(2).map(|v| v.is_true()).unwrap_or(false);
    match bcrypt::verify(input.as_bytes(), &hash) {
        Ok(ok) => Ok(CfmlValue::Bool(ok)),
        Err(e) => {
            if throw_on_error {
                Err(CfmlError::runtime(format!("BCryptVerify error: {}", e)))
            } else {
                Ok(CfmlValue::Bool(false))
            }
        }
    }
}

// ===============================================
// YAML (BoxLang-compatible: yamlDeserialize / yamlSerialize / yamlDeserializeFile)
// ===============================================

#[cfg(feature = "yaml")]
fn yaml_scalar_to_string(v: &serde_yaml::Value) -> String {
    use serde_yaml::Value as Y;
    match v {
        Y::Null => String::new(),
        Y::Bool(b) => b.to_string(),
        Y::Number(n) => n.to_string(),
        Y::String(s) => s.clone(),
        other => serde_yaml::to_string(other).unwrap_or_default().trim().to_string(),
    }
}

/// Convert a parsed `serde_yaml::Value` into native CFML values (recursively):
/// mappings → Struct, sequences → Array, scalars → Bool/Int/Double/String.
#[cfg(feature = "yaml")]
fn yaml_to_cfml(v: serde_yaml::Value) -> CfmlValue {
    use serde_yaml::Value as Y;
    match v {
        Y::Null => CfmlValue::Null,
        Y::Bool(b) => CfmlValue::Bool(b),
        Y::Number(n) => {
            if let Some(i) = n.as_i64() {
                CfmlValue::Int(i)
            } else {
                CfmlValue::Double(n.as_f64().unwrap_or(0.0))
            }
        }
        Y::String(s) => CfmlValue::string(s),
        Y::Sequence(seq) => CfmlValue::array(seq.into_iter().map(yaml_to_cfml).collect()),
        Y::Mapping(map) => {
            let mut m = ValueMap::default();
            for (k, val) in map {
                let key = match &k {
                    Y::String(s) => s.clone(),
                    other => yaml_scalar_to_string(other),
                };
                m.insert(key, yaml_to_cfml(val));
            }
            CfmlValue::strukt(m)
        }
        Y::Tagged(t) => yaml_to_cfml(t.value),
    }
}

/// Convert a CFML value into a `serde_yaml::Value` for serialization. Internal
/// component keys (`__*`) are skipped, mirroring serializeJSON.
#[cfg(feature = "yaml")]
fn cfml_to_yaml(v: &CfmlValue) -> serde_yaml::Value {
    use serde_yaml::Value as Y;
    match v {
        CfmlValue::Null => Y::Null,
        CfmlValue::Bool(b) => Y::Bool(*b),
        CfmlValue::Int(i) => Y::Number((*i).into()),
        CfmlValue::Double(d) => Y::Number((*d).into()),
        CfmlValue::String(s) => Y::String((**s).clone()),
        CfmlValue::Array(a) => {
            Y::Sequence(a.snapshot().iter().map(cfml_to_yaml).collect())
        }
        CfmlValue::Struct(s) => {
            let mut m = serde_yaml::Mapping::new();
            for (k, val) in s.iter() {
                if k.starts_with("__") {
                    continue;
                }
                m.insert(Y::String(k), cfml_to_yaml(&val));
            }
            Y::Mapping(m)
        }
        // Non-data values (queries, functions, binary, …) stringify.
        _ => Y::String(v.as_string()),
    }
}

/// yamlDeserialize( content ) — parse a YAML string into native CFML values.
#[cfg(feature = "yaml")]
fn fn_yaml_deserialize(args: Vec<CfmlValue>) -> CfmlResult {
    let content = get_str(&args, 0);
    let v: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|e| CfmlError::runtime(format!("yamlDeserialize: invalid YAML: {}", e)))?;
    Ok(yaml_to_cfml(v))
}

/// yamlSerialize( content, [filepath], [charset="utf8"] ) — serialize a CFML
/// value to a YAML string; optionally also write it to `filepath` (BoxLang
/// parity). Returns the YAML string. `charset` is accepted but ignored (UTF-8).
#[cfg(feature = "yaml")]
fn fn_yaml_serialize(args: Vec<CfmlValue>) -> CfmlResult {
    let data = args.first().cloned().unwrap_or(CfmlValue::Null);
    let yaml = serde_yaml::to_string(&cfml_to_yaml(&data))
        .map_err(|e| CfmlError::runtime(format!("yamlSerialize: {}", e)))?;
    if let Some(fp) = args.get(1) {
        let path = fp.as_string();
        if !path.is_empty() {
            std::fs::write(&path, &yaml).map_err(|e| {
                CfmlError::runtime(format!("yamlSerialize: cannot write [{}]: {}", path, e))
            })?;
        }
    }
    Ok(CfmlValue::string(yaml))
}

/// yamlDeserializeFile( filepath, [charset="utf8"] ) — read a YAML file and
/// parse it into native CFML values.
#[cfg(feature = "yaml")]
fn fn_yaml_deserialize_file(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    let content = std::fs::read_to_string(&path).map_err(|e| {
        CfmlError::runtime(format!("yamlDeserializeFile: cannot read [{}]: {}", path, e))
    })?;
    fn_yaml_deserialize(vec![CfmlValue::string(content)])
}

// ===============================================
// JSON SCHEMA VALIDATION (Lucee validateJSON + ca.vanmulligen shim helper)
// ===============================================

/// Removes trailing commas (a comma immediately before a closing `}`/`]`, modulo
/// whitespace) from a JSON string. String-aware: commas inside string literals
/// (and escaped quotes) are left untouched. `serde_json` is strict and rejects
/// trailing commas, but the Java JSON-schema validators Preside targets tolerate
/// them — several Preside schema files (e.g. `webflow.init.schema.json`) carry a
/// trailing comma, so we sanitize before parsing schema JSON to match Lucee.
#[cfg(feature = "jsonschema")]
fn strip_trailing_commas(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut pending_comma = false;
    let mut pending_ws = String::new();
    for c in s.chars() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if pending_comma {
            if c.is_whitespace() {
                pending_ws.push(c);
                continue;
            }
            // Only drop the comma when the next token closes an object/array.
            if c != '}' && c != ']' {
                out.push(',');
            }
            out.push_str(&pending_ws);
            pending_ws.clear();
            pending_comma = false;
            // fall through to emit `c` normally
        }
        if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == ',' {
            pending_comma = true;
        } else {
            out.push(c);
        }
    }
    if pending_comma {
        out.push(',');
        out.push_str(&pending_ws);
    }
    out
}

/// Lenient JSON parse for the schema-validation path: tries strict `serde_json`
/// first, then retries with trailing commas stripped. Keeps the common (valid)
/// case on the fast path and only pays the sanitizer cost on a parse error.
#[cfg(feature = "jsonschema")]
fn parse_json_lenient(s: &str) -> Result<serde_json::Value, serde_json::Error> {
    match serde_json::from_str(s) {
        Ok(v) => Ok(v),
        Err(_) => serde_json::from_str(&strip_trailing_commas(s)),
    }
}

/// Resolves external `$ref`s by reading the referenced file from disk. Preside's
/// cfflow schemas use relative cross-file refs (`"$ref":"transition.schema.json"`)
/// against a `file://<dir>` base URI; this reads each from the local filesystem
/// (no network — the crate is built without HTTP resolution).
#[cfg(feature = "jsonschema")]
struct FileRefRetriever;

#[cfg(feature = "jsonschema")]
impl jsonschema::Retrieve for FileRefRetriever {
    fn retrieve(
        &self,
        uri: &jsonschema::Uri<String>,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        let s = uri.as_str();
        // file:///abs/path -> /abs/path ; tolerate a plain path too.
        let path = s.strip_prefix("file://").unwrap_or(s);
        let content = std::fs::read_to_string(path)?;
        Ok(parse_json_lenient(&content)?)
    }
}

/// Core JSON-schema validation. Returns the list of (instancePointer, message)
/// violations (empty = valid), or Err if the instance/schema can't be parsed or
/// the schema is itself invalid. `base_uri` (e.g. `file://<dir>/`) anchors
/// relative cross-file `$ref` resolution.
#[cfg(feature = "jsonschema")]
fn json_schema_validate_core(
    json_str: &str,
    schema_str: &str,
    base_uri: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    let instance: serde_json::Value = parse_json_lenient(json_str)
        .map_err(|e| format!("invalid JSON instance: {}", e))?;
    let schema: serde_json::Value =
        parse_json_lenient(schema_str).map_err(|e| format!("invalid JSON schema: {}", e))?;

    let mut opts = jsonschema::options().with_retriever(FileRefRetriever);
    if let Some(b) = base_uri {
        if !b.is_empty() {
            opts = opts.with_base_uri(b.to_string());
        }
    }
    let validator = opts.build(&schema).map_err(|e| format!("{}", e))?;

    let errors = validator
        .iter_errors(&instance)
        .map(|err| (err.instance_path().to_string(), err.to_string()))
        .collect();
    Ok(errors)
}

/// validateJSON( json, schema, throwOnError=false [, baseUri] ) — Lucee BIF.
/// Returns an array of error structs (`{message, pointerToViolation}`); an empty
/// array means valid. With `throwOnError=true`, throws on the first violation.
/// The optional 4th arg `baseUri` anchors cross-file `$ref` resolution (used by
/// the ca.vanmulligen shim; not part of the Lucee signature).
#[cfg(feature = "jsonschema")]
fn fn_validate_json(args: Vec<CfmlValue>) -> CfmlResult {
    let json = get_str(&args, 0);
    let schema = get_str(&args, 1);
    let throw_on_error = args.get(2).map(|v| v.is_true()).unwrap_or(false);
    let base_uri = args.get(3).map(|v| v.as_string());

    match json_schema_validate_core(&json, &schema, base_uri.as_deref()) {
        Ok(errs) => {
            if throw_on_error && !errs.is_empty() {
                return Err(CfmlError::runtime(format!(
                    "JSON validation failed: {}",
                    errs[0].1
                )));
            }
            let arr: Vec<CfmlValue> = errs
                .into_iter()
                .map(|(path, msg)| {
                    let mut m = ValueMap::default();
                    m.insert("message".to_string(), CfmlValue::string(msg));
                    m.insert("pointerToViolation".to_string(), CfmlValue::string(path));
                    CfmlValue::strukt(m)
                })
                .collect();
            Ok(CfmlValue::array(arr))
        }
        Err(e) => Err(CfmlError::runtime(format!("validateJSON: {}", e))),
    }
}

/// Internal helper for the `ca.vanmulligen.json.schema.Validator` shim: validates
/// and returns the `{"valid":bool,"error":{…}}` JSON STRING that Preside's
/// JsonSchemaValidator.validate() deserializes. Args: (json, schema [, baseUri]).
#[cfg(feature = "jsonschema")]
fn fn_json_schema_validate_result(args: Vec<CfmlValue>) -> CfmlResult {
    let json = get_str(&args, 0);
    let schema = get_str(&args, 1);
    let base_uri = args.get(2).map(|v| v.as_string());

    match json_schema_validate_core(&json, &schema, base_uri.as_deref()) {
        Ok(errs) => {
            let result = if errs.is_empty() {
                serde_json::json!({ "valid": true, "error": {} })
            } else {
                let all: Vec<String> = errs.iter().map(|(_, m)| m.clone()).collect();
                let first_ptr = errs
                    .first()
                    .map(|(p, _)| if p.is_empty() { "#".to_string() } else { p.clone() })
                    .unwrap_or_else(|| "#".to_string());
                let first_msg = errs.first().map(|(_, m)| m.clone()).unwrap_or_default();
                serde_json::json!({
                    "valid": false,
                    "error": {
                        "violationCount": errs.len(),
                        "pointerToViolation": first_ptr,
                        "message": first_msg,
                        "keyword": "",
                        "allMessages": all,
                    }
                })
            };
            Ok(CfmlValue::string(result.to_string()))
        }
        // A schema/instance that can't even be parsed/built: report invalid with
        // the message rather than 500'ing (mirrors the Java validator's catch).
        Err(e) => Ok(CfmlValue::string(
            serde_json::json!({
                "valid": false,
                "error": { "violationCount": 1, "pointerToViolation": "#", "message": e, "keyword": "", "allMessages": [] }
            })
            .to_string(),
        )),
    }
}

/// generateSCryptHash(password)
/// Generate a scrypt hash. Returns encoded hash string.
#[cfg(feature = "security")]
fn fn_generate_scrypt_hash(args: Vec<CfmlValue>) -> CfmlResult {
    use scrypt::password_hash::{PasswordHasher, SaltString};
    use rand::rngs::OsRng;

    if args.is_empty() {
        return Err(CfmlError::runtime("generateSCryptHash requires at least 1 argument: password".to_string()));
    }

    let password = get_str(&args, 0);
    let salt = SaltString::generate(&mut OsRng);
    let params = scrypt::Params::recommended();
    let hasher = scrypt::Scrypt;

    let hash = hasher
        .hash_password_customized(
            password.as_bytes(),
            None,
            None,
            params,
            &salt,
        )
        .map_err(|e| CfmlError::runtime(format!("generateSCryptHash error: {}", e)))?;

    Ok(CfmlValue::string(hash.to_string()))
}

/// verifySCryptHash(password, hash)
/// Verify a password against a scrypt hash. Returns boolean.
#[cfg(feature = "security")]
fn fn_verify_scrypt_hash(args: Vec<CfmlValue>) -> CfmlResult {
    use scrypt::password_hash::{PasswordHash, PasswordVerifier};

    if args.len() < 2 {
        return Err(CfmlError::runtime("verifySCryptHash requires 2 arguments: password, hash".to_string()));
    }

    let password = get_str(&args, 0);
    let hash_str = get_str(&args, 1);

    let parsed_hash = PasswordHash::new(&hash_str)
        .map_err(|e| CfmlError::runtime(format!("verifySCryptHash: invalid hash format: {}", e)))?;

    let result = scrypt::Scrypt.verify_password(password.as_bytes(), &parsed_hash).is_ok();

    Ok(CfmlValue::Bool(result))
}

/// generateArgon2Hash(password)
/// Generate an Argon2id hash. Returns encoded hash string.
#[cfg(feature = "security")]
fn fn_generate_argon2_hash(args: Vec<CfmlValue>) -> CfmlResult {
    use argon2::password_hash::{PasswordHasher, SaltString};
    use rand::rngs::OsRng;

    if args.is_empty() {
        return Err(CfmlError::runtime("generateArgon2Hash requires at least 1 argument: password".to_string()));
    }

    let password = get_str(&args, 0);
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = argon2::Argon2::default(); // Argon2id with default params

    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| CfmlError::runtime(format!("generateArgon2Hash error: {}", e)))?;

    Ok(CfmlValue::string(hash.to_string()))
}

/// argon2CheckHash(hash, password)
/// Verify a password against an Argon2 hash. Note: CFML has hash first, password second.
/// Returns boolean.
#[cfg(feature = "security")]
fn fn_argon2_check_hash(args: Vec<CfmlValue>) -> CfmlResult {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};

    if args.len() < 2 {
        return Err(CfmlError::runtime("argon2CheckHash requires 2 arguments: hash, password".to_string()));
    }

    // Note: CFML convention is (hash, password) - reversed from other verify functions
    let hash_str = get_str(&args, 0);
    let password = get_str(&args, 1);

    let parsed_hash = PasswordHash::new(&hash_str)
        .map_err(|e| CfmlError::runtime(format!("argon2CheckHash: invalid hash format: {}", e)))?;

    let result = argon2::Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok();

    Ok(CfmlValue::Bool(result))
}

/// csrfGenerateToken([key, forceNew])
/// Generate a CSRF token. Returns a random 32-byte hex string (64 hex chars).
/// The key parameter is accepted but ignored (no server-side session storage).
#[cfg(feature = "security")]
fn fn_csrf_generate_token(args: Vec<CfmlValue>) -> CfmlResult {
    use rand::RngCore;

    if !security_flags().csrf_enabled {
        return Err(CfmlError::runtime(
            "csrfGenerateToken is disabled by security.csrfEnabled".to_string(),
        ));
    }

    // key (args[0]) and forceNew (args[1]) are accepted but ignored
    // since we don't have server-side session storage
    let _ = &args;

    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);

    Ok(CfmlValue::string(hex_encode(&bytes)))
}

/// csrfVerifyToken(token [, key])
/// Verify a CSRF token. Without session storage, we verify the token is a valid
/// 64-character hex string. Returns boolean.
#[cfg(feature = "security")]
fn fn_csrf_verify_token(args: Vec<CfmlValue>) -> CfmlResult {
    if !security_flags().csrf_enabled {
        return Err(CfmlError::runtime(
            "csrfVerifyToken is disabled by security.csrfEnabled".to_string(),
        ));
    }
    if args.is_empty() {
        return Err(CfmlError::runtime("csrfVerifyToken requires at least 1 argument: token".to_string()));
    }

    let token = get_str(&args, 0);
    // key (args[1]) is accepted but ignored

    // Verify: must be exactly 64 hex characters (32 bytes)
    let is_valid = token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit());

    Ok(CfmlValue::Bool(is_valid))
}

// ---- cfzip ----

#[cfg(feature = "zip_support")]
#[cfg(not(target_arch = "wasm32"))]
fn fn_cfzip(args: Vec<CfmlValue>) -> CfmlResult {
    use zip::write::SimpleFileOptions;
    use std::io::{Read, Write, Cursor};

    let opts = match args.into_iter().next() {
        Some(CfmlValue::Struct(s)) => s,
        _ => return Err(CfmlError::runtime("cfzip requires a struct argument".into())),
    };

    let action = opts.get("action").map(|v| v.as_string().to_lowercase()).unwrap_or_else(|| "zip".to_string());
    let file_path = opts.get("file").map(|v| v.as_string()).unwrap_or_default();
    let source = opts.get("source").map(|v| v.as_string()).unwrap_or_default();
    let destination = opts.get("destination").map(|v| v.as_string()).unwrap_or_default();
    let entry_path = opts.get("entrypath").map(|v| v.as_string()).unwrap_or_default();
    let overwrite = opts.get("overwrite").map(|v| v.is_true()).unwrap_or(false);
    let recurse = opts.get("recurse").map(|v| v.is_true()).unwrap_or(true);
    let store_path = opts.get("storepath").map(|v| v.is_true()).unwrap_or(true);
    let prefix = opts.get("prefix").map(|v| v.as_string()).unwrap_or_default();
    let charset = opts.get("charset").map(|v| v.as_string()).unwrap_or_else(|| "utf-8".to_string());
    let filter = opts.get("filter").map(|v| v.as_string()).unwrap_or_default();

    match action.as_str() {
        "zip" => {
            if file_path.is_empty() || source.is_empty() {
                return Err(CfmlError::runtime("cfzip action=zip requires file and source attributes".into()));
            }
            let out_file = std::fs::File::create(&file_path)
                .map_err(|e| CfmlError::runtime(format!("cfzip: cannot create file '{}': {}", file_path, e)))?;
            let mut zip_writer = zip::ZipWriter::new(out_file);
            let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

            let source_path = std::path::Path::new(&source);
            if source_path.is_dir() {
                cfzip_add_directory(&mut zip_writer, source_path, source_path, &options, recurse, store_path, &prefix, &filter)?;
            } else if source_path.is_file() {
                let name = if prefix.is_empty() {
                    source_path.file_name().unwrap_or_default().to_string_lossy().to_string()
                } else {
                    format!("{}/{}", prefix.trim_end_matches('/'), source_path.file_name().unwrap_or_default().to_string_lossy())
                };
                let mut f = std::fs::File::open(source_path)
                    .map_err(|e| CfmlError::runtime(format!("cfzip: cannot read '{}': {}", source, e)))?;
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).map_err(|e| CfmlError::runtime(e.to_string()))?;
                zip_writer.start_file(&name, options).map_err(|e| CfmlError::runtime(e.to_string()))?;
                zip_writer.write_all(&buf).map_err(|e| CfmlError::runtime(e.to_string()))?;
            } else {
                return Err(CfmlError::runtime(format!("cfzip: source '{}' does not exist", source)));
            }
            zip_writer.finish().map_err(|e| CfmlError::runtime(e.to_string()))?;
            Ok(CfmlValue::Null)
        }
        "unzip" => {
            if file_path.is_empty() || destination.is_empty() {
                return Err(CfmlError::runtime("cfzip action=unzip requires file and destination attributes".into()));
            }
            let f = std::fs::File::open(&file_path)
                .map_err(|e| CfmlError::runtime(format!("cfzip: cannot open '{}': {}", file_path, e)))?;
            let mut archive = zip::ZipArchive::new(f)
                .map_err(|e| CfmlError::runtime(format!("cfzip: invalid zip '{}': {}", file_path, e)))?;

            let dest = std::path::Path::new(&destination);
            std::fs::create_dir_all(dest).map_err(|e| CfmlError::runtime(e.to_string()))?;

            for i in 0..archive.len() {
                let mut entry = archive.by_index(i).map_err(|e| CfmlError::runtime(e.to_string()))?;
                let name = entry.name().to_string();

                if !entry_path.is_empty() && !name.starts_with(&entry_path) {
                    continue;
                }

                let out_path = if store_path {
                    dest.join(&name)
                } else {
                    dest.join(std::path::Path::new(&name).file_name().unwrap_or_default())
                };

                if entry.is_dir() {
                    std::fs::create_dir_all(&out_path).map_err(|e| CfmlError::runtime(e.to_string()))?;
                } else {
                    if let Some(parent) = out_path.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| CfmlError::runtime(e.to_string()))?;
                    }
                    if out_path.exists() && !overwrite {
                        continue;
                    }
                    let mut buf = Vec::new();
                    std::io::Read::read_to_end(&mut entry, &mut buf).map_err(|e| CfmlError::runtime(e.to_string()))?;
                    std::fs::write(&out_path, &buf).map_err(|e| CfmlError::runtime(e.to_string()))?;
                }
            }
            Ok(CfmlValue::Null)
        }
        "list" => {
            if file_path.is_empty() {
                return Err(CfmlError::runtime("cfzip action=list requires file attribute".into()));
            }
            let f = std::fs::File::open(&file_path)
                .map_err(|e| CfmlError::runtime(format!("cfzip: cannot open '{}': {}", file_path, e)))?;
            let mut archive = zip::ZipArchive::new(f)
                .map_err(|e| CfmlError::runtime(format!("cfzip: invalid zip '{}': {}", file_path, e)))?;

            let columns = vec![
                "name".to_string(), "directory".to_string(), "size".to_string(),
                "compressedsize".to_string(), "type".to_string(),
                "datelastmodified".to_string(), "comment".to_string(), "crc".to_string(),
            ];
            let query = cfml_common::dynamic::CfmlQuery::new(columns);

            for i in 0..archive.len() {
                let entry = archive.by_index(i).map_err(|e| CfmlError::runtime(e.to_string()))?;
                let name = entry.name().to_string();
                let is_dir = entry.is_dir();
                let dir = std::path::Path::new(&name).parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();

                let mut row = ValueMap::default();
                row.insert("name".to_string(), CfmlValue::string(name));
                row.insert("directory".to_string(), CfmlValue::string(dir));
                row.insert("size".to_string(), CfmlValue::Int(entry.size() as i64));
                row.insert("compressedsize".to_string(), CfmlValue::Int(entry.compressed_size() as i64));
                row.insert("type".to_string(), CfmlValue::string(if is_dir { "Dir".to_string() } else { "File".to_string() }));
                row.insert("datelastmodified".to_string(), CfmlValue::string(String::new()));
                row.insert("comment".to_string(), CfmlValue::string(entry.comment().to_string()));
                row.insert("crc".to_string(), CfmlValue::Int(entry.crc32() as i64));
                query.add_row(row);
            }

            Ok(CfmlValue::Query(query))
        }
        "read" => {
            if file_path.is_empty() || entry_path.is_empty() {
                return Err(CfmlError::runtime("cfzip action=read requires file and entrypath attributes".into()));
            }
            let f = std::fs::File::open(&file_path)
                .map_err(|e| CfmlError::runtime(format!("cfzip: cannot open '{}': {}", file_path, e)))?;
            let mut archive = zip::ZipArchive::new(f)
                .map_err(|e| CfmlError::runtime(format!("cfzip: invalid zip '{}': {}", file_path, e)))?;
            let mut entry = archive.by_name(&entry_path)
                .map_err(|e| CfmlError::runtime(format!("cfzip: entry '{}' not found: {}", entry_path, e)))?;
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut buf).map_err(|e| CfmlError::runtime(e.to_string()))?;
            let text = if charset.to_lowercase() == "utf-8" {
                String::from_utf8_lossy(&buf).to_string()
            } else {
                String::from_utf8_lossy(&buf).to_string()
            };
            Ok(CfmlValue::string(text))
        }
        "readbinary" => {
            if file_path.is_empty() || entry_path.is_empty() {
                return Err(CfmlError::runtime("cfzip action=readBinary requires file and entrypath attributes".into()));
            }
            let f = std::fs::File::open(&file_path)
                .map_err(|e| CfmlError::runtime(format!("cfzip: cannot open '{}': {}", file_path, e)))?;
            let mut archive = zip::ZipArchive::new(f)
                .map_err(|e| CfmlError::runtime(format!("cfzip: invalid zip '{}': {}", file_path, e)))?;
            let mut entry = archive.by_name(&entry_path)
                .map_err(|e| CfmlError::runtime(format!("cfzip: entry '{}' not found: {}", entry_path, e)))?;
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut buf).map_err(|e| CfmlError::runtime(e.to_string()))?;
            Ok(CfmlValue::Binary(buf))
        }
        "delete" => {
            if file_path.is_empty() || entry_path.is_empty() {
                return Err(CfmlError::runtime("cfzip action=delete requires file and entrypath attributes".into()));
            }
            // Read existing archive, write new one without the entry
            let f = std::fs::File::open(&file_path)
                .map_err(|e| CfmlError::runtime(format!("cfzip: cannot open '{}': {}", file_path, e)))?;
            let mut archive = zip::ZipArchive::new(f)
                .map_err(|e| CfmlError::runtime(format!("cfzip: invalid zip '{}': {}", file_path, e)))?;

            let mut buf = Cursor::new(Vec::new());
            {
                let mut writer = zip::ZipWriter::new(&mut buf);
                let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
                for i in 0..archive.len() {
                    let mut entry = archive.by_index(i).map_err(|e| CfmlError::runtime(e.to_string()))?;
                    let name = entry.name().to_string();
                    if name == entry_path {
                        continue;
                    }
                    let mut data = Vec::new();
                    std::io::Read::read_to_end(&mut entry, &mut data).map_err(|e| CfmlError::runtime(e.to_string()))?;
                    if entry.is_dir() {
                        writer.add_directory(&name, options).map_err(|e| CfmlError::runtime(e.to_string()))?;
                    } else {
                        writer.start_file(&name, options).map_err(|e| CfmlError::runtime(e.to_string()))?;
                        writer.write_all(&data).map_err(|e| CfmlError::runtime(e.to_string()))?;
                    }
                }
                writer.finish().map_err(|e| CfmlError::runtime(e.to_string()))?;
            }
            std::fs::write(&file_path, buf.into_inner())
                .map_err(|e| CfmlError::runtime(format!("cfzip: cannot write '{}': {}", file_path, e)))?;
            Ok(CfmlValue::Null)
        }
        _ => Err(CfmlError::runtime(format!("cfzip: unsupported action '{}'", action))),
    }
}

#[cfg(feature = "zip_support")]
#[cfg(not(target_arch = "wasm32"))]
fn cfzip_add_directory(
    writer: &mut zip::ZipWriter<std::fs::File>,
    dir: &std::path::Path,
    base: &std::path::Path,
    options: &zip::write::SimpleFileOptions,
    recurse: bool,
    store_path: bool,
    prefix: &str,
    filter: &str,
) -> Result<(), CfmlError> {
    use std::io::Read;

    let entries = std::fs::read_dir(dir)
        .map_err(|e| CfmlError::runtime(format!("cfzip: cannot read directory '{}': {}", dir.display(), e)))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if recurse {
                cfzip_add_directory(writer, &path, base, options, recurse, store_path, prefix, filter)?;
            }
        } else {
            let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            // Apply filter (simple glob: *.ext)
            if !filter.is_empty() {
                let pattern = filter.replace("*", "");
                if !file_name.ends_with(&pattern) {
                    continue;
                }
            }
            let archive_name = if store_path {
                let rel = path.strip_prefix(base).unwrap_or(&path);
                if prefix.is_empty() {
                    rel.to_string_lossy().to_string()
                } else {
                    format!("{}/{}", prefix.trim_end_matches('/'), rel.to_string_lossy())
                }
            } else {
                if prefix.is_empty() {
                    file_name
                } else {
                    format!("{}/{}", prefix.trim_end_matches('/'), file_name)
                }
            };
            let mut f = std::fs::File::open(&path)
                .map_err(|e| CfmlError::runtime(format!("cfzip: cannot read '{}': {}", path.display(), e)))?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).map_err(|e| CfmlError::runtime(e.to_string()))?;
            writer.start_file(&archive_name, *options).map_err(|e| CfmlError::runtime(e.to_string()))?;
            std::io::Write::write_all(writer, &buf).map_err(|e| CfmlError::runtime(e.to_string()))?;
        }
    }
    Ok(())
}

#[cfg(feature = "zip_support")]
#[cfg(target_arch = "wasm32")]
fn fn_cfzip(_args: Vec<CfmlValue>) -> CfmlResult {
    Err(CfmlError::runtime("cfzip is not supported in wasm".into()))
}

#[cfg(all(test, any(feature = "postgres_db", feature = "mysql_db")))]
mod db_tls_tests {

    #[cfg(feature = "postgres_db")]
    mod postgres {
    use crate::builtins::{pg_sanitize_url_sslmode, pg_sslmode_from_url};

    #[test]
    fn sslmode_defaults_to_prefer_when_absent() {
        assert_eq!(pg_sslmode_from_url("postgresql://u:p@host/db"), "prefer");
        assert_eq!(
            pg_sslmode_from_url("postgresql://u:p@host/db?application_name=x"),
            "prefer"
        );
    }

    #[test]
    fn sslmode_is_extracted_case_insensitively() {
        assert_eq!(
            pg_sslmode_from_url("postgresql://u:p@host/db?sslmode=require"),
            "require"
        );
        assert_eq!(
            pg_sslmode_from_url("postgresql://u:p@host/db?SSLMode=Verify-Full"),
            "verify-full"
        );
        assert_eq!(
            pg_sslmode_from_url("postgresql://u:p@host/db?x=1&sslmode=disable&y=2"),
            "disable"
        );
    }

    #[test]
    fn sanitize_maps_libpq_only_modes_to_tokio_tokens() {
        // verify-ca / verify-full / allow are not understood by tokio-postgres
        // and must be rewritten; the verification semantics are applied via the
        // rustls config, not the URL.
        assert_eq!(
            pg_sanitize_url_sslmode("postgresql://u:p@host/db?sslmode=verify-full"),
            "postgresql://u:p@host/db?sslmode=require"
        );
        assert_eq!(
            pg_sanitize_url_sslmode("postgresql://u:p@host/db?sslmode=verify-ca"),
            "postgresql://u:p@host/db?sslmode=require"
        );
        assert_eq!(
            pg_sanitize_url_sslmode("postgresql://u:p@host/db?sslmode=allow"),
            "postgresql://u:p@host/db?sslmode=prefer"
        );
    }

    #[test]
    fn sanitize_preserves_standard_modes_and_other_options() {
        assert_eq!(
            pg_sanitize_url_sslmode("postgresql://u:p@host/db?sslmode=require&channel_binding=require"),
            "postgresql://u:p@host/db?sslmode=require&channel_binding=require"
        );
        assert_eq!(
            pg_sanitize_url_sslmode("postgresql://u:p@host/db?sslmode=disable"),
            "postgresql://u:p@host/db?sslmode=disable"
        );
        // No query string at all → untouched.
        assert_eq!(
            pg_sanitize_url_sslmode("postgresql://u:p@host/db"),
            "postgresql://u:p@host/db"
        );
    }
    }

    #[cfg(feature = "mysql_db")]
    mod mysql {
        use crate::builtins::mysql_extract_ssl;

        #[test]
        fn no_ssl_param_stays_plaintext_and_untouched() {
            let (url, ssl) = mysql_extract_ssl("mysql://u:p@host:3306/db");
            assert_eq!(url, "mysql://u:p@host:3306/db");
            assert!(ssl.is_none());
            let (url, ssl) = mysql_extract_ssl("mysql://u:p@host/db?pool_max=4");
            assert_eq!(url, "mysql://u:p@host/db?pool_max=4");
            assert!(ssl.is_none());
        }

        #[test]
        fn ssl_keys_are_stripped_from_url() {
            // The mysql crate errors on unknown URL params, so our SSL keys must
            // be removed while genuine options are preserved.
            let (url, ssl) =
                mysql_extract_ssl("mysql://u:p@host/db?ssl_mode=REQUIRED&pool_max=4");
            assert_eq!(url, "mysql://u:p@host/db?pool_max=4");
            assert!(ssl.is_some());
        }

        #[test]
        fn required_encrypts_without_verifying() {
            let (_, ssl) = mysql_extract_ssl("mysql://u:p@host/db?ssl_mode=required");
            let ssl = ssl.expect("ssl opts");
            assert!(ssl.accept_invalid_certs());
            assert!(ssl.skip_domain_validation());
        }

        #[test]
        fn verify_identity_validates_fully() {
            let (_, ssl) = mysql_extract_ssl("mysql://u:p@host/db?ssl_mode=VERIFY_IDENTITY");
            let ssl = ssl.expect("ssl opts");
            assert!(!ssl.accept_invalid_certs());
            assert!(!ssl.skip_domain_validation());
        }

        #[test]
        fn verify_ca_checks_chain_not_hostname() {
            let (_, ssl) = mysql_extract_ssl("mysql://u:p@host/db?ssl_mode=verify_ca");
            let ssl = ssl.expect("ssl opts");
            assert!(!ssl.accept_invalid_certs());
            assert!(ssl.skip_domain_validation());
        }

        #[test]
        fn disabled_is_none() {
            let (_, ssl) = mysql_extract_ssl("mysql://u:p@host/db?ssl_mode=DISABLED");
            assert!(ssl.is_none());
        }

        #[test]
        fn jdbc_use_ssl_and_verify_flags() {
            let (_, ssl) = mysql_extract_ssl(
                "mysql://u:p@host/db?useSSL=true&verifyServerCertificate=true",
            );
            let ssl = ssl.expect("ssl opts");
            assert!(!ssl.accept_invalid_certs());
            assert!(!ssl.skip_domain_validation());

            let (_, ssl) = mysql_extract_ssl("mysql://u:p@host/db?useSSL=true");
            let ssl = ssl.expect("ssl opts");
            assert!(ssl.accept_invalid_certs()); // required, no verify
        }

        // A NULL column must read back as an empty string (Lucee/ACF default,
        // full null support OFF), matching sqlite_to_cfml. A raw `Null` here
        // fails to bind to a positional required arg — this is exactly what
        // broke Preside's sitetree editPage on the homepage (parent_page=NULL):
        // `_isManagedPage( prc.page.parent_page, ... )` → "parentId undefined".
        #[test]
        fn null_column_reads_as_empty_string_not_null() {
            use crate::builtins::mysql_value_to_cfml;
            use cfml_common::dynamic::CfmlValue;
            let v = mysql_value_to_cfml(mysql::Value::NULL);
            assert!(matches!(&v, CfmlValue::String(s) if s.is_empty()),
                "MySQL NULL should map to empty string, got {:?}", v);
            assert!(!matches!(v, CfmlValue::Null), "must not be CfmlValue::Null");
        }
    }
}

/// Regression tests for the MySQL named-parameter rewrite. The mysql crate's own
/// `Params::Named` parser only consumes lowercase identifier chars after `:`, so
/// it truncated camelCase placeholders (`:dateCreated` -> `?` + stray `Created`),
/// crashing Preside's object SQL with a MySQL syntax error. We rewrite `:name`
/// to positional `?` ourselves instead. Run with:
///   cargo test -p cfml-stdlib --features mysql_db
#[cfg(all(test, feature = "mysql_db"))]
mod mysql_named_param_tests {
    use super::{cfqueryparam_unwrap_typed, mysql_named_to_positional};
    use cfml_common::dynamic::{CfmlStruct, CfmlValue, ValueMap};

    fn struct_of(pairs: &[(&str, CfmlValue)]) -> CfmlStruct {
        let mut m = ValueMap::default();
        for (k, v) in pairs {
            m.insert((*k).to_string(), v.clone());
        }
        CfmlStruct::new(m)
    }

    // --- cfqueryparam cfsqltype coercion on the named-param path ---

    #[test]
    fn bit_typed_string_true_coerces_to_bool() {
        // Preside binds boolean columns as {value:"true", cfsqltype:"cf_sql_bit"}.
        // Without coercion this reached MySQL as the literal string 'true', which
        // it rejects for an integer/bit column.
        let p = CfmlValue::Struct(struct_of(&[
            ("value", CfmlValue::string("true")),
            ("cfsqltype", CfmlValue::string("cf_sql_bit")),
        ]));
        assert!(matches!(cfqueryparam_unwrap_typed(&p), CfmlValue::Bool(true)));

        let p = CfmlValue::Struct(struct_of(&[
            ("value", CfmlValue::string("false")),
            ("cfsqltype", CfmlValue::string("cf_sql_bit")),
        ]));
        assert!(matches!(cfqueryparam_unwrap_typed(&p), CfmlValue::Bool(false)));
    }

    #[test]
    fn integer_typed_string_coerces_to_int() {
        let p = CfmlValue::Struct(struct_of(&[
            ("value", CfmlValue::string("42")),
            ("cfsqltype", CfmlValue::string("cf_sql_integer")),
        ]));
        assert!(matches!(cfqueryparam_unwrap_typed(&p), CfmlValue::Int(42)));
    }

    #[test]
    fn null_flag_wins_over_type() {
        let p = CfmlValue::Struct(struct_of(&[
            ("value", CfmlValue::string("true")),
            ("null", CfmlValue::Bool(true)),
            ("cfsqltype", CfmlValue::string("cf_sql_bit")),
        ]));
        assert!(matches!(cfqueryparam_unwrap_typed(&p), CfmlValue::Null));
    }

    #[test]
    fn varchar_and_plain_values_pass_through() {
        let p = CfmlValue::Struct(struct_of(&[
            ("value", CfmlValue::string("hello")),
            ("cfsqltype", CfmlValue::string("cf_sql_varchar")),
        ]));
        assert_eq!(cfqueryparam_unwrap_typed(&p).as_string(), "hello");
        // A non-cfqueryparam plain value is returned unchanged.
        assert_eq!(cfqueryparam_unwrap_typed(&CfmlValue::Int(7)).as_string(), "7");
    }

    #[test]
    fn camelcase_placeholder_is_captured_whole() {
        // The exact shape that broke Preside: a camelCase named param.
        let map = struct_of(&[("dateCreated", CfmlValue::string("2026-06-27"))]);
        let (sql, vals) = mysql_named_to_positional(
            "insert into t ( datecreated ) values ( :dateCreated )",
            &map,
        );
        // No stray `Created` left in the SQL; placeholder became a single `?`.
        assert_eq!(sql, "insert into t ( datecreated ) values ( ? )");
        assert!(!sql.contains("Created"), "camelCase name must not leak into SQL: {sql}");
        assert_eq!(vals.len(), 1);
        assert_eq!(vals[0].as_string(), "2026-06-27");
    }

    #[test]
    fn repeated_named_param_binds_once_per_occurrence() {
        let map = struct_of(&[("id", CfmlValue::Int(7))]);
        let (sql, vals) = mysql_named_to_positional(
            "update t set parent = :id where id = :id",
            &map,
        );
        assert_eq!(sql, "update t set parent = ? where id = ?");
        assert_eq!(vals.len(), 2, "one bound value per textual occurrence");
        assert_eq!(vals[0].as_string(), "7");
        assert_eq!(vals[1].as_string(), "7");
    }

    #[test]
    fn name_is_matched_case_insensitively() {
        let map = struct_of(&[("DateCreated", CfmlValue::string("x"))]);
        let (sql, vals) = mysql_named_to_positional("select :datecreated", &map);
        assert_eq!(sql, "select ?");
        assert_eq!(vals.len(), 1);
        assert_eq!(vals[0].as_string(), "x");
    }

    #[test]
    fn colon_inside_string_literal_is_not_a_placeholder() {
        let map = struct_of(&[("x", CfmlValue::Int(1))]);
        let (sql, vals) = mysql_named_to_positional(
            "select ':notAParam', :x",
            &map,
        );
        assert_eq!(sql, "select ':notAParam', ?");
        assert_eq!(vals.len(), 1);
    }

    #[test]
    fn missing_named_param_binds_null() {
        let map = struct_of(&[("present", CfmlValue::Int(1))]);
        let (sql, vals) = mysql_named_to_positional("select :missing", &map);
        assert_eq!(sql, "select ?");
        assert_eq!(vals.len(), 1);
        assert!(matches!(vals[0], CfmlValue::Null));
    }

    #[test]
    fn list_param_expands_to_multiple_placeholders() {
        // The exact Preside shape: an array-valued filter joins ids with chr(31)
        // and binds with list=true. Must expand to `IN (?,?,?)`, not one `?`
        // bound to the whole "1\x1F3\x1F4" string (which MySQL rejects as a
        // truncated integer).
        let sep = "\u{1f}".to_string();
        let listval = format!("1{sep}3{sep}4");
        let map = struct_of(&[(
            "ids",
            CfmlValue::Struct(struct_of(&[
                ("value", CfmlValue::string(listval)),
                ("cfsqltype", CfmlValue::string("cf_sql_integer")),
                ("list", CfmlValue::Bool(true)),
                ("separator", CfmlValue::string("\u{1f}")),
            ])),
        )]);
        let (sql, vals) = mysql_named_to_positional("select * from t where id in (:ids)", &map);
        assert_eq!(sql, "select * from t where id in (?,?,?)");
        assert_eq!(vals.len(), 3);
        assert!(matches!(vals[0], CfmlValue::Int(1)));
        assert!(matches!(vals[1], CfmlValue::Int(3)));
        assert!(matches!(vals[2], CfmlValue::Int(4)));
    }

    #[test]
    fn list_param_default_comma_separator() {
        let map = struct_of(&[(
            "vals",
            CfmlValue::Struct(struct_of(&[
                ("value", CfmlValue::string("a,b,c")),
                ("cfsqltype", CfmlValue::string("cf_sql_varchar")),
                ("list", CfmlValue::Bool(true)),
            ])),
        )]);
        let (sql, vals) = mysql_named_to_positional("where x in (:vals)", &map);
        assert_eq!(sql, "where x in (?,?,?)");
        assert_eq!(vals.len(), 3);
        assert_eq!(vals[0].as_string(), "a");
        assert_eq!(vals[2].as_string(), "c");
    }
}

/// A row-returning statement wrapped in parentheses — `( SELECT ... ) UNION ALL
/// ( SELECT ... )`, the shape Preside's selectUnion() emits — must still be
/// classified as a query (run via the row-returning path), not a mutation.
#[cfg(all(test, any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db")))]
mod sql_classifier_tests {
    use super::{is_select_query, strip_leading_sql_noise};

    #[test]
    fn parenthesised_union_is_a_select() {
        assert!(is_select_query("( select id from a ) union all ( select id from b )"));
        assert!(is_select_query("(( select 1 ))"));
        assert!(is_select_query("  (  select 1 )"));
        assert!(is_select_query("(\n  select 1\n)"));
    }

    #[test]
    fn leading_paren_is_peeled_for_classification() {
        assert_eq!(strip_leading_sql_noise("( select 1 )").trim_start(), "select 1 )");
        assert_eq!(strip_leading_sql_noise("/* c */ ( select 1 )").trim_start(), "select 1 )");
    }

    #[test]
    fn plain_statements_unaffected() {
        assert!(is_select_query("select * from t"));
        assert!(is_select_query("WITH x as (select 1) select * from x"));
        assert!(!is_select_query("insert into t (a) values (1)"));
        assert!(!is_select_query("update t set a = 1"));
        assert!(!is_select_query("delete from t"));
    }
}

/// cfhttp connection-reuse regression. Before this fix cfhttp built a fresh
/// `ureq::Agent` per call, so every request opened and client-closed its own TCP
/// connection — no keep-alive. Under a burst that churns ephemeral ports into
/// TIME_WAIT and intermittently exhausts them, which is how a healthy local
/// ElasticSearch would still report "made N attempts but none returned a
/// response" (the Preside symptom that motivated this). With a shared pooled
/// agent, sequential requests to the same host reuse one connection. We prove
/// that by counting accepted TCP connections against a tiny keep-alive server:
/// old behaviour == one accept per request; fixed == a single accept for many.
#[cfg(all(test, feature = "http"))]
mod cfhttp_connection_reuse_tests {
    use super::fn_cfhttp;
    use cfml_common::dynamic::{CfmlValue, ValueMap};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn sequential_requests_reuse_one_keepalive_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let port = listener.local_addr().unwrap().port();
        let accepts = Arc::new(AtomicUsize::new(0));
        let accepts_srv = accepts.clone();

        // Keep-alive HTTP/1.1 server: one accept can serve many requests. It
        // never sends `Connection: close`, so ureq keeps the socket pooled.
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(s) => s,
                    Err(_) => break,
                };
                accepts_srv.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 1024];
                let mut acc: Vec<u8> = Vec::new();
                loop {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break, // client closed
                        Ok(n) => {
                            acc.extend_from_slice(&buf[..n]);
                            // Serve every complete (header-only, GET) request.
                            while let Some(pos) = find_double_crlf(&acc) {
                                acc.drain(..pos + 4);
                                let body = b"OK";
                                let resp = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                                    body.len()
                                );
                                if stream.write_all(resp.as_bytes()).is_err()
                                    || stream.write_all(body).is_err()
                                {
                                    return;
                                }
                                let _ = stream.flush();
                            }
                        }
                    }
                }
            }
        });

        let url = format!("http://127.0.0.1:{}/", port);
        let num_calls = 6;
        for _ in 0..num_calls {
            let mut opts = ValueMap::default();
            opts.insert("url".to_string(), CfmlValue::string(url.clone()));
            let result = fn_cfhttp(vec![CfmlValue::strukt(opts)]).expect("cfhttp ok");
            let s = match &result {
                CfmlValue::Struct(m) => m
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("fileContent"))
                    .map(|(_, v)| v.as_string())
                    .unwrap_or_default(),
                _ => String::new(),
            };
            assert_eq!(s, "OK", "each request round-trips a body");
        }

        let total = accepts.load(Ordering::SeqCst);
        // With pooling, all sequential calls ride one connection. Allow a tiny
        // margin for pool timing but it must be far below one-per-call.
        assert!(
            total < num_calls,
            "expected connection reuse (< {} accepts) but the server accepted {} \
             — cfhttp is opening a fresh connection per call",
            num_calls,
            total
        );
    }

    fn find_double_crlf(buf: &[u8]) -> Option<usize> {
        buf.windows(4).position(|w| w == b"\r\n\r\n")
    }
}
