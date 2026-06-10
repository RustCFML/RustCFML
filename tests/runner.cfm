<cfscript>
writeOutput("============================================================" & chr(10));
writeOutput("RustCFML Test Suite" & chr(10));
writeOutput("============================================================" & chr(10) & chr(10));

include "harness.cfm";

// --- cfconfig ---
try { include "config/test_cfconfig_loading.cfm"; } catch (any e) { writeOutput("ERROR | config/test_cfconfig_loading.cfm | " & e.message & chr(10)); }
try { include "config/test_cfconfig_datasource.cfm"; } catch (any e) { writeOutput("ERROR | config/test_cfconfig_datasource.cfm | " & e.message & chr(10)); }
try { include "config/test_cfconfig_security.cfm"; } catch (any e) { writeOutput("ERROR | config/test_cfconfig_security.cfm | " & e.message & chr(10)); }
try { include "config/test_app_datasources.cfm"; } catch (any e) { writeOutput("ERROR | config/test_app_datasources.cfm | " & e.message & chr(10)); }

// --- Core Language ---
try { include "core/test_variables.cfm"; } catch (any e) { writeOutput("ERROR | core/test_variables.cfm | " & e.message & chr(10)); }
try { include "core/test_access_identifiers.cfm"; } catch (any e) { writeOutput("ERROR | core/test_access_identifiers.cfm | " & e.message & chr(10)); }
try { include "core/test_function_scope_capture.cfm"; } catch (any e) { writeOutput("ERROR | core/test_function_scope_capture.cfm | " & e.message & chr(10)); }
try { include "core/test_closure_env_leak.cfm"; } catch (any e) { writeOutput("ERROR | core/test_closure_env_leak.cfm | " & e.message & chr(10)); }
try { include "core/test_compound_assignment.cfm"; } catch (any e) { writeOutput("ERROR | core/test_compound_assignment.cfm | " & e.message & chr(10)); }
try { include "core/test_undeclared_named_args.cfm"; } catch (any e) { writeOutput("ERROR | core/test_undeclared_named_args.cfm | " & e.message & chr(10)); }
try { include "core/test_struct_method_sequential.cfm"; } catch (any e) { writeOutput("ERROR | core/test_struct_method_sequential.cfm | " & e.message & chr(10)); }
try { include "core/test_include_scope_capture.cfm"; } catch (any e) { writeOutput("ERROR | core/test_include_scope_capture.cfm | " & e.message & chr(10)); }
try { include "core/test_operators.cfm"; } catch (any e) { writeOutput("ERROR | core/test_operators.cfm | " & e.message & chr(10)); }
try { include "core/test_subscript_autovivify.cfm"; } catch (any e) { writeOutput("ERROR | core/test_subscript_autovivify.cfm | " & e.message & chr(10)); }
try { include "core/test_control_flow.cfm"; } catch (any e) { writeOutput("ERROR | core/test_control_flow.cfm | " & e.message & chr(10)); }
try { include "core/test_cfloop_negative_step.cfm"; } catch (any e) { writeOutput("ERROR | core/test_cfloop_negative_step.cfm | " & e.message & chr(10)); }
try { include "core/test_cfloop_array_item_index.cfm"; } catch (any e) { writeOutput("ERROR | core/test_cfloop_array_item_index.cfm | " & e.message & chr(10)); }
try { include "core/test_cfloop_collection_item_index.cfm"; } catch (any e) { writeOutput("ERROR | core/test_cfloop_collection_item_index.cfm | " & e.message & chr(10)); }
try { include "core/test_error_handling.cfm"; } catch (any e) { writeOutput("ERROR | core/test_error_handling.cfm | " & e.message & chr(10)); }
try { include "core/test_builtin_shadowing.cfm"; } catch (any e) { writeOutput("ERROR | core/test_builtin_shadowing.cfm | " & e.message & chr(10)); }
try { include "core/test_functions.cfm"; } catch (any e) { writeOutput("ERROR | core/test_functions.cfm | " & e.message & chr(10)); }
try { include "core/test_arrow_functions.cfm"; } catch (any e) { writeOutput("ERROR | core/test_arrow_functions.cfm | " & e.message & chr(10)); }
try { include "core/test_arguments_writeback.cfm"; } catch (any e) { writeOutput("ERROR | core/test_arguments_writeback.cfm | " & e.message & chr(10)); }
try { include "core/test_argument_reference_nested.cfm"; } catch (any e) { writeOutput("ERROR | core/test_argument_reference_nested.cfm | " & e.message & chr(10)); }
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
try { include "types/test_ordered_struct_literals.cfm"; } catch (any e) { writeOutput("ERROR | types/test_ordered_struct_literals.cfm | " & e.message & chr(10)); }
try { include "types/test_nested_writeback.cfm"; } catch (any e) { writeOutput("ERROR | types/test_nested_writeback.cfm | " & e.message & chr(10)); }
try { include "types/test_query.cfm"; } catch (any e) { writeOutput("ERROR | types/test_query.cfm | " & e.message & chr(10)); }
try { include "types/test_query_column.cfm"; } catch (any e) { writeOutput("ERROR | types/test_query_column.cfm | " & e.message & chr(10)); }
try { include "types/test_query_reference.cfm"; } catch (any e) { writeOutput("ERROR | types/test_query_reference.cfm | " & e.message & chr(10)); }
try { include "types/test_binary.cfm"; } catch (any e) { writeOutput("ERROR | types/test_binary.cfm | " & e.message & chr(10)); }
try { include "types/test_hash_in_strings.cfm"; } catch (any e) { writeOutput("ERROR | types/test_hash_in_strings.cfm | " & e.message & chr(10)); }
try { include "comments/test_hash_in_comments.cfm"; } catch (any e) { writeOutput("ERROR | comments/test_hash_in_comments.cfm | " & e.message & chr(10)); }
try { include "comments/test_tags_in_block_comments.cfm"; } catch (any e) { writeOutput("ERROR | comments/test_tags_in_block_comments.cfm | " & e.message & chr(10)); }

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
try { include "stdlib/test_list_rest_literal_remainder.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_list_rest_literal_remainder.cfm | " & e.message & chr(10)); }
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
try { include "stdlib/test_len_scalar_coercion.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_len_scalar_coercion.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_create_unique_id.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_create_unique_id.cfm | " & e.message & chr(10)); }
try { include "stdlib/test_preserve_single_quotes.cfm"; } catch (any e) { writeOutput("ERROR | stdlib/test_preserve_single_quotes.cfm | " & e.message & chr(10)); }
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
try { include "members/test_string_member_regex.cfm"; } catch (any e) { writeOutput("ERROR | members/test_string_member_regex.cfm | " & e.message & chr(10)); }
try { include "members/test_array_members.cfm"; } catch (any e) { writeOutput("ERROR | members/test_array_members.cfm | " & e.message & chr(10)); }
try { include "members/test_struct_members.cfm"; } catch (any e) { writeOutput("ERROR | members/test_struct_members.cfm | " & e.message & chr(10)); }
try { include "members/test_number_members.cfm"; } catch (any e) { writeOutput("ERROR | members/test_number_members.cfm | " & e.message & chr(10)); }

