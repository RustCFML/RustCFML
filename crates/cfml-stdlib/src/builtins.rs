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

use cfml_common::dynamic::{CfmlAccess, CfmlClosureBody, CfmlFunction, CfmlQuery, CfmlQueryData, CfmlStruct, CfmlValue};
use cfml_common::vm::{CfmlError, CfmlResult};
use std::collections::HashMap;
use indexmap::IndexMap;
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

pub type BuiltinFunction = fn(Vec<CfmlValue>) -> CfmlResult;

/// Returns all builtin functions as CfmlValue::Function references for the globals table
pub fn get_builtins() -> IndexMap<String, CfmlValue> {
    let mut builtins = IndexMap::new();
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
    f.insert("writeDump".into(), write_dump);
    f.insert("dump".into(), write_dump);

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
    f.insert("isBinary".into(), fn_is_binary);
    f.insert("isCustomFunction".into(), fn_is_custom_function);
    f.insert("isClosure".into(), fn_is_closure);
    f.insert("isValid".into(), fn_is_valid);

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
    f.insert("getFunctionCalledName".into(), fn_get_function_called_name);
    f.insert("getContextRoot".into(), fn_get_context_root);
    f.insert("GetContextRoot".into(), fn_get_context_root);
    f.insert("getPageContext".into(), fn_get_page_context);

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

    // ---- JSON functions ----
    f.insert("serializeJSON".into(), fn_serialize_json);
    f.insert("deserializeJSON".into(), fn_deserialize_json);
    f.insert("isJSON".into(), fn_is_json);

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
    // QoQ custom-function registration (VM-intercepted).
    f.insert("queryRegisterFunction".into(), fn_query_register_function_stub as BuiltinFunction);

    // ---- Query value list functions ----
    f.insert("valueList".into(), fn_value_list as BuiltinFunction);
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
    f.insert("getContextRoot".into(), fn_get_context_root);
    f.insert("GetContextRoot".into(), fn_get_context_root);
    f.insert("getPageContext".into(), fn_get_page_context);
    f.insert("getFileFromPath".into(), fn_get_file_from_path);
    f.insert("getCanonicalPath".into(), fn_get_canonical_path);
    f.insert("systemOutput".into(), fn_system_output);
    f.insert("getEnvironmentVariable".into(), fn_get_environment_variable);
    f.insert("readLine".into(), fn_read_line);
    f.insert("getTemplatePath".into(), fn_get_current_template_path);  // alias
    f.insert("writeLog".into(), fn_write_log);
    f.insert("setLocale".into(), fn_set_locale);
    f.insert("getLocale".into(), fn_get_locale);
    f.insert("setTimeZone".into(), fn_set_time_zone);

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
    f.insert("location".into(), fn_cflocation_stub);  // VM intercepts
    f.insert("trace".into(), fn_trace);
    f.insert("exceptionKeyExists".into(), fn_exception_key_exists);

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
    f.insert("__cflog".into(), fn_cflog_stub);
    f.insert("__cfparam".into(), fn_cfparam_stub);
    f.insert("__cfsetting".into(), fn_cfsetting_stub);
    f.insert("__cflock_start".into(), fn_cflock_start_stub);
    f.insert("__cflock_end".into(), fn_cflock_end_stub);
    f.insert("__cfcookie".into(), fn_cfcookie_stub);
    f.insert("__cfcache".into(), fn_cfcache_stub);
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

    // ---- Cache functions (VM-intercepted) ----
    f.insert("cachePut".into(), fn_cache_stub);
    f.insert("cacheGet".into(), fn_cache_stub);
    f.insert("cacheDelete".into(), fn_cache_stub);
    f.insert("cacheClear".into(), fn_cache_stub);
    f.insert("cacheKeyExists".into(), fn_cache_stub);
    f.insert("cacheCount".into(), fn_cache_stub);
    f.insert("cacheGetAll".into(), fn_cache_stub);
    f.insert("cacheGetAllIds".into(), fn_cache_stub);

    // ---- Session & Auth functions (VM-intercepted) ----
    f.insert("sessionInvalidate".into(), fn_session_stub);
    f.insert("sessionRotate".into(), fn_session_stub);
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
        f.insert("generateSCryptHash".into(), fn_generate_scrypt_hash);
        f.insert("verifySCryptHash".into(), fn_verify_scrypt_hash);
        f.insert("generateArgon2Hash".into(), fn_generate_argon2_hash);
        f.insert("argon2CheckHash".into(), fn_argon2_check_hash);
        f.insert("csrfGenerateToken".into(), fn_csrf_generate_token);
        f.insert("csrfVerifyToken".into(), fn_csrf_verify_token);
    }

    // ---- XML functions ----
    #[cfg(feature = "xml")]
    {
        f.insert("xmlParse".into(), fn_xml_parse);
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
    CfmlValue::Function(Box::new(CfmlFunction {
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
    s.with_read(|m| {
        if m.contains_key(key) {
            return Some(key.to_string());
        }
        let key_lower = key.to_lowercase();
        m.keys().find(|k| k.to_lowercase() == key_lower).cloned()
    })
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
    PRNG_SEEDED.with(|seeded| {
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
        Some(CfmlValue::String(s)) => Ok(CfmlValue::Int(s.len() as i64)),
        Some(CfmlValue::Bool(_)) | Some(CfmlValue::Int(_)) | Some(CfmlValue::Double(_)) => Ok(
            CfmlValue::Int(args.first().unwrap().as_string().chars().count() as i64),
        ),
        Some(CfmlValue::Array(a)) => Ok(CfmlValue::Int(a.len() as i64)),
        Some(CfmlValue::Struct(s)) => Ok(CfmlValue::Int(s.len() as i64)),
        Some(CfmlValue::Binary(b)) => Ok(CfmlValue::Int(b.len() as i64)),
        // Lucee@7 parity: len(q.col) treats the column as a string and returns
        // the first row's stringified length. This deliberately disagrees with
        // arrayLen() — which errors instead, matching Lucee's stricter rules.
        Some(v @ CfmlValue::QueryColumn(_)) => Ok(CfmlValue::Int(v.as_string().chars().count() as i64)),
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

fn fn_find(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        let substring = get_str(&args, 0);
        let string = get_str(&args, 1);
        let start = if args.len() >= 3 { get_int(&args, 2).max(1) as usize - 1 } else { 0 };
        if start < string.len() {
            if let Some(pos) = string[start..].find(&substring) {
                return Ok(CfmlValue::Int((pos + start + 1) as i64));
            }
        }
        Ok(CfmlValue::Int(0))
    } else {
        Ok(CfmlValue::Int(0))
    }
}

fn fn_find_no_case(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        let substring = get_str(&args, 0).to_lowercase();
        let string = get_str(&args, 1).to_lowercase();
        let start = if args.len() >= 3 { get_int(&args, 2).max(1) as usize - 1 } else { 0 };
        if start < string.len() {
            if let Some(pos) = string[start..].find(&substring) {
                return Ok(CfmlValue::Int((pos + start + 1) as i64));
            }
        }
        Ok(CfmlValue::Int(0))
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
        let pos = get_int(&args, 2).max(0) as usize;
        let mut result = string.clone();
        if pos <= result.len() {
            result.insert_str(pos, &substring);
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
    let start = if args.len() >= 3 { (get_int(&args, 2).max(1) as usize).saturating_sub(1) } else { 0 };
    let return_sub = if args.len() >= 4 { args[3].is_true() } else { false };

    let pat = if case_insensitive { format!("(?i){}", pattern) } else { pattern };
    let re = match Regex::new(&pat) {
        Ok(r) => r,
        Err(_) => return Ok(CfmlValue::Int(0)),
    };

    let search_str = if start < string.len() { &string[start..] } else { "" };

    if return_sub {
        if let Some(caps) = re.captures(search_str) {
            let mut pos_arr = Vec::new();
            let mut match_arr = Vec::new();
            let mut len_arr = Vec::new();
            for i in 0..caps.len() {
                if let Some(m) = caps.get(i) {
                    pos_arr.push(CfmlValue::Int((m.start() + start + 1) as i64));
                    match_arr.push(CfmlValue::string(m.as_str().to_string()));
                    len_arr.push(CfmlValue::Int(m.len() as i64));
                } else {
                    pos_arr.push(CfmlValue::Int(0));
                    match_arr.push(CfmlValue::string(String::new()));
                    len_arr.push(CfmlValue::Int(0));
                }
            }
            let mut result = IndexMap::new();
            result.insert("POS".to_string(), CfmlValue::array(pos_arr));
            result.insert("MATCH".to_string(), CfmlValue::array(match_arr));
            result.insert("LEN".to_string(), CfmlValue::array(len_arr));
            Ok(CfmlValue::strukt(result))
        } else {
            let mut result = IndexMap::new();
            result.insert("POS".to_string(), CfmlValue::array(vec![CfmlValue::Int(0)]));
            result.insert("MATCH".to_string(), CfmlValue::array(vec![CfmlValue::string(String::new())]));
            result.insert("LEN".to_string(), CfmlValue::array(vec![CfmlValue::Int(0)]));
            Ok(CfmlValue::strukt(result))
        }
    } else {
        match re.find(search_str) {
            Some(m) => Ok(CfmlValue::Int((m.start() + start + 1) as i64)),
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
fn expand_cfml_replacement(template: &str, caps: &regex::Captures) -> String {
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
            i += 2;
            let mut buf = [0u8; 4];
            match next {
                '0'..='9' => {
                    let g = next as usize - '0' as usize;
                    let text = caps.get(g).map(|m| m.as_str()).unwrap_or("");
                    emit(&mut out, text, ranged_upper, &mut oneshot_upper);
                }
                'u' => oneshot_upper = Some(true),
                'l' => oneshot_upper = Some(false),
                'U' => ranged_upper = Some(true),
                'L' => ranged_upper = Some(false),
                'E' => ranged_upper = None,
                'n' => emit(&mut out, "\n", ranged_upper, &mut oneshot_upper),
                't' => emit(&mut out, "\t", ranged_upper, &mut oneshot_upper),
                'r' => emit(&mut out, "\r", ranged_upper, &mut oneshot_upper),
                // Unknown escape (incl. "\\"): emit the escaped char literally.
                other => emit(&mut out, other.encode_utf8(&mut buf), ranged_upper, &mut oneshot_upper),
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
    let re = match Regex::new(&pat) {
        Ok(r) => r,
        Err(_) => return Ok(CfmlValue::string(string)),
    };

    // Use a custom replacer so CFML's `\N` backreferences and `\u`/`\l`/`\U`/`\L`
    // case modifiers are honored (and `$` stays literal).
    if scope == "all" {
        Ok(CfmlValue::string(
            re.replace_all(&string, |caps: &regex::Captures| expand_cfml_replacement(&replacement, caps))
                .to_string(),
        ))
    } else {
        Ok(CfmlValue::string(
            re.replace(&string, |caps: &regex::Captures| expand_cfml_replacement(&replacement, caps))
                .to_string(),
        ))
    }
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
    let re = match Regex::new(&pat) {
        Ok(r) => r,
        Err(_) => return Ok(CfmlValue::array(Vec::new())),
    };

    let matches: Vec<CfmlValue> = re.find_iter(&string)
        .map(|m| CfmlValue::string(m.as_str().to_string()))
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
    // Simple base64 encoding
    let input = get_str(&args, 0);
    let bytes = input.as_bytes();
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

fn fn_url_encode(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    let mut result = String::new();
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '*' => result.push(c),
            ' ' => result.push('+'),
            _ => {
                for b in c.to_string().as_bytes() {
                    result.push_str(&format!("%{:02X}", b));
                }
            }
        }
    }
    Ok(CfmlValue::string(result))
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
    Ok(CfmlValue::string(
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#x27;")
            .replace('/', "&#x2f;"),
    ))
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

    let int_formatted = if has_comma {
        add_thousands_separator(int_part)
    } else {
        int_part.to_string()
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
        Some(v @ CfmlValue::QueryColumn(_)) => Err(CfmlError::runtime(format!(
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

fn fn_array_contains(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let Some(arr) = args[0].as_array() {
            let needle = args[1].as_string();
            return Ok(CfmlValue::Bool(arr.iter().any(|v| v.as_string() == needle)));
        }
    }
    Ok(CfmlValue::Bool(false))
}

fn fn_array_contains_no_case(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let Some(arr) = args[0].as_array() {
            let needle = args[1].as_string().to_lowercase();
            return Ok(CfmlValue::Bool(
                arr.iter().any(|v| v.as_string().to_lowercase() == needle),
            ));
        }
    }
    Ok(CfmlValue::Bool(false))
}

fn fn_array_find(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let CfmlValue::Array(arr) = &args[0] {
            let needle = args[1].as_string();
            for (i, v) in arr.iter().enumerate() {
                if v.as_string() == needle {
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
            let needle = args[1].as_string().to_lowercase();
            for (i, v) in arr.iter().enumerate() {
                if v.as_string().to_lowercase() == needle {
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
        if let CfmlValue::Array(arr) = &args[0] {
            let idx = get_int(&args, 1) as usize;
            return Ok(CfmlValue::Bool(idx >= 1 && idx <= arr.len()));
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
    Ok(CfmlValue::Bool(false))
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
    Ok(CfmlValue::Bool(false))
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
    // Lucee@7 parity: QueryColumn is NOT an array.
    Ok(CfmlValue::Bool(matches!(args.first(), Some(CfmlValue::Array(_)))))
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
            let value = args[1].as_string();
            let indices: Vec<CfmlValue> = arr.iter().enumerate()
                .filter(|(_, v)| v.as_string() == value)
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
            let value = args[1].as_string().to_lowercase();
            let indices: Vec<CfmlValue> = arr.iter().enumerate()
                .filter(|(_, v)| v.as_string().to_lowercase() == value)
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
    Ok(CfmlValue::strukt(IndexMap::new()))
}

/// The `arguments` scope carries two private markers (`__arguments_scope` +
/// `__arguments_params`) used for positional-index fallback in `arguments[i]`.
/// Neither must appear in user-visible struct introspection (`structKeyList`,
/// `structCount`, `structKeyExists`, key arrays, for-in). Real numeric keys —
/// produced when overflow positional args have no matching declared param
/// name (e.g. paramless fn called positionally) — DO remain visible: that's
/// how Lucee surfaces them.
fn visible_struct_keys(s: &cfml_common::dynamic::CfmlStruct) -> Vec<String> {
    let keys: Vec<String> = s.keys();
    if !keys.iter().any(|k| k == "__arguments_scope") {
        return keys;
    }
    keys.into_iter()
        .filter(|k| k != "__arguments_scope" && k != "__arguments_params")
        .collect()
}

fn fn_struct_count(args: Vec<CfmlValue>) -> CfmlResult {
    match args.first() {
        Some(CfmlValue::Struct(s)) => Ok(CfmlValue::Int(visible_struct_keys(s).len() as i64)),
        _ => Ok(CfmlValue::Int(0)),
    }
}

fn fn_struct_key_exists(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let CfmlValue::Struct(s) = &args[0] {
            let key = args[1].as_string();
            let found = struct_find_key_ci(s, &key).is_some();
            // Hide the private arguments-scope markers from introspection.
            if found
                && s.contains_key("__arguments_scope")
                && (key == "__arguments_scope" || key == "__arguments_params")
            {
                return Ok(CfmlValue::Bool(false));
            }
            return Ok(CfmlValue::Bool(found));
        }
    }
    Ok(CfmlValue::Bool(false))
}

fn fn_struct_key_list(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(CfmlValue::Struct(s)) = args.first() {
        let delimiter = get_delimiter(&args, 1);
        Ok(CfmlValue::string(visible_struct_keys(s).join(&delimiter)))
    } else {
        Ok(CfmlValue::string(String::new()))
    }
}

fn fn_struct_key_array(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(CfmlValue::Struct(s)) = args.first() {
        let keys: Vec<CfmlValue> = visible_struct_keys(s)
            .into_iter()
            .map(CfmlValue::string)
            .collect();
        Ok(CfmlValue::array(keys))
    } else {
        Ok(CfmlValue::array(Vec::new()))
    }
}

fn fn_struct_delete(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let CfmlValue::Struct(s) = &args[0] {
            // Mutate the shared handle in place (Lucee reference semantics):
            // aliases of the struct observe the deletion.
            let key = args[1].as_string();
            s.remove_ci(&key);
            return Ok(CfmlValue::Struct(s.clone()));
        }
    }
    Ok(CfmlValue::Bool(false))
}

fn fn_struct_insert(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 3 {
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
        if k.to_lowercase() == search_lower {
            let mut result_struct = IndexMap::new();
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
            let mut result_struct = IndexMap::new();
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
    // Empty the shared handle in place (Lucee reference semantics).
    if let Some(CfmlValue::Struct(s)) = args.first() {
        s.clear();
        return Ok(CfmlValue::Struct(s.clone()));
    }
    Ok(CfmlValue::strukt(IndexMap::new()))
}

fn fn_struct_copy(args: Vec<CfmlValue>) -> CfmlResult {
    // Shallow copy: a fresh top-level struct over the same (shared) values.
    match args.first() {
        Some(CfmlValue::Struct(s)) => Ok(CfmlValue::strukt(s.snapshot())),
        _ => Ok(CfmlValue::strukt(IndexMap::new())),
    }
}

fn fn_struct_append(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        if let (CfmlValue::Struct(a), CfmlValue::Struct(b)) = (&args[0], &args[1]) {
            // Mutate the first struct's shared handle in place (Lucee semantics).
            let overwrite = if args.len() >= 3 { args[2].is_true() } else { true };
            for (k, v) in b.iter() {
                if overwrite || struct_find_key_ci(a, &k).is_none() {
                    a.insert(k, v);
                }
            }
            return Ok(CfmlValue::Struct(a.clone()));
        }
    }
    Ok(args.into_iter().next().unwrap_or(CfmlValue::strukt(IndexMap::new())))
}

fn fn_struct_is_empty(args: Vec<CfmlValue>) -> CfmlResult {
    match args.first() {
        Some(CfmlValue::Struct(s)) => Ok(CfmlValue::Bool(s.is_empty())),
        _ => Ok(CfmlValue::Bool(true)),
    }
}

fn fn_struct_sort(args: Vec<CfmlValue>) -> CfmlResult {
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
    Ok(CfmlValue::Bool(matches!(args.first(), Some(CfmlValue::Struct(_)))))
}

fn fn_struct_get(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = CfmlValue::strukt(IndexMap::new());
    for part in parts.iter().rev() {
        let mut s = IndexMap::new();
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
    if let Some(CfmlValue::Struct(s)) = args.first() {
        let values: Vec<CfmlValue> = s.iter().map(|(_, v)| v).collect();
        Ok(CfmlValue::array(values))
    } else {
        Ok(CfmlValue::array(Vec::new()))
    }
}

fn fn_struct_equals(args: Vec<CfmlValue>) -> CfmlResult {
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
    if let Some(CfmlValue::Struct(s)) = args.first() {
        let retain = args.get(1).map(|v| v.is_true()).unwrap_or(false);
        let mut result = IndexMap::new();
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
    Ok(CfmlValue::Bool(matches!(
        args.first(),
        Some(CfmlValue::Bool(_) | CfmlValue::Int(_) | CfmlValue::Double(_) | CfmlValue::String(_))
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
    match args.first() {
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
    let s = get_str(&args, 0);
    Ok(CfmlValue::Bool(parse_cfml_date(&s).is_some()))
}

fn fn_is_query(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Bool(matches!(args.first(), Some(CfmlValue::Query(_)))))
}

fn fn_is_object(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Bool(match args.first() {
        Some(CfmlValue::Component(_)) => true,
        Some(CfmlValue::Struct(s)) => {
            s.contains_key("__name") || s.contains_key("__java_shim")
        }
        Some(CfmlValue::NativeObject(_)) => true,
        _ => false,
    }))
}

fn fn_is_binary(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Bool(matches!(args.first(), Some(CfmlValue::Binary(_)))))
}

fn fn_is_custom_function(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Bool(matches!(args.first(), Some(CfmlValue::Function(_)))))
}

fn fn_is_closure(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Bool(matches!(args.first(), Some(CfmlValue::Closure(_)))))
}

fn fn_is_valid(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() >= 2 {
        let type_name = get_str(&args, 0).to_lowercase();
        let value = &args[1];
        match type_name.as_str() {
            "string" => Ok(CfmlValue::Bool(true)),
            "numeric" | "float" | "double" => fn_is_numeric(vec![value.clone()]),
            "integer" => {
                let s = value.as_string();
                Ok(CfmlValue::Bool(s.trim().parse::<i64>().is_ok()))
            }
            "boolean" => fn_is_boolean(vec![value.clone()]),
            "date" => fn_is_date(vec![value.clone()]),
            "array" => fn_is_array(vec![value.clone()]),
            "struct" => fn_is_struct(vec![value.clone()]),
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
                match Regex::new(&pattern) {
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
            _ => Ok(CfmlValue::Bool(false)),
        }
    } else {
        Ok(CfmlValue::Bool(false))
    }
}

// ===============================================
// CONVERSION FUNCTIONS
// ===============================================

fn fn_to_string(args: Vec<CfmlValue>) -> CfmlResult {
    match args.first() {
        Some(CfmlValue::Binary(bytes)) => {
            Ok(CfmlValue::string(String::from_utf8_lossy(bytes).to_string()))
        }
        _ => Ok(CfmlValue::string(get_str(&args, 0))),
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
        "%m/%d/%Y %H:%M",
        "%d %b %Y %H:%M:%S",
        "%b %d, %Y %H:%M:%S",
        "%B %d, %Y %H:%M:%S",
        "%d-%b-%Y %H:%M:%S",
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
            let secs = (frac * 86400.0) as u32;
            return base.and_hms_opt(0, 0, 0)
                .and_then(|dt| dt.checked_add_signed(chrono::Duration::days(days)))
                .and_then(|dt| dt.checked_add_signed(chrono::Duration::seconds(secs as i64)));
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
    Ok(CfmlValue::string(format!("{:04}-{:02}-{:02}", year, month, day)))
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
        "y" | "d" => dt.checked_add_signed(chrono::Duration::days(number)),
        "w" => dt.checked_add_signed(chrono::Duration::days(number)),
        "ww" => dt.checked_add_signed(chrono::Duration::weeks(number)),
        "h" => dt.checked_add_signed(chrono::Duration::hours(number)),
        "n" => dt.checked_add_signed(chrono::Duration::minutes(number)),
        "s" => dt.checked_add_signed(chrono::Duration::seconds(number)),
        "l" => dt.checked_add_signed(chrono::Duration::milliseconds(number)),
        _ => Some(dt),
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
        _ => 0,
    };
    Ok(CfmlValue::Int(diff))
}

fn fn_date_format(args: Vec<CfmlValue>) -> CfmlResult {
    let date_str = get_str(&args, 0);
    let mask = if args.len() > 1 { get_str(&args, 1) } else { String::new() };
    let dt = parse_cfml_date(&date_str)
        .ok_or_else(|| CfmlError::runtime(format!("Invalid date: {}", date_str)))?;
    Ok(CfmlValue::string(format_cfml_date(&dt, &mask, FormatMode::Date)))
}

fn fn_time_format(args: Vec<CfmlValue>) -> CfmlResult {
    let date_str = get_str(&args, 0);
    let mask = if args.len() > 1 { get_str(&args, 1) } else { String::new() };
    let dt = parse_cfml_date(&date_str)
        .ok_or_else(|| CfmlError::runtime(format!("Invalid date: {}", date_str)))?;
    Ok(CfmlValue::string(format_cfml_date(&dt, &mask, FormatMode::Time)))
}

fn fn_date_time_format(args: Vec<CfmlValue>) -> CfmlResult {
    let date_str = get_str(&args, 0);
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
    let mut result = IndexMap::new();
    for (name, _) in get_builtin_functions() {
        result.insert(name, CfmlValue::string(String::new()));
    }
    Ok(CfmlValue::strukt(result))
}

fn fn_get_context_root(_args: Vec<CfmlValue>) -> CfmlResult {
    // In a servlet context, returns the context root. For RustCFML, always "".
    Ok(CfmlValue::string(String::new()))
}

fn fn_get_page_context(_args: Vec<CfmlValue>) -> CfmlResult {
    // Fallback only. The real getPageContext() is VM-intercepted in
    // `cfml-vm` (`call_function` → `build_page_context_shim`) so it can read
    // the request's CGI scope and return a servlet bridge whose getRequest()/
    // getResponse() are method-faithful (getRequestURL, getMethod, setStatus,
    // …). This stub keeps the name registered as a builtin for resolution; it
    // is never reached when running on the VM.
    let mut ctx = IndexMap::new();
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
    let index = (get_int(&args, 1) as usize).saturating_sub(1);
    let value = get_str(&args, 2);
    let delimiter = get_delimiter(&args, 3);
    let first_delim = delimiter.chars().next().unwrap_or(',').to_string();
    let mut items: Vec<String> = cfml_list_split(&list, &delimiter).iter().map(|s| s.to_string()).collect();
    if index < items.len() {
        items[index] = value;
    }
    Ok(CfmlValue::string(items.join(&first_delim)))
}

fn fn_list_insert_at(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let index = (get_int(&args, 1) as usize).saturating_sub(1);
    let value = get_str(&args, 2);
    let delimiter = get_delimiter(&args, 3);
    let first_delim = delimiter.chars().next().unwrap_or(',').to_string();
    let mut items: Vec<String> = cfml_list_split(&list, &delimiter).iter().map(|s| s.to_string()).collect();
    if index <= items.len() {
        items.insert(index, value);
    }
    Ok(CfmlValue::string(items.join(&first_delim)))
}

fn fn_list_delete_at(args: Vec<CfmlValue>) -> CfmlResult {
    let list = get_str(&args, 0);
    let index = (get_int(&args, 1) as usize).saturating_sub(1);
    let delimiter = get_delimiter(&args, 2);
    let first_delim = delimiter.chars().next().unwrap_or(',').to_string();
    let mut items: Vec<String> = cfml_list_split(&list, &delimiter).iter().map(|s| s.to_string()).collect();
    if index < items.len() {
        items.remove(index);
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
// JSON FUNCTIONS
// ===============================================

fn fn_serialize_json(args: Vec<CfmlValue>) -> CfmlResult {
    let body = serialize_value(args.first().unwrap_or(&CfmlValue::Null));
    let flags = security_flags();
    if flags.secure_json && !flags.secure_json_prefix.is_empty() {
        Ok(CfmlValue::string(format!("{}{}", flags.secure_json_prefix, body)))
    } else {
        Ok(CfmlValue::string(body))
    }
}

fn serialize_value(val: &CfmlValue) -> String {
    match val {
        CfmlValue::Null => "null".to_string(),
        CfmlValue::Bool(b) => b.to_string(),
        CfmlValue::Int(i) => i.to_string(),
        CfmlValue::Double(d) => d.to_string(),
        CfmlValue::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")),
        CfmlValue::Array(arr) => {
            let items: Vec<String> = arr.iter().map(|v| serialize_value(&v)).collect();
            format!("[{}]", items.join(","))
        }
        CfmlValue::Struct(s) => {
            let items: Vec<String> = s.iter().map(|(k, v)| format!("\"{}\":{}", k.replace('"', "\\\""), serialize_value(&v))).collect();
            format!("{{{}}}", items.join(","))
        }
        CfmlValue::Query(q) => {
            q.with_read(|d| {
                let row_count = d.row_count();
                let rows: Vec<String> = (0..row_count).map(|r| {
                    let fields: Vec<String> = d.columns.iter().enumerate().map(|(ci, col)| {
                        let val = &d.data[ci][r];
                        format!("\"{}\":{}", col.replace('"', "\\\""), serialize_value(val))
                    }).collect();
                    format!("{{{}}}", fields.join(","))
                }).collect();
                format!("[{}]", rows.join(","))
            })
        }
        CfmlValue::NativeObject(obj) => {
            // Native Rust-backed objects don't have a JSON representation by
            // default. Emit a tagged marker rather than silently outputting
            // "null" — keeps the output visibly wrong if a caller forgets to
            // expose a Serializable method on their CfmlNative implementation.
            let name = obj.read().map(|g| g.class_name().to_string())
                .unwrap_or_else(|_| "poisoned".to_string());
            format!("\"<NativeObject:{}>\"", name.replace('"', "\\\""))
        }
        _ => "null".to_string(),
    }
}

fn fn_deserialize_json(args: Vec<CfmlValue>) -> CfmlResult {
    let json = get_str(&args, 0);
    match serde_json::from_str::<serde_json::Value>(&json) {
        Ok(value) => Ok(serde_json_to_cfml(value)),
        Err(e) => Err(CfmlError::runtime(format!("Invalid JSON: {}", e))),
    }
}

fn serde_json_to_cfml(value: serde_json::Value) -> CfmlValue {
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
            CfmlValue::array(arr.into_iter().map(serde_json_to_cfml).collect())
        }
        serde_json::Value::Object(obj) => {
            let mut map = IndexMap::new();
            for (k, v) in obj {
                map.insert(k, serde_json_to_cfml(v));
            }
            CfmlValue::strukt(map)
        }
    }
}

fn fn_is_json(args: Vec<CfmlValue>) -> CfmlResult {
    let s = get_str(&args, 0);
    Ok(CfmlValue::Bool(serde_json::from_str::<serde_json::Value>(&s).is_ok()))
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
        CfmlValue::String(s) => s.split(',').map(|c| c.trim().to_string()).collect(),
        CfmlValue::Array(arr) => arr.iter().map(|v| v.as_string()).collect(),
        _ => Vec::new(),
    };
    let mut rows: Vec<IndexMap<String, CfmlValue>> = Vec::new();
    // 3rd arg: initial data as array of arrays or array of structs
    if args.len() >= 3 {
        if let CfmlValue::Array(data_rows) = &args[2] {
            for row_data in data_rows.iter() {
                match row_data {
                    CfmlValue::Array(values) => {
                        // Array of arrays: each inner array maps positionally to columns
                        let mut row = IndexMap::new();
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
                        let mut row = IndexMap::new();
                        if !columns.is_empty() {
                            row.insert(columns[0].clone(), row_data.clone());
                        }
                        rows.push(row);
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
            q.add_row(IndexMap::new());
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
            let mut data = q.with_read(|d| d.clone());
            let col_name = args[1].as_string();
            let values: Vec<CfmlValue> = match args.get(2) {
                Some(CfmlValue::Array(arr)) => arr.snapshot(),
                _ => Vec::new(),
            };
            data.add_column(col_name, values);
            return Ok(CfmlValue::Query(CfmlQuery::from_data(data)));
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
                    if k.to_lowercase() == col_lower {
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
            let row_data: IndexMap<String, CfmlValue> = match &args[1] {
                CfmlValue::Struct(d) => d.snapshot(),
                _ => IndexMap::new(),
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
            let row_data: IndexMap<String, CfmlValue> = match &args[2] {
                CfmlValue::Struct(d) => d.snapshot(),
                _ => IndexMap::new(),
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
    let mut meta = IndexMap::new();
    if let Some(val) = args.first() {
        match val {
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

                // Extract __extends info
                if let Some(CfmlValue::Array(chain)) = s.get("__extends_chain") {
                    if let Some(first) = chain.first() {
                        let mut extends_meta = IndexMap::new();
                        extends_meta.insert("name".to_string(), first.clone());
                        meta.insert("extends".to_string(), CfmlValue::strukt(extends_meta));
                    }
                    meta.insert("fullExtends".to_string(), CfmlValue::Array(chain.clone()));
                }

                // Extract __metadata (custom attributes)
                // In CFML, custom attributes appear as top-level keys in getMetadata()
                if let Some(CfmlValue::Struct(md)) = s.get("__metadata") {
                    for (mk, mv) in md.iter() {
                        meta.insert(mk.clone(), mv.clone());
                    }
                    meta.insert("metadata".to_string(), CfmlValue::Struct(md.clone()));
                }

                // Enumerate functions
                let mut functions = Vec::new();
                for (k, v) in s.iter() {
                    if k.starts_with("__") { continue; }
                    if let CfmlValue::Function(f) = v {
                        let mut func_meta = IndexMap::new();
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
                            let mut pm = IndexMap::new();
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
                        func_meta.insert("parameters".to_string(), CfmlValue::array(params));
                        // Check for function metadata (__funcmeta_<name>)
                        let meta_key = format!("__funcmeta_{}", k);
                        if let Some(CfmlValue::Struct(fm)) = s.get(&meta_key) {
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
                        let mut prop_meta = IndexMap::new();
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
                    let mut pm = IndexMap::new();
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

    if let CfmlValue::Struct(s) = obj {
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
    }

    Ok(CfmlValue::Bool(false))
}

fn fn_create_object(args: Vec<CfmlValue>) -> CfmlResult {
    // Stub - VM intercepts this call before it reaches here
    // If we get here, just return a struct with a marker
    if args.len() >= 2 {
        let obj_type = args[0].as_string().to_lowercase();
        if obj_type == "component" {
            let mut s = IndexMap::new();
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
            // Fallback to MD5
            let mut hasher = Md5::new();
            hasher.update(input.as_bytes());
            format!("{:X}", hasher.finalize())
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
                if k.to_lowercase() == entry_lower {
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
        if k.to_lowercase() == entry_lower {
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
    let mut result = IndexMap::new();

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
        Err(e) => Err(CfmlError::runtime(format!("fileRead: {}", e))),
    }
}

fn fn_file_write(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() < 2 {
        return Err(CfmlError::runtime("fileWrite requires path and data".to_string()));
    }
    let path = get_str(&args, 0);
    let data = get_str(&args, 1);
    match std::fs::write(&path, &data) {
        Ok(_) => Ok(CfmlValue::Null),
        Err(e) => Err(CfmlError::runtime(format!("fileWrite: {}", e))),
    }
}

fn fn_file_append(args: Vec<CfmlValue>) -> CfmlResult {
    if args.len() < 2 {
        return Err(CfmlError::runtime("fileAppend requires path and data".to_string()));
    }
    let path = get_str(&args, 0);
    let data = get_str(&args, 1);
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| CfmlError::runtime(format!("fileAppend: {}", e)))?;
    file.write_all(data.as_bytes())
        .map_err(|e| CfmlError::runtime(format!("fileAppend: {}", e)))?;
    Ok(CfmlValue::Null)
}

fn fn_file_exists(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    Ok(CfmlValue::Bool(std::path::Path::new(&path).exists()))
}

fn fn_file_delete(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    match std::fs::remove_file(&path) {
        Ok(_) => Ok(CfmlValue::Bool(true)),
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
    // directoryList(path [, recurse [, listInfo [, filter]]])
    let path = get_str(&args, 0);
    let recurse = if args.len() >= 2 { args[1].is_true() } else { false };
    let list_info = if args.len() >= 3 { get_str(&args, 2).to_lowercase() } else { "path".to_string() };
    let filter = if args.len() >= 4 { get_str(&args, 3) } else { String::new() };

    fn matches_filter(filename: &str, filter: &str) -> bool {
        if filter.is_empty() { return true; }
        // Support glob patterns like "*.cfc", "*.cfm"
        if filter.starts_with("*.") {
            let ext = &filter[1..]; // ".cfc"
            filename.to_lowercase().ends_with(&ext.to_lowercase())
        } else if filter.contains('*') || filter.contains('?') {
            // Simple glob: convert to check
            filename.to_lowercase().contains(&filter.replace("*", "").to_lowercase())
        } else {
            filename.to_lowercase().contains(&filter.to_lowercase())
        }
    }

    enum Entry {
        Scalar(CfmlValue),
        Row { name: String, directory: String, size: u64, is_dir: bool },
    }

    fn list_dir(path: &str, recurse: bool, filter: &str, list_info: &str) -> Result<Vec<Entry>, std::io::Error> {
        let mut results = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let entry_path = entry.path();
            let full_path = entry_path.to_string_lossy().to_string();
            let file_name = entry.file_name().to_string_lossy().to_string();

            let is_dir = entry_path.is_dir();
            let is_file = entry_path.is_file();

            // Include both files and directories (matching CFML behavior)
            if (is_file && matches_filter(&file_name, filter)) || (is_dir && (filter.is_empty() || !filter.starts_with("*."))) {
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
                results.extend(list_dir(&full_path, true, filter, list_info)?);
            }
        }
        Ok(results)
    }

    match list_dir(&path, recurse, &filter, &list_info) {
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
                        let mut row = indexmap::IndexMap::new();
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
        Err(e) => Err(CfmlError::runtime(format!("directoryList: {}", e))),
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
    let meta = std::fs::metadata(path)
        .map_err(|e| CfmlError::runtime(format!("getFileInfo: {}", e)))?;

    let mut info = IndexMap::new();
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
        info.insert("lastModified".to_string(), CfmlValue::Int(secs as i64));
    }
    Ok(CfmlValue::strukt(info))
}

fn fn_expand_path(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    match std::fs::canonicalize(&path) {
        Ok(abs) => Ok(CfmlValue::string(abs.to_string_lossy().to_string())),
        Err(_) => {
            let cwd = std::env::current_dir().unwrap_or_default();
            Ok(CfmlValue::string(cwd.join(&path).to_string_lossy().to_string()))
        }
    }
}

fn fn_get_directory_from_path(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    if path.is_empty() {
        return Ok(CfmlValue::string(String::new()));
    }
    let p = std::path::Path::new(&path);
    match p.parent() {
        Some(parent) => {
            let mut dir = parent.to_string_lossy().to_string();
            if !dir.is_empty() && !dir.ends_with(std::path::MAIN_SEPARATOR) {
                dir.push(std::path::MAIN_SEPARATOR);
            }
            Ok(CfmlValue::string(dir))
        }
        None => Ok(CfmlValue::string(path)),
    }
}

fn fn_get_current_template_path(_args: Vec<CfmlValue>) -> CfmlResult {
    // Stub — VM intercepts this call to return the actual template path
    Ok(CfmlValue::string(String::new()))
}

fn fn_get_component_metadata(_args: Vec<CfmlValue>) -> CfmlResult {
    // Stub — VM intercepts this call to resolve component metadata
    Ok(CfmlValue::strukt(IndexMap::new()))
}

// ===============================================
// ADDITIONAL BUILTINS (Feature 3)
// ===============================================

fn fn_encode_for_url(args: Vec<CfmlValue>) -> CfmlResult {
    fn_url_encode(args)
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
    for c in s.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&#x27;"),
            '/' => result.push_str("&#x2f;"),
            '\t' => result.push_str("&#x9;"),
            '\n' => result.push_str("&#xa;"),
            '\r' => result.push_str("&#xd;"),
            '`' => result.push_str("&#x60;"),
            _ => result.push(c),
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
        "url" => fn_url_encode(value_args),
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
        s = url_decode_string(&s);
        if s == prev {
            break;
        }
    }
    Ok(CfmlValue::string(s))
}

fn url_decode_string(s: &str) -> String {
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
fn fn_cfhttp(args: Vec<CfmlValue>) -> CfmlResult {
    use std::collections::HashMap;

    let arg = args.into_iter().next().unwrap_or(CfmlValue::Null);

    // Parse arguments: either a URL string or an options struct
    let (mut url, method, headers, body, timeout_secs, throw_on_error, follow_redirects, encode_url, port, proxy_server, proxy_port) = match &arg {
        CfmlValue::String(url) => ((**url).clone(), "GET".to_string(), HashMap::<String, String>::new(), None::<Vec<u8>>, 30u64, false, true, true, None::<u16>, None::<String>, None::<u16>),
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

            (url, method, hdrs, body, timeout, throw_on_error, follow_redirects, encode_url, port, proxy_server, proxy_port)
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

    let mut agent_builder = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(timeout_secs));

    if !follow_redirects {
        agent_builder = agent_builder.redirects(0);
    }

    if let Some(ref proxy_host) = proxy_server {
        if !proxy_host.is_empty() {
            let proxy_url = if let Some(pp) = proxy_port {
                format!("http://{}:{}", proxy_host, pp)
            } else {
                format!("http://{}", proxy_host)
            };
            if let Ok(proxy) = ureq::Proxy::new(&proxy_url) {
                agent_builder = agent_builder.proxy(proxy);
            }
        }
    }

    let agent = agent_builder.build();

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

    for (k, v) in &headers {
        request = request.set(k, v);
    }

    let response = if let Some(body_bytes) = &body {
        request.send_bytes(body_bytes)
    } else {
        request.call()
    };

    let mut result_struct: IndexMap<String, CfmlValue> = IndexMap::new();

    match response {
        Ok(resp) => {
            let status = resp.status();
            let status_text = resp.status_text().to_string();
            let http_version = resp.http_version().to_string();
            let content_type = resp.content_type().to_string();

            let mut resp_headers: IndexMap<String, CfmlValue> = IndexMap::new();
            for name in resp.headers_names() {
                if let Some(val) = resp.header(&name) {
                    resp_headers.insert(name, CfmlValue::string(val.to_string()));
                }
            }

            let body_text = resp.into_string().unwrap_or_default();

            let (mime, charset) = parse_content_type(&content_type);

            if throw_on_error && status >= 400 {
                return Err(CfmlError::runtime(format!("cfhttp request failed: {} {}", status, status_text)));
            }

            result_struct.insert("statusCode".to_string(), CfmlValue::string(format!("{} {}", status, status_text)));
            result_struct.insert("status_code".to_string(), CfmlValue::Int(status as i64));
            result_struct.insert("statusText".to_string(), CfmlValue::string(status_text.clone()));
            result_struct.insert("status_text".to_string(), CfmlValue::string(status_text));
            result_struct.insert("fileContent".to_string(), CfmlValue::string(body_text));
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

            let mut resp_headers: IndexMap<String, CfmlValue> = IndexMap::new();
            for name in resp.headers_names() {
                if let Some(val) = resp.header(&name) {
                    resp_headers.insert(name, CfmlValue::string(val.to_string()));
                }
            }

            let body_text = resp.into_string().unwrap_or_default();
            let (mime, charset) = parse_content_type(&content_type);

            result_struct.insert("statusCode".to_string(), CfmlValue::string(format!("{} {}", code, status_text)));
            result_struct.insert("status_code".to_string(), CfmlValue::Int(code as i64));
            result_struct.insert("statusText".to_string(), CfmlValue::string(status_text.clone()));
            result_struct.insert("status_text".to_string(), CfmlValue::string(status_text));
            result_struct.insert("fileContent".to_string(), CfmlValue::string(body_text));
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
            result_struct.insert("responseHeader".to_string(), CfmlValue::strukt(IndexMap::new()));
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

#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
pub(crate) fn parse_datasource(ds: &str) -> DbDriver {
    if ds.starts_with("mysql://") {
        DbDriver::Mysql(ds.to_string())
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
    let mgr = SqliteConnectionManager { path: path.to_string() };
    let pool = r2d2::Pool::builder()
        .max_size(10)
        .min_idle(Some(1))
        .connection_timeout(std::time::Duration::from_secs(30))
        .build(mgr)
        .map_err(|e| CfmlError::runtime(format!("queryExecute: failed to create SQLite pool: {}", e)))?;
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
        .map_err(|e| CfmlError::runtime(format!("queryExecute: invalid MySQL connection string: {}", e)))?;
    let builder = mysql::OptsBuilder::from_opts(opts).ssl_opts(ssl);
    let pool = mysql::Pool::new(builder)
        .map_err(|e| CfmlError::runtime(format!("queryExecute: MySQL pool creation error: {}", e)))?;
    manager.insert(key, Box::new(pool.clone()));
    Ok(pool)
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
        Ok(PgConn { client, stmt_cache: HashMap::new() })
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
        conn.client.is_closed()
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
        .max_size(10)
        .min_idle(Some(1))
        .connection_timeout(std::time::Duration::from_secs(30))
        // Do NOT ping `SELECT 1` on every checkout (Lucee parity / remote-DB
        // perf): Lucee's pool defaults `validate` off, so a cfquery is ONE
        // round-trip, not two. We rely on PostgresConnectionManager::has_broken
        // (is_closed) to evict dead connections on return instead. See PR #125.
        .test_on_check_out(false)
        .build(mgr)
        .map_err(|e| CfmlError::runtime(format!("queryExecute: failed to create PostgreSQL pool: {}", e)))?;
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

/// Strip leading whitespace and SQL comments (`-- line` and `/* block */`)
/// so the first *significant* keyword can be classified. Returns the
/// remainder starting at the first non-comment, non-whitespace byte.
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
            .map(|(_, v)| v.as_string()),
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

#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
pub fn fn_query_execute(args: Vec<CfmlValue>) -> CfmlResult {
    let sql = get_str(&args, 0);
    if sql.is_empty() {
        return Err(CfmlError::runtime("queryExecute: SQL string is required".to_string()));
    }

    let raw_params = args.get(1).cloned().unwrap_or(CfmlValue::Null);
    let options_arg = args.get(2).cloned().unwrap_or(CfmlValue::Null);

    // Normalize structured params (cfqueryparam-style array of structs) to plain values
    // Also expand SQL for list params (single ? → multiple ?,?,?)
    let (sql, params_arg) = match &raw_params {
        CfmlValue::Array(arr) if !arr.is_empty() => {
            if let Some(CfmlValue::Struct(first)) = arr.first() {
                if first.iter().any(|(k, _)| k.eq_ignore_ascii_case("value")) {
                    let (values, _hints) = normalize_query_params(&raw_params);
                    // Check if any list params require SQL expansion
                    let placeholder_counts = get_list_placeholder_counts(&raw_params);
                    let expanded_sql = if placeholder_counts.iter().any(|&c| c > 1) {
                        expand_sql_placeholders(&sql, &placeholder_counts)
                    } else {
                        sql
                    };
                    (expanded_sql, CfmlValue::array(values))
                } else {
                    (sql, raw_params)
                }
            } else {
                (sql, raw_params)
            }
        }
        _ => (sql, raw_params),
    };

    // Extract datasource from options. Look up against the cfconfig
    // registry first; fall back to the default datasource if the call
    // omitted one entirely; final fallback is the historical `:memory:`
    // sqlite default so existing tests keep working.
    let datasource_attr: Option<String> = match &options_arg {
        CfmlValue::Struct(opts) => opts
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("datasource"))
            .map(|(_, v)| v.as_string()),
        _ => None,
    };

    // Dynamic-driver fast path: if the literal datasource name is registered
    // via cfml_stdlib::db_driver, hand off without touching the URL/enum
    // pipeline. This is how cfml-worker plugs in Cloudflare D1.
    let dynamic_driver = datasource_attr
        .as_deref()
        .and_then(crate::db_driver::lookup_dynamic_datasource);

    let datasource = match datasource_attr {
        Some(name) => resolve_datasource(&name),
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
                let key = opts.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("columnkey") || k.eq_ignore_ascii_case("columnKey"))
                    .map(|(_, v)| v.as_string())
                    .unwrap_or_default();
                if key.is_empty() { rt } else { format!("struct:{}", key) }
            } else {
                rt
            }
        }
        _ => "query".to_string(),
    };

    if let Some(driver) = dynamic_driver {
        return driver.execute(&sql, &params_arg, &return_type);
    }

    match parse_datasource(&datasource) {
        #[cfg(feature = "sqlite")]
        DbDriver::Sqlite(path) => execute_sqlite(&path, &sql, &params_arg, &return_type),
        #[cfg(feature = "mysql_db")]
        DbDriver::Mysql(url) => execute_mysql(&url, &sql, &params_arg, &return_type),
        #[cfg(feature = "postgres_db")]
        DbDriver::Postgres(url) => execute_postgres(&url, &sql, &params_arg, &return_type),
        #[cfg(feature = "mssql_db")]
        DbDriver::Mssql(url) => execute_mssql(&url, &sql, &params_arg, &return_type),
        #[allow(unreachable_patterns)]
        _ => Err(CfmlError::runtime(format!(
            "queryExecute: database driver not available for datasource '{}'. Enable the appropriate feature (sqlite, mysql_db, postgres_db, mssql_db).",
            datasource
        ))),
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
        .map_err(|e| CfmlError::runtime(format!("queryExecute: failed to get SQLite connection from pool: {}", e)))?;

    let bound_params = build_sqlite_params(params_arg, sql)?;

    if is_select_query(sql) {
        let mut stmt = conn.prepare(sql)
            .map_err(|e| CfmlError::runtime(format!("queryExecute: SQL error: {}", e)))?;

        let column_count = stmt.column_count();
        let columns: Vec<String> = (0..column_count)
            .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
            .collect();

        let rows_result: Result<Vec<IndexMap<String, CfmlValue>>, _> = stmt
            .query_map(rusqlite::params_from_iter(bound_params.iter()), |row| {
                let mut row_map = IndexMap::new();
                for (i, col) in columns.iter().enumerate() {
                    let val: SqlValue = row.get_unwrap(i);
                    row_map.insert(col.clone(), sqlite_to_cfml(val));
                }
                Ok(row_map)
            })
            .map_err(|e| CfmlError::runtime(format!("queryExecute: query error: {}", e)))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CfmlError::runtime(format!("queryExecute: row error: {}", e)));

        let rows = rows_result?;
        build_query_result(columns, rows, sql, return_type)
    } else {
        let affected = conn.execute(sql, rusqlite::params_from_iter(bound_params.iter()))
            .map_err(|e| CfmlError::runtime(format!("queryExecute: SQL error: {}", e)))?;

        let last_id = conn.last_insert_rowid();
        build_mutation_result(affected as i64, last_id, sql)
    }
}

#[cfg(feature = "sqlite")]
fn build_sqlite_params(params_arg: &CfmlValue, sql: &str) -> Result<Vec<rusqlite::types::Value>, CfmlError> {
    match params_arg {
        CfmlValue::Null => Ok(vec![]),
        CfmlValue::Array(arr) => {
            arr.iter().map(|v| Ok(cfml_to_sqlite(&v))).collect()
        }
        CfmlValue::Struct(map) => {
            let mut result = Vec::new();
            let bytes = sql.as_bytes();
            let len = bytes.len();
            let mut i = 0;
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
                        // ({value, cfsqltype, null, ...}). Unwrap to the bind
                        // value (or NULL when null=true) before driver binding,
                        // matching Lucee. Positional arrays are already
                        // normalized by normalize_query_params() upstream.
                        let val = cfqueryparam_unwrap(&raw);
                        result.push(cfml_to_sqlite(&val));
                        i = end;
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
            Ok(result)
        }
        _ => Ok(vec![]),
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
        SqlValue::Null => CfmlValue::Null,
        SqlValue::Integer(i) => CfmlValue::Int(i),
        SqlValue::Real(d) => CfmlValue::Double(d),
        SqlValue::Text(s) => CfmlValue::string(s),
        SqlValue::Blob(b) => CfmlValue::Binary(b),
    }
}

// -----------------------------------------------
// MySQL driver
// -----------------------------------------------

#[cfg(feature = "mysql_db")]
fn execute_mysql(url: &str, sql: &str, params_arg: &CfmlValue, return_type: &str) -> CfmlResult {
    use mysql::*;
    use mysql::prelude::*;

    let pool = get_mysql_pool(url)?;
    let mut conn = pool.get_conn()
        .map_err(|e| CfmlError::runtime(format!("queryExecute: MySQL connection error: {}", e)))?;

    let params = match params_arg {
        CfmlValue::Array(arr) => {
            let vals: Vec<mysql::Value> = arr.iter().map(|v| cfml_to_mysql_value(&v)).collect();
            Params::Positional(vals)
        }
        CfmlValue::Struct(map) => {
            let mut named: HashMap<Vec<u8>, mysql::Value> = HashMap::new();
            for (k, v) in map.iter() {
                // Unwrap cfqueryparam-style param structs before binding.
                let val = cfqueryparam_unwrap(&v);
                named.insert(k.as_bytes().to_vec(), cfml_to_mysql_value(&val));
            }
            Params::Named(named)
        }
        _ => Params::Empty,
    };

    if is_select_query(sql) {
        let result: Vec<Row> = conn.exec(sql, &params)
            .map_err(|e| CfmlError::runtime(format!("queryExecute: MySQL query error: {}", e)))?;

        // Extract column names from result set
        let columns: Vec<String> = if let Some(first_row) = result.first() {
            first_row.columns_ref().iter()
                .map(|c| c.name_str().to_string())
                .collect()
        } else {
            // No rows - try to get columns from prepared statement
            vec![]
        };

        let mut rows: Vec<IndexMap<String, CfmlValue>> = Vec::with_capacity(result.len());
        for row in &result {
            let mut row_map = IndexMap::new();
            for (i, col) in columns.iter().enumerate() {
                let val: mysql::Value = row.get(i).unwrap_or(mysql::Value::NULL);
                row_map.insert(col.clone(), mysql_value_to_cfml(val));
            }
            rows.push(row_map);
        }

        build_query_result(columns, rows, sql, return_type)
    } else {
        conn.exec_drop(sql, &params)
            .map_err(|e| CfmlError::runtime(format!("queryExecute: MySQL error: {}", e)))?;

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
        mysql::Value::NULL => CfmlValue::Null,
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
        // DATE columns come with all-zero time fields; DATETIME/TIMESTAMP have
        // populated time fields. Match CFML's canonical formats so dateFormat
        // and timeFormat work without further coercion.
        mysql::Value::Date(y, mo, d, h, mi, s, _us) => {
            if h == 0 && mi == 0 && s == 0 {
                CfmlValue::string(format!("{:04}-{:02}-{:02}", y, mo, d))
            } else {
                CfmlValue::string(format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, mo, d, h, mi, s))
            }
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

// -----------------------------------------------
// PostgreSQL driver
// -----------------------------------------------

#[cfg(feature = "postgres_db")]
fn execute_postgres(url: &str, sql: &str, params_arg: &CfmlValue, return_type: &str) -> CfmlResult {
    let pool = get_postgres_pool(url)?;
    let mut conn = pool.get()
        .map_err(|e| CfmlError::runtime(format!("queryExecute: PostgreSQL connection error: {}", e)))?;
    let pg = &mut *conn;
    run_postgres_statements(&mut pg.client, &mut pg.stmt_cache, sql, params_arg, return_type)
}

/// Shared PostgreSQL execution for both the pooled (`execute_postgres`) and
/// persistent-connection (`execute_postgres_with_conn`) paths.
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
fn run_postgres_statements(
    client: &mut postgres::Client,
    cache: &mut PgStmtCache,
    sql: &str,
    params_arg: &CfmlValue,
    return_type: &str,
) -> CfmlResult {
    let select = is_select_query(sql);
    let statements = crate::pg_sql::prepare_pg_statements(sql, params_arg, !select)?;

    if select {
        // split=false guarantees exactly one statement.
        let stmt = &statements[0];
        let pg_params: Vec<PgParam> = stmt.params.iter().map(cfml_to_pg_param).collect();
        let param_refs: Vec<&(dyn postgres::types::ToSql + Sync)> = pg_params.iter()
            .map(|v| v as &(dyn postgres::types::ToSql + Sync))
            .collect();
        // Cached prepared statement → one round-trip in steady state. On a stale
        // cached plan (DDL changed the table), evict and re-prepare once.
        let prepared = pg_prepare_cached(client, cache, stmt.sql.as_str())
            .map_err(|e| CfmlError::runtime(format_pg_error("queryExecute: PostgreSQL prepare error", &e)))?;
        let rows = match client.query(&prepared, &param_refs) {
            Ok(r) => r,
            Err(e) if is_stale_cached_plan(&e) => {
                cache.remove(stmt.sql.as_str());
                let prepared = pg_prepare_cached(client, cache, stmt.sql.as_str())
                    .map_err(|e| CfmlError::runtime(format_pg_error("queryExecute: PostgreSQL prepare error", &e)))?;
                client.query(&prepared, &param_refs)
                    .map_err(|e| CfmlError::runtime(format_pg_error("queryExecute: PostgreSQL query error", &e)))?
            }
            Err(e) => return Err(CfmlError::runtime(format_pg_error("queryExecute: PostgreSQL query error", &e))),
        };

        let columns: Vec<String> = if let Some(first_row) = rows.first() {
            first_row.columns().iter().map(|c| c.name().to_string()).collect()
        } else {
            vec![]
        };

        let mut result_rows: Vec<IndexMap<String, CfmlValue>> = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut row_map = IndexMap::new();
            for (i, col) in columns.iter().enumerate() {
                row_map.insert(col.clone(), postgres_row_to_cfml(row, i));
            }
            result_rows.push(row_map);
        }
        build_query_result(columns, result_rows, sql, return_type)
    } else {
        let mut total: i64 = 0;
        for stmt in &statements {
            let pg_params: Vec<PgParam> = stmt.params.iter().map(cfml_to_pg_param).collect();
            let param_refs: Vec<&(dyn postgres::types::ToSql + Sync)> = pg_params.iter()
                .map(|v| v as &(dyn postgres::types::ToSql + Sync))
                .collect();
            let prepared = pg_prepare_cached(client, cache, stmt.sql.as_str())
                .map_err(|e| CfmlError::runtime(format_pg_error("queryExecute: PostgreSQL prepare error", &e)))?;
            let affected = match client.execute(&prepared, &param_refs) {
                Ok(n) => n,
                Err(e) if is_stale_cached_plan(&e) => {
                    cache.remove(stmt.sql.as_str());
                    let prepared = pg_prepare_cached(client, cache, stmt.sql.as_str())
                        .map_err(|e| CfmlError::runtime(format_pg_error("queryExecute: PostgreSQL prepare error", &e)))?;
                    client.execute(&prepared, &param_refs)
                        .map_err(|e| CfmlError::runtime(format_pg_error("queryExecute: PostgreSQL error", &e)))?
                }
                Err(e) => return Err(CfmlError::runtime(format_pg_error("queryExecute: PostgreSQL error", &e))),
            };
            total += affected as i64;
        }
        build_mutation_result(total, 0, sql) // PG uses RETURNING, not last_insert_id
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
        CfmlValue::QueryColumn(_) => cfml_to_pg_param(val.query_column_scalar()),
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
fn postgres_row_to_cfml(row: &postgres::Row, col_idx: usize) -> CfmlValue {
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
    use tiberius::{Client, Config, AuthMethod};
    use tokio::runtime::Builder;
    use tokio_util::compat::TokioAsyncWriteCompatExt;

    // Parse URL: mssql://user:pass@host:port/database[?options] or sqlserver://...
    let clean_url = url.replace("mssql://", "").replace("sqlserver://", "");
    // Split off an optional query string before parsing the path components, so
    // the database name isn't polluted by `?trustServerCertificate=...` etc.
    let (clean_url, query) = match clean_url.split_once('?') {
        Some((b, q)) => (b.to_string(), Some(q.to_string())),
        None => (clean_url, None),
    };
    // Format: user:pass@host:port/database
    let (auth_part, host_db) = clean_url.split_once('@')
        .ok_or_else(|| CfmlError::runtime("queryExecute: MSSQL URL must be mssql://user:pass@host:port/database".to_string()))?;
    let (user, pass) = auth_part.split_once(':')
        .ok_or_else(|| CfmlError::runtime("queryExecute: MSSQL URL must include user:password".to_string()))?;
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

    let addr = format!("{}:{}", host, port);

    // Normalize params
    let (effective_params, _type_hints) = normalize_query_params(params_arg);

    // Create single-threaded tokio runtime (safe inside spawn_blocking)
    let rt = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CfmlError::runtime(format!("queryExecute: failed to create MSSQL runtime: {}", e)))?;

    rt.block_on(async {
        let tcp = tokio::net::TcpStream::connect(&addr).await
            .map_err(|e| CfmlError::runtime(format!("queryExecute: MSSQL connection error: {}", e)))?;
        let mut client = Client::connect(config, tcp.compat_write()).await
            .map_err(|e| CfmlError::runtime(format!("queryExecute: MSSQL connection error: {}", e)))?;

        if is_select_query(sql) {
            // Build query with inline params (tiberius simple query)
            let query_sql = mssql_inline_params(sql, &effective_params);
            let stream = client.simple_query(&query_sql).await
                .map_err(|e| CfmlError::runtime(format!("queryExecute: MSSQL query error: {}", e)))?;
            let result = stream.into_first_result().await
                .map_err(|e| CfmlError::runtime(format!("queryExecute: MSSQL result error: {}", e)))?;

            let columns: Vec<String> = if let Some(first_row) = result.first() {
                first_row.columns().iter()
                    .map(|c| c.name().to_string())
                    .collect()
            } else {
                vec![]
            };

            let mut rows: Vec<IndexMap<String, CfmlValue>> = Vec::with_capacity(result.len());
            for row in &result {
                let mut row_map = IndexMap::new();
                for (i, col) in columns.iter().enumerate() {
                    let val = mssql_column_to_cfml(row, i);
                    row_map.insert(col.clone(), val);
                }
                rows.push(row_map);
            }

            build_query_result(columns, rows, sql, return_type)
        } else {
            let query_sql = mssql_inline_params(sql, &effective_params);
            let result = client.simple_query(&query_sql).await
                .map_err(|e| CfmlError::runtime(format!("queryExecute: MSSQL error: {}", e)))?;
            let rows = result.into_first_result().await
                .map_err(|e| CfmlError::runtime(format!("queryExecute: MSSQL result error: {}", e)))?;
            build_mutation_result(rows.len() as i64, 0, sql)
        }
    })
}

#[cfg(feature = "mssql_db")]
fn mssql_inline_params(sql: &str, params: &[CfmlValue]) -> String {
    if params.is_empty() {
        return sql.to_string();
    }
    let mut result = String::with_capacity(sql.len() + params.len() * 10);
    let mut param_idx = 0;
    let bytes = sql.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] == b'?' && param_idx < params.len() {
            match &params[param_idx] {
                CfmlValue::Null => result.push_str("NULL"),
                CfmlValue::Int(n) => result.push_str(&n.to_string()),
                CfmlValue::Double(d) => result.push_str(&d.to_string()),
                CfmlValue::Bool(b) => result.push_str(if *b { "1" } else { "0" }),
                CfmlValue::String(s) => {
                    result.push('\'');
                    result.push_str(&s.replace('\'', "''"));
                    result.push('\'');
                }
                _ => {
                    let s = params[param_idx].as_string();
                    result.push('\'');
                    result.push_str(&s.replace('\'', "''"));
                    result.push('\'');
                }
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

#[cfg(feature = "mssql_db")]
fn mssql_column_to_cfml(row: &tiberius::Row, col_idx: usize) -> CfmlValue {
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
                .map_err(|e| CfmlError::runtime(format!("cftransaction: SQLite pool error: {}", e)))?;
            conn.execute_batch("BEGIN")
                .map_err(|e| CfmlError::runtime(format!("cftransaction: BEGIN error: {}", e)))?;
            Ok(TransactionConn::Sqlite(conn))
        }
        #[cfg(feature = "mysql_db")]
        DbDriver::Mysql(url) => {
            let pool = get_mysql_pool(&url)?;
            let mut conn = pool.get_conn()
                .map_err(|e| CfmlError::runtime(format!("cftransaction: MySQL pool error: {}", e)))?;
            use mysql::prelude::Queryable;
            conn.query_drop("BEGIN")
                .map_err(|e| CfmlError::runtime(format!("cftransaction: BEGIN error: {}", e)))?;
            Ok(TransactionConn::Mysql(conn))
        }
        #[cfg(feature = "postgres_db")]
        DbDriver::Postgres(url) => {
            let pool = get_postgres_pool(&url)?;
            let mut conn = pool.get()
                .map_err(|e| CfmlError::runtime(format!("cftransaction: PostgreSQL pool error: {}", e)))?;
            conn.client.simple_query("BEGIN")
                .map_err(|e| CfmlError::runtime(format!("cftransaction: BEGIN error: {}", e)))?;
            Ok(TransactionConn::Postgres(conn))
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
                .map_err(|e| CfmlError::runtime(format!("cftransaction: COMMIT error: {}", e)))
        }
        #[cfg(feature = "mysql_db")]
        TransactionConn::Mysql(c) => {
            use mysql::prelude::Queryable;
            c.query_drop("COMMIT")
                .map_err(|e| CfmlError::runtime(format!("cftransaction: COMMIT error: {}", e)))
        }
        #[cfg(feature = "postgres_db")]
        TransactionConn::Postgres(c) => {
            c.client.simple_query("COMMIT").map(|_| ())
                .map_err(|e| CfmlError::runtime(format!("cftransaction: COMMIT error: {}", e)))
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
                .map_err(|e| CfmlError::runtime(format!("cftransaction: ROLLBACK error: {}", e)))
        }
        #[cfg(feature = "mysql_db")]
        TransactionConn::Mysql(c) => {
            use mysql::prelude::Queryable;
            c.query_drop("ROLLBACK")
                .map_err(|e| CfmlError::runtime(format!("cftransaction: ROLLBACK error: {}", e)))
        }
        #[cfg(feature = "postgres_db")]
        TransactionConn::Postgres(c) => {
            c.client.simple_query("ROLLBACK").map(|_| ())
                .map_err(|e| CfmlError::runtime(format!("cftransaction: ROLLBACK error: {}", e)))
        }
        #[allow(unreachable_patterns)]
        _ => Ok(()),
    }
}

/// Execute a query using an existing transaction connection
#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn execute_with_transaction(conn: &mut TransactionConn, sql: &str, params_arg: &CfmlValue, return_type: &str) -> CfmlResult {
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
            execute_postgres_with_conn(&mut c.client, sql, params_arg, return_type)
        }
        #[allow(unreachable_patterns)]
        _ => Err(CfmlError::runtime("Transaction: unsupported driver".to_string())),
    }
}

#[cfg(feature = "sqlite")]
fn execute_sqlite_with_conn(conn: &rusqlite::Connection, sql: &str, params_arg: &CfmlValue, return_type: &str) -> CfmlResult {
    use rusqlite::types::Value as SqlValue;

    let bound_params = build_sqlite_params(params_arg, sql)?;

    if is_select_query(sql) {
        let mut stmt = conn.prepare(sql)
            .map_err(|e| CfmlError::runtime(format!("queryExecute: SQL error: {}", e)))?;
        let column_count = stmt.column_count();
        let columns: Vec<String> = (0..column_count)
            .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
            .collect();
        let rows_result: Result<Vec<IndexMap<String, CfmlValue>>, _> = stmt
            .query_map(rusqlite::params_from_iter(bound_params.iter()), |row| {
                let mut row_map = IndexMap::new();
                for (i, col) in columns.iter().enumerate() {
                    let val: SqlValue = row.get_unwrap(i);
                    row_map.insert(col.clone(), sqlite_to_cfml(val));
                }
                Ok(row_map)
            })
            .map_err(|e| CfmlError::runtime(format!("queryExecute: query error: {}", e)))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CfmlError::runtime(format!("queryExecute: row error: {}", e)));
        let rows = rows_result?;
        build_query_result(columns, rows, sql, return_type)
    } else {
        let affected = conn.execute(sql, rusqlite::params_from_iter(bound_params.iter()))
            .map_err(|e| CfmlError::runtime(format!("queryExecute: SQL error: {}", e)))?;
        let last_id = conn.last_insert_rowid();
        build_mutation_result(affected as i64, last_id, sql)
    }
}

#[cfg(feature = "mysql_db")]
fn execute_mysql_with_conn(conn: &mut mysql::PooledConn, sql: &str, params_arg: &CfmlValue, return_type: &str) -> CfmlResult {
    use mysql::*;
    use mysql::prelude::*;

    let params = match params_arg {
        CfmlValue::Array(arr) => {
            let vals: Vec<mysql::Value> = arr.iter().map(|v| cfml_to_mysql_value(&v)).collect();
            Params::Positional(vals)
        }
        CfmlValue::Struct(map) => {
            let mut named: HashMap<Vec<u8>, mysql::Value> = HashMap::new();
            for (k, v) in map.iter() {
                // Unwrap cfqueryparam-style param structs before binding.
                let val = cfqueryparam_unwrap(&v);
                named.insert(k.as_bytes().to_vec(), cfml_to_mysql_value(&val));
            }
            Params::Named(named)
        }
        _ => Params::Empty,
    };

    if is_select_query(sql) {
        let result: Vec<Row> = conn.exec(sql, &params)
            .map_err(|e| CfmlError::runtime(format!("queryExecute: MySQL query error: {}", e)))?;
        let columns: Vec<String> = if let Some(first_row) = result.first() {
            first_row.columns_ref().iter()
                .map(|c| c.name_str().to_string())
                .collect()
        } else {
            vec![]
        };
        let mut rows: Vec<IndexMap<String, CfmlValue>> = Vec::with_capacity(result.len());
        for row in &result {
            let mut row_map = IndexMap::new();
            for (i, col) in columns.iter().enumerate() {
                let val: mysql::Value = row.get(i).unwrap_or(mysql::Value::NULL);
                row_map.insert(col.clone(), mysql_value_to_cfml(val));
            }
            rows.push(row_map);
        }
        build_query_result(columns, rows, sql, return_type)
    } else {
        conn.exec_drop(sql, &params)
            .map_err(|e| CfmlError::runtime(format!("queryExecute: MySQL error: {}", e)))?;
        let affected = conn.affected_rows() as i64;
        let last_id = conn.last_insert_id() as i64;
        build_mutation_result(affected, last_id, sql)
    }
}

#[cfg(feature = "postgres_db")]
fn execute_postgres_with_conn(client: &mut postgres::Client, sql: &str, params_arg: &CfmlValue, return_type: &str) -> CfmlResult {
    // Transaction path: a transaction is short-lived, so use a local statement
    // cache (still de-dupes repeated SQL within the transaction).
    let mut cache = PgStmtCache::new();
    run_postgres_statements(client, &mut cache, sql, params_arg, return_type)
}

// -----------------------------------------------
// Shared result builders
// -----------------------------------------------

#[cfg(any(feature = "sqlite", feature = "mysql_db", feature = "postgres_db", feature = "mssql_db"))]
fn build_query_result(columns: Vec<String>, rows: Vec<IndexMap<String, CfmlValue>>, sql: &str, return_type: &str) -> CfmlResult {
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
        let mut out: IndexMap<String, CfmlValue> = IndexMap::new();
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
            Ok(CfmlValue::strukt(IndexMap::new()))
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
    let mut result = IndexMap::new();
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

            let columns = vec![
                "name".to_string(),
                "directory".to_string(),
                "type".to_string(),
                "size".to_string(),
                "datelastmodified".to_string(),
            ];
            let mut rows: Vec<IndexMap<String, CfmlValue>> = Vec::new();

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
                recurse: bool,
                rows: &mut Vec<IndexMap<String, CfmlValue>>,
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

                    let should_include = is_dir || matches_glob(&name, filter);

                    if should_include {
                        let mut row = IndexMap::new();
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
                            list_dir(&entry.path(), filter, recurse, rows, visited)?;
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
            list_dir(Path::new(&directory), &filter, recurse, &mut rows, &mut visited)?;

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
        "write" => fn_file_write(vec![
            CfmlValue::string(str_attr("file")),
            CfmlValue::string(str_attr("output")),
        ]),
        "append" => {
            let mut data = str_attr("output");
            // addNewLine appends a trailing newline; defaults off to match the
            // fileAppend() BIF. The tag form is delivery-equivalent.
            if bool_attr("addnewline") {
                data.push('\n');
            }
            fn_file_append(vec![CfmlValue::string(str_attr("file")), CfmlValue::string(data)])
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

fn base64_encode_bytes(data: &[u8]) -> String {
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

fn base64_decode_bytes(s: &str) -> Vec<u8> {
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

#[cfg(feature = "xml")]
fn fn_xml_parse(args: Vec<CfmlValue>) -> CfmlResult {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let xml_str = get_str(&args, 0);
    let mut reader = Reader::from_str(&xml_str);

    let mut stack: Vec<IndexMap<String, CfmlValue>> = Vec::new();
    let mut root: Option<IndexMap<String, CfmlValue>> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let mut element = IndexMap::new();
                let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                element.insert("xmlName".to_string(), CfmlValue::string(tag_name));
                element.insert("xmlType".to_string(), CfmlValue::string("ELEMENT".to_string()));
                element.insert("xmlText".to_string(), CfmlValue::string(String::new()));
                element.insert("xmlChildren".to_string(), CfmlValue::array(Vec::new()));

                let mut attrs = IndexMap::new();
                for attr in e.attributes().flatten() {
                    let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                    let val = String::from_utf8_lossy(&attr.value).to_string();
                    attrs.insert(key, CfmlValue::string(val));
                }
                element.insert("xmlAttributes".to_string(), CfmlValue::strukt(attrs));

                stack.push(element);
            }
            Ok(Event::End(_)) => {
                if let Some(completed) = stack.pop() {
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
                let mut element = IndexMap::new();
                let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                element.insert("xmlName".to_string(), CfmlValue::string(tag_name));
                element.insert("xmlType".to_string(), CfmlValue::string("ELEMENT".to_string()));
                element.insert("xmlText".to_string(), CfmlValue::string(String::new()));
                element.insert("xmlChildren".to_string(), CfmlValue::array(Vec::new()));

                let mut attrs = IndexMap::new();
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
            let mut doc = IndexMap::new();
            doc.insert("xmlRoot".to_string(), CfmlValue::strukt(root_element));
            doc.insert("xmlType".to_string(), CfmlValue::string("DOCUMENT".to_string()));
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
    let mut doc = IndexMap::new();
    doc.insert("xmlComment".to_string(), CfmlValue::string(String::new()));
    doc.insert("__xmlDoc".to_string(), CfmlValue::Bool(true));
    let mut doc_type = IndexMap::new();
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
    let mut elem = IndexMap::new();
    elem.insert("xmlName".to_string(), CfmlValue::string(child_name));
    elem.insert("xmlNsPrefix".to_string(), CfmlValue::string(String::new()));
    elem.insert("xmlNsURI".to_string(), CfmlValue::string(namespace));
    elem.insert("xmlText".to_string(), CfmlValue::string(String::new()));
    elem.insert("xmlComment".to_string(), CfmlValue::string(String::new()));
    elem.insert("xmlCData".to_string(), CfmlValue::string(String::new()));
    elem.insert("xmlAttributes".to_string(), CfmlValue::strukt(IndexMap::new()));
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
                let mut element = IndexMap::new();
                element.insert("xmlName".to_string(), CfmlValue::string(tag_name));
                element.insert("xmlType".to_string(), CfmlValue::string("ELEMENT".to_string()));

                let mut attrs = IndexMap::new();
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
                let mut element = IndexMap::new();
                element.insert("xmlName".to_string(), CfmlValue::string("#text".to_string()));
                element.insert("xmlType".to_string(), CfmlValue::string("TEXT".to_string()));
                element.insert("xmlText".to_string(), CfmlValue::string(text));
                element.insert("xmlChildren".to_string(), CfmlValue::array(Vec::new()));
                element.insert("xmlAttributes".to_string(), CfmlValue::strukt(IndexMap::new()));
                Some(CfmlValue::strukt(element))
            }
            _ => None,
        }
    }

    let html_str = get_str(&args, 0);
    let html = Html::parse_document(&html_str);

    let mut doc = IndexMap::new();
    doc.insert("xmlType".to_string(), CfmlValue::string("DOCUMENT".to_string()));

    let root_el = html.root_element();
    if let Some(root_cfml) = walk_node(*root_el) {
        doc.insert("xmlRoot".to_string(), root_cfml);
    } else {
        let mut root = IndexMap::new();
        root.insert("xmlName".to_string(), CfmlValue::string("html".to_string()));
        root.insert("xmlType".to_string(), CfmlValue::string("ELEMENT".to_string()));
        root.insert("xmlText".to_string(), CfmlValue::string(String::new()));
        root.insert("xmlChildren".to_string(), CfmlValue::array(Vec::new()));
        root.insert("xmlAttributes".to_string(), CfmlValue::strukt(IndexMap::new()));
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
                    v.push(CfmlValue::string(String::new()));
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
            let mut map = IndexMap::new();
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
            let mut result = IndexMap::new();
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
    Ok(CfmlValue::Double(total_days))
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
                CfmlQueryData { columns: d.columns.clone(), data: new_data, sql: None }
            });
            Ok(CfmlValue::Query(CfmlQuery::from_data(sliced)))
        }
        _ => Err(CfmlError::runtime("querySlice() requires a query".to_string())),
    }
}

fn fn_query_get_result(_args: Vec<CfmlValue>) -> CfmlResult {
    // Returns metadata about last query execution
    let mut result = IndexMap::new();
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
        Some(CfmlValue::Query(_)) => Ok(CfmlValue::Int(0)),
        _ => Ok(CfmlValue::Int(0)),
    }
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
    let file_name = std::path::Path::new(&path)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
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

fn fn_write_log(args: Vec<CfmlValue>) -> CfmlResult {
    let text = get_str(&args, 0);
    eprintln!("[LOG] {}", text);
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

fn fn_set_locale(args: Vec<CfmlValue>) -> CfmlResult {
    let locale = get_str(&args, 0);
    let code = cfml_locale_to_code(&locale);
    Ok(CfmlValue::string(code))
}

fn fn_get_locale(_args: Vec<CfmlValue>) -> CfmlResult {
    // Lucee returns lowercase friendly name by default
    Ok(CfmlValue::string(locale_code_to_friendly("en_US")))
}

fn fn_set_time_zone(args: Vec<CfmlValue>) -> CfmlResult {
    let _tz = get_str(&args, 0);
    Ok(CfmlValue::Null)
}

fn fn_application_stop(_args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Null)
}

fn fn_get_application_metadata(_args: Vec<CfmlValue>) -> CfmlResult {
    let mut meta = IndexMap::new();
    meta.insert("name".to_string(), CfmlValue::string(String::new()));
    Ok(CfmlValue::strukt(meta))
}

fn fn_trace(args: Vec<CfmlValue>) -> CfmlResult {
    let text = get_str(&args, 0);
    eprintln!("[TRACE] {}", text);
    Ok(CfmlValue::Null)
}

// ---- File functions ----

fn fn_file_read_binary(args: Vec<CfmlValue>) -> CfmlResult {
    let path = get_str(&args, 0);
    match std::fs::read(&path) {
        Ok(bytes) => Ok(CfmlValue::Binary(bytes)),
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
    let mut handle = IndexMap::new();
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
    let mut result = IndexMap::new();
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

    let mut result = IndexMap::new();
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

        let email = if attachments.is_empty() {
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

        let mut transport_builder = SmtpTransport::builder_dangerous(smtp_server)
            .port(port);

        if let (Some(ref user), Some(ref pass)) = (&username, &password) {
            transport_builder = transport_builder.credentials(
                Credentials::new(user.clone(), pass.clone())
            );
        }

        let mailer = transport_builder.build();
        match mailer.send(&email) {
            Ok(_) => eprintln!("[CFMAIL] Sent successfully via {}", smtp_server),
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
    let mut meta = IndexMap::new();
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

fn fn_ls_currency_format(args: Vec<CfmlValue>) -> CfmlResult {
    let n = get_float(&args, 0);
    let currency_type = if args.len() > 1 {
        get_str(&args, 1).to_lowercase()
    } else {
        "local".to_string()
    };

    let negative = n < 0.0;
    let abs_n = n.abs();
    let formatted_num = format!("{:.2}", abs_n);
    let parts: Vec<&str> = formatted_num.split('.').collect();
    let int_with_commas = add_thousands_separator(parts[0]);
    let decimal = parts.get(1).unwrap_or(&"00");

    // Lucee format (en_US locale): "$1,234.56" local, "USD 1,234.56" international,
    // "1,234.56" none. Negatives wrapped in parens: "($99.99)", "(USD 99.99)".
    match currency_type.as_str() {
        "international" => {
            if negative {
                Ok(CfmlValue::string(format!("USD ({}.{})", int_with_commas, decimal)))
            } else {
                Ok(CfmlValue::string(format!("USD {}.{}", int_with_commas, decimal)))
            }
        }
        "none" => {
            if negative {
                Ok(CfmlValue::string(format!("-{}.{}", int_with_commas, decimal)))
            } else {
                Ok(CfmlValue::string(format!("{}.{}", int_with_commas, decimal)))
            }
        }
        _ => {
            if negative {
                Ok(CfmlValue::string(format!("(${}.{})", int_with_commas, decimal)))
            } else {
                Ok(CfmlValue::string(format!("${}.{}", int_with_commas, decimal)))
            }
        }
    }
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
    let pass_args = if args.len() >= 2 {
        vec![args[0].clone(), args[1].clone()]
    } else {
        vec![args.first().cloned().unwrap_or(CfmlValue::string(String::new()))]
    };
    fn_number_format(pass_args)
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
        let exists = s.keys().iter().any(|k| k.to_lowercase() == key_str);
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

                let mut row = IndexMap::new();
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
    }
}
