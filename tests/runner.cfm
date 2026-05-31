<cfscript>
writeOutput("============================================================" & chr(10));
writeOutput("RustCFML Test Suite" & chr(10));
writeOutput("============================================================" & chr(10) & chr(10));

include "harness.cfm";

// --- cfconfig ---
try { include "config/test_cfconfig_loading.cfm"; } catch (any e) { writeOutput("ERROR | config/test_cfconfig_loading.cfm | " & e.message & chr(10)); }
try { include "config/test_cfconfig_datasource.cfm"; } catch (any e) { writeOutput("ERROR | config/test_cfconfig_datasource.cfm | " & e.message & chr(10)); }
try { include "config/test_cfconfig_security.cfm"; } catch (any e) { writeOutput("ERROR | config/test_cfconfig_security.cfm | " & e.message & chr(10)); }

// --- Core Language ---
try { include "core/test_variables.cfm"; } catch (any e) { writeOutput("ERROR | core/test_variables.cfm | " & e.message & chr(10)); }
try { include "core/test_access_identifiers.cfm"; } catch (any e) { writeOutput("ERROR | core/test_access_identifiers.cfm | " & e.message & chr(10)); }
try { include "core/test_function_scope_capture.cfm"; } catch (any e) { writeOutput("ERROR | core/test_function_scope_capture.cfm | " & e.message & chr(10)); }
try { include "core/test_closure_env_leak.cfm"; } catch (any e) { writeOutput("ERROR | core/test_closure_env_leak.cfm | " & e.message & chr(10)); }
try { include "core/test_compound_assignment.cfm"; } catch (any e) { writeOutput("ERROR | core/test_compound_assignment.cfm | " & e.message & chr(10)); }
try { include "core/test_struct_method_sequential.cfm"; } catch (any e) { writeOutput("ERROR | core/test_struct_method_sequential.cfm | " & e.message & chr(10)); }
try { include "core/test_include_scope_capture.cfm"; } catch (any e) { writeOutput("ERROR | core/test_include_scope_capture.cfm | " & e.message & chr(10)); }
try { include "core/test_operators.cfm"; } catch (any e) { writeOutput("ERROR | core/test_operators.cfm | " & e.message & chr(10)); }
try { include "core/test_control_flow.cfm"; } catch (any e) { writeOutput("ERROR | core/test_control_flow.cfm | " & e.message & chr(10)); }
try { include "core/test_error_handling.cfm"; } catch (any e) { writeOutput("ERROR | core/test_error_handling.cfm | " & e.message & chr(10)); }
try { include "core/test_functions.cfm"; } catch (any e) { writeOutput("ERROR | core/test_functions.cfm | " & e.message & chr(10)); }
try { include "core/test_arrow_functions.cfm"; } catch (any e) { writeOutput("ERROR | core/test_arrow_functions.cfm | " & e.message & chr(10)); }
try { include "core/test_arguments_writeback.cfm"; } catch (any e) { writeOutput("ERROR | core/test_arguments_writeback.cfm | " & e.message & chr(10)); }
try { include "core/test_language_features.cfm"; } catch (any e) { writeOutput("ERROR | core/test_language_features.cfm | " & e.message & chr(10)); }
try { include "core/test_scopes.cfm"; } catch (any e) { writeOutput("ERROR | core/test_scopes.cfm | " & e.message & chr(10)); }
try { include "core/test_server_scope.cfm"; } catch (any e) { writeOutput("ERROR | core/test_server_scope.cfm | " & e.message & chr(10)); }
try { include "core/test_localmode.cfm"; } catch (any e) { writeOutput("ERROR | core/test_localmode.cfm | " & e.message & chr(10)); }
try { include "core/test_error_context.cfm"; } catch (any e) { writeOutput("ERROR | core/test_error_context.cfm | " & e.message & chr(10)); }
try { include "core/test_null_coalescing.cfm"; } catch (any e) { writeOutput("ERROR | core/test_null_coalescing.cfm | " & e.message & chr(10)); }