// --- OOP ---
try { include "oop/test_components.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_components.cfm | " & e.message & chr(10)); }
try { include "oop/test_component_method_builtin_name.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_component_method_builtin_name.cfm | " & e.message & chr(10)); }
try { include "oop/test_component_return_type.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_component_return_type.cfm | " & e.message & chr(10)); }
try { include "oop/test_inheritance.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_inheritance.cfm | " & e.message & chr(10)); }
try { include "oop/test_inherited_helpers.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_inherited_helpers.cfm | " & e.message & chr(10)); }
try { include "oop/test_interfaces.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_interfaces.cfm | " & e.message & chr(10)); }
try { include "oop/test_metadata.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_metadata.cfm | " & e.message & chr(10)); }
try { include "oop/test_dotted_function_names.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_dotted_function_names.cfm | " & e.message & chr(10)); }
try { include "oop/test_soft_keyword_function_name.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_soft_keyword_function_name.cfm | " & e.message & chr(10)); }
try { include "oop/test_property_attributes.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_property_attributes.cfm | " & e.message & chr(10)); }
try { include "oop/test_struct_method_dispatch.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_struct_method_dispatch.cfm | " & e.message & chr(10)); }
try { include "oop/test_external_prop.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_external_prop.cfm | " & e.message & chr(10)); }
try { include "oop/test_repeated_instantiation.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_repeated_instantiation.cfm | " & e.message & chr(10)); }
try { include "oop/test_component_mapping_paths.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_component_mapping_paths.cfm | " & e.message & chr(10)); }
try { include "oop/test_component_method_named_args.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_component_method_named_args.cfm | " & e.message & chr(10)); }
try { include "oop/test_component_method_precedence.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_component_method_precedence.cfm | " & e.message & chr(10)); }
try { include "oop/test_method_ref_binding.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_method_ref_binding.cfm | " & e.message & chr(10)); }
try { include "oop/test_returned_service_chain.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_returned_service_chain.cfm | " & e.message & chr(10)); }
try { include "oop/test_mixin_writeback.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_mixin_writeback.cfm | " & e.message & chr(10)); }
try { include "oop/test_property_method_name_collision.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_property_method_name_collision.cfm | " & e.message & chr(10)); }
try { include "oop/test_new_named_args.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_new_named_args.cfm | " & e.message & chr(10)); }
try { include "oop/test_dynamic_lhs_assign.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_dynamic_lhs_assign.cfm | " & e.message & chr(10)); }
try { include "oop/test_getmetadata_properties.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_getmetadata_properties.cfm | " & e.message & chr(10)); }
try { include "oop/test_component_bool_attr.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_component_bool_attr.cfm | " & e.message & chr(10)); }
try { include "oop/test_chained_writeback_clobber.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_chained_writeback_clobber.cfm | " & e.message & chr(10)); }

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
try { include "tags/test_custom_tag_attribute_collection.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_custom_tag_attribute_collection.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_customtag_lifecycle.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_customtag_lifecycle.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_buffer_recovery.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_buffer_recovery.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfexecute.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfexecute.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfmail.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfmail.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfcache.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfcache.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfstoredproc.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfstoredproc.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfqueryparam_attribute_collection.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfqueryparam_attribute_collection.cfm | " & e.message & chr(10)); }
try { include "tags/test_cfqueryparam_interpolated_value.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_cfqueryparam_interpolated_value.cfm | " & e.message & chr(10)); }
try { include "tags/test_cfquery_quoted_identifier.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_cfquery_quoted_identifier.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfquery_control_tags.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfquery_control_tags.cfm | " & e.message & chr(10)); }
try { include "tags/test_cfdirectory_mapping_path.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_cfdirectory_mapping_path.cfm | " & e.message & chr(10)); }
try { include "tags/test_cfdirectory_function_form.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_cfdirectory_function_form.cfm | " & e.message & chr(10)); }
try { include "tags/test_cfhttpparam_runtime_body.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_cfhttpparam_runtime_body.cfm | " & e.message & chr(10)); }
try { include "tags/test_cfmail_runtime_body.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_cfmail_runtime_body.cfm | " & e.message & chr(10)); }
try { include "tags/test_cfstoredproc_runtime_body.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_cfstoredproc_runtime_body.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfimport.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfimport.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfthread.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfthread.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfthread_concurrency.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfthread_concurrency.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfscript_statements.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfscript_statements.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfhttp_interpolation.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfhttp_interpolation.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfhttp_multipart.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfhttp_multipart.cfm | " & e.message & chr(10)); }
try { include "tags/test_cfhttp_timeout_interpolation.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_cfhttp_timeout_interpolation.cfm | " & e.message & chr(10)); }
try { include "tags/test_cfexecute_interpolation.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_cfexecute_interpolation.cfm | " & e.message & chr(10)); }
try { include "tags/test_tag_attribute_interpolation_sweep.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tag_attribute_interpolation_sweep.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfhttpparam_interpolation.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfhttpparam_interpolation.cfm | " & e.message & chr(10)); }
try { include "tags/test_tag_string_interpolation.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tag_string_interpolation.cfm | " & e.message & chr(10)); }
try { include "tags/test_tag_attribute_interpolation.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tag_attribute_interpolation.cfm | " & e.message & chr(10)); }
try { include "tags/test_cffinally_tag_body.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_cffinally_tag_body.cfm | " & e.message & chr(10)); }
try { include "tags/test_cflog_cfmail_attribute_interpolation.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_cflog_cfmail_attribute_interpolation.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_cfzip.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_cfzip.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_tld.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_tld.cfm | " & e.message & chr(10)); }
try { include "tags/test_tags_whitespace.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_tags_whitespace.cfm | " & e.message & chr(10)); }