// --- Data Types ---
try { include "types/test_null.cfm"; } catch (any e) { writeOutput("ERROR | types/test_null.cfm | " & e.message & chr(10)); }
try { include "types/test_boolean.cfm"; } catch (any e) { writeOutput("ERROR | types/test_boolean.cfm | " & e.message & chr(10)); }
try { include "types/test_numeric.cfm"; } catch (any e) { writeOutput("ERROR | types/test_numeric.cfm | " & e.message & chr(10)); }
try { include "types/test_string.cfm"; } catch (any e) { writeOutput("ERROR | types/test_string.cfm | " & e.message & chr(10)); }
try { include "types/test_array.cfm"; } catch (any e) { writeOutput("ERROR | types/test_array.cfm | " & e.message & chr(10)); }
try { include "types/test_array_append_grow.cfm"; } catch (any e) { writeOutput("ERROR | types/test_array_append_grow.cfm | " & e.message & chr(10)); }
try { include "types/test_array_reference_semantics.cfm"; } catch (any e) { writeOutput("ERROR | types/test_array_reference_semantics.cfm | " & e.message & chr(10)); }
try { include "types/test_struct.cfm"; } catch (any e) { writeOutput("ERROR | types/test_struct.cfm | " & e.message & chr(10)); }
try { include "types/test_struct_reference_semantics.cfm"; } catch (any e) { writeOutput("ERROR | types/test_struct_reference_semantics.cfm | " & e.message & chr(10)); }
try { include "types/test_nested_writeback.cfm"; } catch (any e) { writeOutput("ERROR | types/test_nested_writeback.cfm | " & e.message & chr(10)); }
try { include "types/test_query.cfm"; } catch (any e) { writeOutput("ERROR | types/test_query.cfm | " & e.message & chr(10)); }
try { include "types/test_query_column.cfm"; } catch (any e) { writeOutput("ERROR | types/test_query_column.cfm | " & e.message & chr(10)); }
try { include "types/test_binary.cfm"; } catch (any e) { writeOutput("ERROR | types/test_binary.cfm | " & e.message & chr(10)); }
try { include "types/test_hash_in_strings.cfm"; } catch (any e) { writeOutput("ERROR | types/test_hash_in_strings.cfm | " & e.message & chr(10)); }
try { include "comments/test_hash_in_comments.cfm"; } catch (any e) { writeOutput("ERROR | comments/test_hash_in_comments.cfm | " & e.message & chr(10)); }

// --- Standard Library ---
try { include "stdlib/test_string_functions.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_string_functions.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_string_functions_regex.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_string_functions_regex.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_string_functions_encoding.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_string_functions_encoding.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_array_functions.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_array_functions.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_array_higher_order.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_array_higher_order.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_struct_functions.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_struct_functions.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_struct_higher_order.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_struct_higher_order.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_math_functions.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_math_functions.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_date_functions.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_date_functions.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_list_functions.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_list_functions.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_list_higher_order.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_list_higher_order.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_query_functions.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_query_functions.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_query_higher_order.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_query_higher_order.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_type_checking.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_type_checking.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_conversion.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_conversion.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_json.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_json.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_file_io.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_file_io.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_security.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_security.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_password_hashing.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_password_hashing.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_xml.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_xml.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_utility.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_utility.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_encoding_functions.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_encoding_functions.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_query_mutations.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_query_mutations.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_date_functions_extra.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_date_functions_extra.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_locale_functions.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_locale_functions.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_cache_functions.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_cache_functions.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_higher_order_functions.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_higher_order_functions.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_bitmask_functions.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_bitmask_functions.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_xml_dom_functions.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_xml_dom_functions.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_misc_functions.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_misc_functions.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_valuelist_functions.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_valuelist_functions.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_callstack.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_callstack.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_precisionevaluate.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_precisionevaluate.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_htmlparse.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_htmlparse.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_ini_functions.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_ini_functions.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_directorylist.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_directorylist.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_cfhttp.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_cfhttp.cfm | " & e.message & chr(10)); }

// --- Function References ---
try { include "functions/test_function_references.cfm"; } catch (any e) { writeOutput("ERROR | functions/test_function_references.cfm | " & e.message & chr(10)); }

// --- Member Functions ---
try { include "members/test_string_members.cfm"; } catch (any e) { writeOutput("ERROR | members/test_string_members.cfm | " & e.message & chr(10)); }
try { include "members/test_array_members.cfm"; } catch (any e) { writeOutput("ERROR | members/test_array_members.cfm | " & e.message & chr(10)); }
try { include "members/test_struct_members.cfm"; } catch (any e) { writeOutput("ERROR | members/test_struct_members.cfm | " & e.message & chr(10)); }
try { include "members/test_number_members.cfm"; } catch (any e) { writeOutput("ERROR | members/test_number_members.cfm | " & e.message & chr(10)); }