// --- Includes ---
try { include "includes/test_variables_scope_includes.cfm"; } catch (any e) { writeOutput("ERROR | includes/test_variables_scope_includes.cfm | " & e.message & chr(10)); }
try { include "includes/test_named_args_includes.cfm"; } catch (any e) { writeOutput("ERROR | includes/test_named_args_includes.cfm | " & e.message & chr(10)); }
try { include "includes/test_closure_in_swapped_program.cfm"; } catch (any e) { writeOutput("ERROR | includes/test_closure_in_swapped_program.cfm | " & e.message & chr(10)); }

// --- Lifecycle / server request fixtures ---
try { include "lifecycle/test_application_mapping_coverage.cfm"; } catch (any e) { writeOutput("ERROR | lifecycle/test_application_mapping_coverage.cfm | " & e.message & chr(10)); }
try { include "lifecycle/test_application_load_errors.cfm"; } catch (any e) { writeOutput("ERROR | lifecycle/test_application_load_errors.cfm | " & e.message & chr(10)); }
try { include "lifecycle/test_application_scope_custom_tag.cfm"; } catch (any e) { writeOutput("ERROR | lifecycle/test_application_scope_custom_tag.cfm | " & e.message & chr(10)); }
try { include "server/test_front_controller_fallback.cfm"; } catch (any e) { writeOutput("ERROR | server/test_front_controller_fallback.cfm | " & e.message & chr(10)); }

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
try { include "core/test_forin_keyword_member_access.cfm"; } catch (any e) { writeOutput("ERROR | core/test_forin_keyword_member_access.cfm | " & e.message & chr(10)); }