// --- OOP ---
try { include "oop/test_components.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_components.cfm | " & e.message & chr(10)); }
try { include "oop/test_inheritance.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_inheritance.cfm | " & e.message & chr(10)); }
try { include "oop/test_inherited_helpers.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_inherited_helpers.cfm | " & e.message & chr(10)); }
try { include "oop/test_interfaces.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_interfaces.cfm | " & e.message & chr(10)); }
try { include "oop/test_metadata.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_metadata.cfm | " & e.message & chr(10)); }
try { include "oop/test_property_attributes.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_property_attributes.cfm | " & e.message & chr(10)); }
try { include "oop/test_struct_method_dispatch.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_struct_method_dispatch.cfm | " & e.message & chr(10)); }
try { include "oop/test_external_prop.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_external_prop.cfm | " & e.message & chr(10)); }
try { include "oop/test_repeated_instantiation.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_repeated_instantiation.cfm | " & e.message & chr(10)); }

// --- Tags ---
try { include "tags/test_tags_basic.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_basic.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_control.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_control.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_include.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_include.cfm | " & e.message & chr(10)); }
try { include "tags/test_cfinclude_css.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_cfinclude_css.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cffunction_hoisting.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cffunction_hoisting.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_savecontent.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_savecontent.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_param.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_param.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_param_dynamic.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_param_dynamic.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_misc.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_misc.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_customtag.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_customtag.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_buffer_recovery.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_buffer_recovery.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfexecute.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfexecute.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfmail.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfmail.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfcache.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfcache.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfstoredproc.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfstoredproc.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfqueryparam_attribute_collection.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfqueryparam_attribute_collection.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfimport.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfimport.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfthread.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfthread.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfscript_statements.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfscript_statements.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfzip.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfzip.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_tld.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_tld.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_whitespace.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_whitespace.cfm | " & e.message & chr(10)); }

// --- Includes ---
try { include "includes/test_variables_scope_includes.cfm"; } catch (any e) { writeOutput("ERROR | includes/test_variables_scope_includes.cfm | " & e.message & chr(10)); }
try { include "includes/test_named_args_includes.cfm"; } catch (any e) { writeOutput("ERROR | includes/test_named_args_includes.cfm | " & e.message & chr(10)); }

// --- Java Shims ---
try { include "java_shims/test_all.cfm"; } catch (any e) { writeOutput("ERROR | java_shims/test_all.cfm | " & e.message & chr(10)); }
try { include "java_shims/test_comprehensive.cfm"; } catch (any e) { writeOutput("ERROR | java_shims/test_comprehensive.cfm | " & e.message & chr(10)); }
try { include "java_shims/test_more.cfm"; } catch (any e) { writeOutput("ERROR | java_shims/test_more.cfm | " & e.message & chr(10)); }
try { include "java_shims/test_security.cfm"; } catch (any e) { writeOutput("ERROR | java_shims/test_security.cfm | " & e.message & chr(10)); }
try { include "java_shims/test_stringbuilder.cfm"; } catch (any e) { writeOutput("ERROR | java_shims/test_stringbuilder.cfm | " & e.message & chr(10)); }
try { include "java_shims/test_system.cfm"; } catch (any e) { writeOutput("ERROR | java_shims/test_system.cfm | " & e.message & chr(10)); }
try { include "java_shims/test_concurrent_map.cfm"; } catch (any e) { writeOutput("ERROR | java_shims/test_concurrent_map.cfm | " & e.message & chr(10)); }

// --- Engine Compatibility ---
try { include "compat_engine/test_math_functions.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_math_functions.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_string_functions.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_string_functions.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_struct_functions.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_struct_functions.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_array_functions.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_array_functions.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_list_functions.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_list_functions.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_query_functions.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_query_functions.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_date_functions.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_date_functions.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_type_checking.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_type_checking.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_json.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_json.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_type_casting.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_type_casting.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_language_operators.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_language_operators.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_language_controlflow.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_language_controlflow.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_language_closures.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_language_closures.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_file_functions.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_file_functions.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_encoding_functions.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_encoding_functions.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_collection_functions.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_collection_functions.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_edge_cases.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_edge_cases.cfm | " & e.message & chr(10)); }
try { include "compat_engine/test_scope_behavior.cfm"; } catch (any e) { writeOutput("ERROR | compat_engine/test_scope_behavior.cfm | " & e.message & chr(10)); }
try { include "native/test_native_fn.cfm"; } catch (any e) { writeOutput("ERROR | native/test_native_fn.cfm | " & e.message & chr(10)); }
try { include "native/test_native_class.cfm"; } catch (any e) { writeOutput("ERROR | native/test_native_class.cfm | " & e.message & chr(10)); }
try { include "native/test_native_thread.cfm"; } catch (any e) { writeOutput("ERROR | native/test_native_thread.cfm | " & e.message & chr(10)); }
try { include "native/test_cfc_extends_rust.cfm"; } catch (any e) { writeOutput("ERROR | native/test_cfc_extends_rust.cfm | " & e.message & chr(10)); }
// S3 tests live under tests/s3/ but are excluded from the default runner —
// they need live credentials (AWS / R2 / MinIO) to pass. Run the full S3
// harness via /tmp/rustcfml-s3-harness/run_live.sh (see docs/s3.md), or
// invoke a single file directly:
//   cargo run -- tests/s3/test_s3_functions.cfm

// --- Cross-engine compatibility (Wheels framework gaps) ---
// These tests exercise CFML behaviors Wheels depends on that pass on Lucee
// 7 but are (or were) gaps in RustCFML. Registered last as a cluster;
// none of them aborts the run on RustCFML, so ordering here is not
// load-bearing -- each one fails its own assertions in isolation and the
// run still reaches printSummary().
//
//   - local_at_template_scope, metadata_name_value, script_syntax_body:
//     parse/behavioral gaps that 0.20.x has since closed; kept as
//     regression tests -- they pass on both engines now.
//   - expandpath_trailing_slash: behavioral gap, still open on 0.20.2 --
//     for an EXISTING path, expandPath canonicalizes and drops the trailing
//     slash, so the Wheels "appDir & '../plugins'" shape fuses into a
//     malformed path. Fails its assertions but does NOT abort the run.
//   - forin_member_loop_var: two distinct for-in gaps on 0.20.2, both
//     non-fatal here. (1) A plain member-path loop var (ctx.item) PARSES
//     but never iterates -- the body is silently skipped. (2) A `this`-
//     headed loop var fails to PARSE, but that parse error is CONTAINED
//     inside a runtime-instantiated fixture CFC (ForInThisLoopFixture),
//     which degrades to a non-object silently instead of aborting. Both
//     modes fail their assertions without taking down the run.
try { include "core/test_local_at_template_scope.cfm"; } catch (any e) { writeOutput("ERROR | core/test_local_at_template_scope.cfm | " & e.message & chr(10)); }
try { include "oop/test_metadata_name_value.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_metadata_name_value.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_script_syntax_body.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_script_syntax_body.cfm | " & e.message & chr(10)); }
try { include "functions/test_expandpath_trailing_slash.cfm"; } catch (any e) { writeOutput("ERROR | functions/test_expandpath_trailing_slash.cfm | " & e.message & chr(10)); }
try { include "core/test_forin_member_loop_var.cfm"; } catch (any e) { writeOutput("ERROR | core/test_forin_member_loop_var.cfm | " & e.message & chr(10)); }

// --- v0.34.3 round: Wheels now parses, constructs, and boots its full app
//     lifecycle + DI on RustCFML. This is the remaining language gap found
//     on the way to serving a request. ---
//   - undefined_var_autovivify: assigning to a member path of an UNDECLARED
//     variable (initArgs.path = "wheels" in Application.cfc.onApplicationStart)
//     must auto-create it as a struct. RustCFML throws "Variable is undefined"
//     for everything except the `local` scope. Wrapped in try/catch so the
//     throw fails its assertions without aborting the run.
try { include "core/test_undefined_var_autovivify.cfm"; } catch (any e) { writeOutput("ERROR | core/test_undefined_var_autovivify.cfm | " & e.message & chr(10)); }
//   - cfinvoke_component_attr: `component` is a soft keyword on Lucee/ACF/BoxLang
//     but a hard reserved keyword on RustCFML, so `cfinvoke component=...` (and a
//     bare `component = x`) fail to PARSE. Wheels' Global.cfc::$cfinvoke() uses
//     this exact shape, so wheels.Global degrades to a non-object and the whole
//     framework boots-but-serves-empty. Parse failure contained in a fixture.
try { include "tags/test_cfinvoke_component_attr.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_cfinvoke_component_attr.cfm | " & e.message & chr(10)); }

printSummary();
</cfscript>