// --- v0.34.3 round: Wheels now parses, constructs, and boots its full app
//     lifecycle + DI on RustCFML. This is the remaining language gap found
//     on the way to serving a request. ---
//   - undefined_var_autovivify: assigning to a member path of an UNDECLARED
//     variable (initArgs.path = "wheels" in Application.cfc.onApplicationStart)
//     must auto-create it as a struct. RustCFML throws "Variable is undefined"
//     for everything except the `local` scope. Wrapped in try/catch so the
//     throw fails its assertions without aborting the run.
try { include "core/test_undefined_var_autovivify.cfm"; } catch (any e) { writeOutput("ERROR | core/test_undefined_var_autovivify.cfm | " & e.message & chr(10)); }
//   - multiword_operators: RustCFML rejects multi-word comparison operators
//     (IS NOT, DOES NOT CONTAIN, GREATER THAN, ...) while accepting all
//     single-word forms. A CFC using one fails to parse -> non-object.
//     wheels.Global uses IS NOT and DOES NOT CONTAIN on the boot path.
//   - mapping_include: RustCFML does not resolve this.mappings for cfinclude
//     template paths (reads the literal "/tags/..." -> ENOENT). wheels.Global's
//     pseudo-constructor does `include "/app/global/functions.cfm"`, so it
//     throws at instantiation -> non-object -> empty dispatch.
try { include "core/test_multiword_operators.cfm"; } catch (any e) { writeOutput("ERROR | core/test_multiword_operators.cfm | " & e.message & chr(10)); }
try { include "tags/test_mapping_include.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_mapping_include.cfm | " & e.message & chr(10)); }
//   - component_soft_keyword: `component` is a SOFT keyword on Lucee/ACF/BoxLang
//     (a CFC introducer only when it begins a declaration; otherwise an ordinary
//     identifier). RustCFML used to treat it as a HARD reserved keyword, so a
//     bare `component = x` (and the `component` attribute of cfinvoke) failed to
//     PARSE. Now soft; genuine declarations still parse.
try { include "core/test_component_soft_keyword.cfm"; } catch (any e) { writeOutput("ERROR | core/test_component_soft_keyword.cfm | " & e.message & chr(10)); }
//   - cfinvoke_statement: `invoke` as a CFScript statement (attributes + optional
//     invokeargument block) is now compiled to __cfinvoke(...). RustCFML previously
//     only supported the <cfinvoke> tag and the invoke(...) call forms. (RustCFML
//     also accepts the ACF-style cf-prefixed `cfinvoke` spelling, but Lucee
//     rejects it, so the cross-engine test uses the cf-less `invoke`.)
try { include "tags/test_cfinvoke_statement.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_cfinvoke_statement.cfm | " & e.message & chr(10)); }
//   - script_transaction_attrs: cfscript `transaction action="begin" { ... }`
//     (the attribute form of the transaction tag, with a body).
try { include "tags/test_script_transaction_attrs.cfm"; } catch (any e) { writeOutput("ERROR | tags/test_script_transaction_attrs.cfm | " & e.message & chr(10)); }
//   - component_declaration_attributes: follow-on to component_soft_keyword.
//     Component-header metadata attributes are order-independent and may be
//     written quoted or unquoted on Lucee/ACF/BoxLang. Two shapes the Wheels
//     boot cascade relies on used to fail to parse on RustCFML: (A) `extends`
//     placed AFTER another attribute (component output="false" extends="..."),
//     and (B) an unquoted boolean attribute value (component output=false).
//     Failing headers live in fixtures (parse errors escape try/catch); via
//     createObject they degrade to a non-object, so the assertions show the gap.
try { include "core/test_component_declaration_attributes.cfm"; } catch (any e) { writeOutput("ERROR | core/test_component_declaration_attributes.cfm | " & e.message & chr(10)); }
//   - interface_extends_attribute: an interface may declare its parent in the
//     attribute form (interface extends="Foo") and order-independently with
//     other header attributes, not just the bareword `extends Foo`. RustCFML
//     used to reject the `=` ("Expected LBrace, found Equal"). Used across
//     vendor/wheels/interfaces/.
try { include "core/test_interface_extends_attribute.cfm"; } catch (any e) { writeOutput("ERROR | core/test_interface_extends_attribute.cfm | " & e.message & chr(10)); }
//   - isinstanceof_interface_chain: isInstanceOf must recognise interfaces
//     inherited via an interface's own `extends` (IDeclDog extends
//     IDeclCreature), for both `new X()` and createObject("component", ...).
try { include "core/test_isinstanceof_interface_chain.cfm"; } catch (any e) { writeOutput("ERROR | core/test_isinstanceof_interface_chain.cfm | " & e.message & chr(10)); }
//   - dotted_param_type: a function/method parameter may carry a dotted FQN
//     type (function f( wheels.system.TestResult x )). RustCFML used to reject
//     the first `.` ("Expected RParen, found Dot"). Parse-only (Lucee enforces
//     the type at call time, so the test never calls with a mismatched value).
try { include "core/test_dotted_param_type.cfm"; } catch (any e) { writeOutput("ERROR | core/test_dotted_param_type.cfm | " & e.message & chr(10)); }
//   - typed_toplevel_function_return_type: a TOP-LEVEL (non-component) cfscript
//     function may carry a return-type annotation (`struct function f()`), like
//     a component method. RustCFML misparsed the leading type token at the top
//     level as a bare expression statement ("Variable 'struct' is undefined").
//     Surfaced booting Wheels (vendor/wheels/public/helpers.cfm:293).
try { include "core/test_typed_toplevel_function_return_type.cfm"; } catch (any e) { writeOutput("ERROR | core/test_typed_toplevel_function_return_type.cfm | " & e.message & chr(10)); }
//   - for_increment_compound: the for-loop increment clause accepts compound
//     assignment (for (i=1; i<=10; i+=2)). RustCFML used to reject it
//     ("Expected RParen, found PlusEqual"). Used in vendor/wheels/model/bulk.cfc.
try { include "core/test_for_increment_compound.cfm"; } catch (any e) { writeOutput("ERROR | core/test_for_increment_compound.cfm | " & e.message & chr(10)); }
//   - script_fn_post_paren_attr: a script function may carry metadata
//     attributes after its () with quoted OR unquoted values
//     (function f() output=true { ... }). RustCFML used to misparse the body as
//     a struct literal for the unquoted form. Used in the wheelstest BaseSpec.
try { include "core/test_script_fn_post_paren_attr.cfm"; } catch (any e) { writeOutput("ERROR | core/test_script_fn_post_paren_attr.cfm | " & e.message & chr(10)); }
//   - invoke_canonical_forms: pins the two cross-engine invoke() forms — the
//     positional BIF invoke(objOrName, method, args) and the statement form
//     invoke component=.. method=.. {invokeargument ..}. The named-arg
//     function-call form invoke(component=..)/cfinvoke(..) is intentionally NOT
//     tested: Lucee rejects it at compile time, so it is not a portable contract.
try { include "core/test_invoke_canonical_forms.cfm"; } catch (any e) { writeOutput("ERROR | core/test_invoke_canonical_forms.cfm | " & e.message & chr(10)); }
//   - invoke_argumentcollection_spread: the argument struct passed to the
//     positional invoke() BIF must special-case an `argumentCollection` key
//     and spread the INNER struct into the callee's arguments scope (the same
//     contract as fn(argumentCollection=st) / cfinvoke). RustCFML drops the
//     key, so declared params keep their defaults. Surfaced booting Wheels:
//     $doubleCheckedLock -> $invoke controller-class resolution received
//     empty args, silently turning every user controller action into a no-op.
try { include "core/test_invoke_argumentcollection_spread.cfm"; } catch (any e) { writeOutput("ERROR | core/test_invoke_argumentcollection_spread.cfm | " & e.message & chr(10)); }
//   - reserved_word_identifiers / quoted_catch_type: follow-on to PR #32 — `new`
//     as a method name, `extends`/`implements` as parameter names, and a quoted
//     string catch type (`catch ("My.Type" e)`) are all soft constructs on
//     Lucee/ACF/BoxLang now accepted by RustCFML.
try { include "core/test_reserved_word_identifiers.cfm"; } catch (any e) { writeOutput("ERROR | core/test_reserved_word_identifiers.cfm | " & e.message & chr(10)); }
try { include "core/test_quoted_catch_type.cfm"; } catch (any e) { writeOutput("ERROR | core/test_quoted_catch_type.cfm | " & e.message & chr(10)); }
//   - chained_compound_assignment: `a = b &= c` (compound assignment as the RHS
//     of an assignment); switch_braced_case: a compound-assignment statement
//     inside a braced `case`/`default` body. Both surfaced while booting Wheels.
try { include "core/test_chained_compound_assignment.cfm"; } catch (any e) { writeOutput("ERROR | core/test_chained_compound_assignment.cfm | " & e.message & chr(10)); }
try { include "core/test_switch_braced_case.cfm"; } catch (any e) { writeOutput("ERROR | core/test_switch_braced_case.cfm | " & e.message & chr(10)); }
//   - switch_fallthrough: CFML switch is C-style — stacked empty labels share
//     the next body and a case without `break` falls through. Surfaced porting
//     WireBox (Builder.cfc's `case "model": case "id":` DSL dispatch).
try { include "core/test_switch_fallthrough.cfm"; } catch (any e) { writeOutput("ERROR | core/test_switch_fallthrough.cfm | " & e.message & chr(10)); }
try { include "core/test_application_scope_persist.cfm"; } catch (any e) { writeOutput("ERROR | core/test_application_scope_persist.cfm | " & e.message & chr(10)); }
try { include "core/test_session_scope_persist.cfm"; } catch (any e) { writeOutput("ERROR | core/test_session_scope_persist.cfm | " & e.message & chr(10)); }
try { include "core/test_application_metadata.cfm"; } catch (any e) { writeOutput("ERROR | core/test_application_metadata.cfm | " & e.message & chr(10)); }
//   - named_args_no_numeric_alias: a named-argument call to a function that
//     DECLARES params must populate the arguments scope by name only. When a
//     named arg lands in a declared positional slot, RustCFML leaks a spurious
//     numeric key (e.g. "2"), inflating StructCount and poisoning for-in /
//     option-forwarding. Surfaced while booting Wheels (contentFor section
//     detection reads StructKeyList(arguments)).
try { include "core/test_named_args_no_numeric_alias.cfm"; } catch (any e) { writeOutput("ERROR | core/test_named_args_no_numeric_alias.cfm | " & e.message & chr(10)); }
//   - param_dotted_lhs: the cfscript `param` shorthand must accept a dotted /
//     scoped lvalue (`param arguments.obj.key = default`), not just a bare
//     identifier. Surfaced while booting WireBox (Injector.cfc uses
//     `param arguments.target.$wbDelegateMap = {}`).
try { include "core/test_param_dotted_lhs.cfm"; } catch (any e) { writeOutput("ERROR | core/test_param_dotted_lhs.cfm | " & e.message & chr(10)); }
try { include "core/test_param_as_identifier.cfm"; } catch (any e) { writeOutput("ERROR | core/test_param_as_identifier.cfm | " & e.message & chr(10)); }
//   - as_string_cycle_safety: stringifying a self-referential struct (now
//     possible since structs are reference types) must terminate rather than
//     overflow the native stack.
try { include "core/test_as_string_cycle_safety.cfm"; } catch (any e) { writeOutput("ERROR | core/test_as_string_cycle_safety.cfm | " & e.message & chr(10)); }
//   - lock_finally_semantics: try/finally + lock { } must run the finally on a
//     `return` (release the lock) and re-propagate exceptions thrown inside.
try { include "core/test_lock_finally_semantics.cfm"; } catch (any e) { writeOutput("ERROR | core/test_lock_finally_semantics.cfm | " & e.message & chr(10)); }
//   - hof_member_writeback: a higher-order struct member fn (some/every/...)
//     run inside a CFC method must not leak the closure's captured `this` onto
//     the receiver variable (the WireBox `binder.hasAspects()` bug).
try { include "core/test_hof_member_writeback.cfm"; } catch (any e) { writeOutput("ERROR | core/test_hof_member_writeback.cfm | " & e.message & chr(10)); }
//   - logical_short_circuit: AND/OR (and &&/||) must skip RHS evaluation
//     once the left determines the result — matches Lucee/ACF. Surfaced as
//     `Variable 'defaultValue' is undefined` while booting WireBox.
try { include "core/test_logical_short_circuit.cfm"; } catch (any e) { writeOutput("ERROR | core/test_logical_short_circuit.cfm | " & e.message & chr(10)); }
//   - getFunctionCalledName: a UDF injected under multiple aliases reports the
//     alias it was called by — the primitive WireBox delegation dispatches on.
try { include "core/test_get_function_called_name.cfm"; } catch (any e) { writeOutput("ERROR | core/test_get_function_called_name.cfm | " & e.message & chr(10)); }
//   - delegate annotation metadata: bare + arbitrary-named property annotations
//     are captured, and component-level annotations surface top-level in
//     getComponentMetadata (Lucee parity) — the surface WireBox delegation reads.
try { include "oop/test_delegate_annotation_metadata.cfm"; } catch (any e) { writeOutput("ERROR | oop/test_delegate_annotation_metadata.cfm | " & e.message & chr(10)); }

// --- Query of Queries ---
try { include "qoq/test_qoq_select.cfm"; } catch (any e) { writeOutput("ERROR | qoq/test_qoq_select.cfm | " & e.message & chr(10)); }
try { include "qoq/test_qoq_aggregates.cfm"; } catch (any e) { writeOutput("ERROR | qoq/test_qoq_aggregates.cfm | " & e.message & chr(10)); }
try { include "qoq/test_qoq_joins.cfm"; } catch (any e) { writeOutput("ERROR | qoq/test_qoq_joins.cfm | " & e.message & chr(10)); }
try { include "qoq/test_qoq_subqueries_union.cfm"; } catch (any e) { writeOutput("ERROR | qoq/test_qoq_subqueries_union.cfm | " & e.message & chr(10)); }
try { include "qoq/test_qoq_custom_functions.cfm"; } catch (any e) { writeOutput("ERROR | qoq/test_qoq_custom_functions.cfm | " & e.message & chr(10)); }
try { include "qoq/test_qoq_rustcfml_ext.cfm"; } catch (any e) { writeOutput("ERROR | qoq/test_qoq_rustcfml_ext.cfm | " & e.message & chr(10)); }

printSummary();
</cfscript>
