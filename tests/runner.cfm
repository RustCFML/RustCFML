<cfscript>
writeOutput("============================================================" & chr(10));
writeOutput("RustCFML Test Suite" & chr(10));
writeOutput("============================================================" & chr(10) & chr(10));

include "harness.cfm";
</cfscript>


<!--- --- cfconfig --- --->
<cf_runtest file="config/test_cfconfig_loading.cfm">
<cf_runtest file="config/test_cfconfig_datasource.cfm">
<cf_runtest file="config/test_cfconfig_security.cfm">
<cf_runtest file="config/test_app_datasources.cfm">

<!--- --- Core Language --- --->
<cf_runtest file="core/test_variables.cfm">
<cf_runtest file="core/test_access_identifiers.cfm">
<cf_runtest file="core/test_reserved_word_param_names.cfm">
<cf_runtest file="core/test_function_scope_capture.cfm">
<cf_runtest file="core/test_bare_call_caller_stack_leak.cfm">
<cf_runtest file="core/test_null_return_no_key.cfm">
<cf_runtest file="core/test_bare_call_shadowing_semantics.cfm">
<cf_runtest file="core/test_closure_env_leak.cfm">
<cf_runtest file="core/test_struct_stored_closure_dotcall.cfm">
<cf_runtest file="core/test_compound_assignment.cfm">
<cf_runtest file="core/test_undeclared_named_args.cfm">
<!--- - invoke_undeclared_keys: the argument struct of the positional BIF --->
<!--- invoke(obj, method, argStruct) is a named-argument collection — EVERY --->
<!--- key must reach the callee's arguments scope, declared param or not, --->
<!--- paramless targets included. RustCFML bound only declared names and --->
<!--- silently dropped the rest (direct obj.m(argumentCollection=st) and --->
<!--- in-context this[name](argumentCollection=st) already deliver all keys; --->
<!--- only the invoke() marshaling path filtered). Surfaced while booting --->
<!--- Wheels: $simpleLock()'s "$locked" re-entry guard key never arrived, so --->
<!--- $readFlash recursed to depth 256 and 500'd every request. --->
<cf_runtest file="core/test_invoke_undeclared_keys.cfm">
<cf_runtest file="core/test_struct_method_sequential.cfm">
<cf_runtest file="core/test_include_scope_capture.cfm">
<!--- savecontent's variable= must deliver the capture to SCOPE-QUALIFIED targets --->
<!--- (variable="local.cap" / "variables.cap"), not just unqualified ones. RustCFML --->
<!--- silently dropped the scoped capture — Wheels renders every view through --->
<!--- savecontent variable="local.$wheels" { include ... } (Global.cfc), so all --->
<!--- views came back empty on an otherwise-booting framework (PR #108). --->
<cf_runtest file="core/test_savecontent_scoped_target.cfm">
<cf_runtest file="core/test_operators.cfm">
<cf_runtest file="core/test_subscript_autovivify.cfm">
<!--- Scope-qualified nested auto-vivification (variables.$class.name = ...): the residual --->
<!--- auto-viv gap that blocked Wheels $initControllerClass. Fixed in the compiler by routing --->
<!--- multi-level scope-rooted nested writes through the runtime scope-path store. --->
<cf_runtest file="core/test_scoped_nested_autoviv.cfm">
<cf_runtest file="core/test_control_flow.cfm">
<cf_runtest file="core/test_cfloop_negative_step.cfm">
<cf_runtest file="core/test_cfloop_array_item_index.cfm">
<cf_runtest file="core/test_cfloop_collection_item_index.cfm">
<cf_runtest file="core/test_scoped_loop_index_and_argcoll.cfm">
<cf_runtest file="core/test_closure_loopvar_in_cfc.cfm">
<cf_runtest file="core/test_error_handling.cfm">
<cf_runtest file="core/test_catchable_undefined.cfm">
<cf_runtest file="core/test_builtin_shadowing.cfm">
<!--- - builtin_data_shadow: a plain DATA variable named like a builtin --->
<!--- (val = "29") must not make the builtin uncallable in call position — --->
<!--- Val(val) throws "Variable is not a function" at template scope, --->
<!--- function-local, and cross-stack (a caller's local.val poisons the --->
<!--- callee's Val()). Surfaced in Wheels' $convertToString (Global.cfc does --->
<!--- `local.val = arguments.value; ... return Val(val);`) — killed --->
<!--- hasChanged() and with it every UPDATE statement. --->
<cf_runtest file="core/test_builtin_data_shadow.cfm">
<cf_runtest file="core/test_functions.cfm">
<cf_runtest file="core/test_arrow_functions.cfm">
<cf_runtest file="core/test_comma_less_params.cfm">
<cf_runtest file="core/test_required_param_with_default.cfm">
<cf_runtest file="core/test_member_index_incdec.cfm">
<cf_runtest file="core/test_member_tostring.cfm">
<cf_runtest file="core/test_getclass_and_image.cfm">
<cf_runtest file="core/test_chained_assign_entryset.cfm">
<cf_runtest file="core/test_thread_scope_page.cfm">
<cf_runtest file="core/test_bif_shadow_and_arg_alias.cfm">
<cf_runtest file="core/test_arguments_writeback.cfm">
<!--- - local_shadows_arguments: `local` and `arguments` are separate scopes --->
<!--- within ONE frame — after `local.X = ...` (or `var X = ...`), an explicit --->
<!--- `arguments.X` read must still resolve to the passed value / declared --->
<!--- default, not the local value. Bare `X` reads (scope cascade: local wins) --->
<!--- are pinned as controls so a fix can't overcorrect. Surfaced booting --->
<!--- Wheels: URLFor() declares `string params = ""` and builds a route-params --->
<!--- struct in `local.params`; its `Len(arguments.params)` query-string check --->
<!--- saw the struct, so EVERY generated URL (linkTo / startFormTag / --->
<!--- redirectTo) grew a ?%7Bcontroller...%7D= junk query string. Sibling of --->
<!--- #77 (fixed v0.92.0) / #93 (open) — same scoped-name-resolution family, --->
<!--- but conflating the local and arguments views of a single frame. --->
<!--- Runtime-level (wrong values, no parse error), so registration is safe. --->
<cf_runtest file="core/test_local_shadows_arguments.cfm">
<cf_runtest file="core/test_unscoped_nested_autoviv.cfm">
<cf_runtest file="core/test_argument_reference_nested.cfm">
<cf_runtest file="core/test_language_features.cfm">
<cf_runtest file="core/test_scopes.cfm">
<!--- - this_dot_call_detaches_writes: inside a component method, a `this.`-DOT --->
<!--- qualified method call (this.noop()) detaches the frame's `this` binding --->
<!--- onto a data-complete SHALLOW COPY on RustCFML 0.108.0 -- every later --->
<!--- this-write in that frame (and in frames it calls, any call shape) lands --->
<!--- on the detached copy: visible in-frame, DISCARDED when the detaching --->
<!--- frame returns. Bare calls, bracket calls (this["noop"]()), dot-READS, --->
<!--- and dot-calls on other objects do not detach; variables-scope writes --->
<!--- survive; nested-struct mutations escape (the copy is shallow -- pinned). --->
<!--- Broke Wheels model persistence twice over (generated PK vanishing after --->
<!--- create(), stale dirty-state after update()). Runtime-level: fails 3 --->
<!--- assertions, does NOT abort the run. --->
<cf_runtest file="core/test_this_dot_call_detaches_writes.cfm">
<cf_runtest file="core/test_server_scope.cfm">
<cf_runtest file="core/test_pagecontext_request_response.cfm">
<cf_runtest file="core/test_localmode.cfm">
<cf_runtest file="core/test_error_context.cfm">
<cf_runtest file="core/test_null_coalescing.cfm">

<!--- --- Data Types --- --->
<cf_runtest file="types/test_null.cfm">
<cf_runtest file="types/test_boolean.cfm">
<cf_runtest file="types/test_numeric.cfm">
<cf_runtest file="types/test_string.cfm">
<cf_runtest file="types/test_array.cfm">
<cf_runtest file="types/test_array_append_grow.cfm">
<cf_runtest file="types/test_array_reference_semantics.cfm">
<cf_runtest file="types/test_struct.cfm">
<cf_runtest file="types/test_struct_reference_semantics.cfm">
<cf_runtest file="types/test_ordered_struct_literals.cfm">
<cf_runtest file="types/test_nested_writeback.cfm">
<cf_runtest file="types/test_query.cfm">
<cf_runtest file="types/test_query_column.cfm">
<!--- A query cell must be a SIMPLE value: IsSimpleValue()=true and SerializeJSON of a --->
<!--- struct holding it preserves the value. RustCFML 0.161.0 returned boxed cells. --->
<cf_runtest file="types/test_query_cell_simple_value.cfm">
<cf_runtest file="types/test_query_reference.cfm">
<cf_runtest file="types/test_binary.cfm">
<cf_runtest file="types/test_hash_in_strings.cfm">
<cf_runtest file="types/test_string_interpolation_nested_strings.cfm">
<cf_runtest file="comments/test_hash_in_comments.cfm">
<cf_runtest file="comments/test_tags_in_block_comments.cfm">

<!--- --- Standard Library --- --->
<cf_runtest file="stdlib/test_string_functions.cfm">
<cf_runtest file="stdlib/test_string_functions_regex.cfm">
<cf_runtest file="stdlib/test_string_functions_encoding.cfm">
<!--- EncodeForHTMLAttribute must encode attribute-dangerous chars (space, =) per OWASP/Lucee; RustCFML leaves them raw. --->
<cf_runtest file="functions/test_encodeforhtmlattribute_space_equals.cfm">
<cf_runtest file="stdlib/test_array_functions.cfm">
<cf_runtest file="stdlib/test_array_higher_order.cfm">
<cf_runtest file="stdlib/test_struct_functions.cfm">
<cf_runtest file="stdlib/test_struct_higher_order.cfm">
<cf_runtest file="stdlib/test_math_functions.cfm">
<cf_runtest file="stdlib/test_date_functions.cfm">
<cf_runtest file="stdlib/test_list_functions.cfm">
<cf_runtest file="stdlib/test_list_rest_literal_remainder.cfm">
<cf_runtest file="stdlib/test_list_higher_order.cfm">
<cf_runtest file="stdlib/test_query_functions.cfm">
<cf_runtest file="stdlib/test_query_higher_order.cfm">
<cf_runtest file="stdlib/test_type_checking.cfm">
<cf_runtest file="stdlib/test_conversion.cfm">
<cf_runtest file="stdlib/test_json.cfm">
<cf_runtest file="stdlib/test_file_io.cfm">
<!--- Relative file-BIF paths resolve against the calling template's dir --->
<!--- (ExpandPath parity), not the entry template / cwd — GitHub #171. --->
<cf_runtest file="stdlib/test_file_relative_path.cfm">
<cf_runtest file="stdlib/test_security.cfm">
<cf_runtest file="stdlib/test_password_hashing.cfm">
<cf_runtest file="stdlib/test_xml.cfm">
<cf_runtest file="stdlib/test_utility.cfm">
<cf_runtest file="stdlib/test_encoding_functions.cfm">
<cf_runtest file="stdlib/test_query_mutations.cfm">
<cf_runtest file="stdlib/test_date_functions_extra.cfm">
<cf_runtest file="stdlib/test_locale_functions.cfm">
<cf_runtest file="stdlib/test_cache_functions.cfm">
<cf_runtest file="stdlib/test_higher_order_functions.cfm">
<cf_runtest file="stdlib/test_bitmask_functions.cfm">
<cf_runtest file="stdlib/test_xml_dom_functions.cfm">
<cf_runtest file="stdlib/test_misc_functions.cfm">
<cf_runtest file="stdlib/test_len_scalar_coercion.cfm">
<cf_runtest file="stdlib/test_create_unique_id.cfm">
<cf_runtest file="stdlib/test_preserve_single_quotes.cfm">
<cf_runtest file="stdlib/test_valuelist_functions.cfm">
<cf_runtest file="stdlib/test_callstack.cfm">
<cf_runtest file="stdlib/test_precisionevaluate.cfm">
<cf_runtest file="stdlib/test_evaluate.cfm">
<cf_runtest file="stdlib/test_htmlparse.cfm">
<cf_runtest file="stdlib/test_gettagdata.cfm">
<cf_runtest file="stdlib/test_ini_functions.cfm">
<cf_runtest file="stdlib/test_directorylist.cfm">
<cf_runtest file="stdlib/test_writedump.cfm">
<cf_runtest file="stdlib/test_cfdirectory_type_filter.cfm">
<cf_runtest file="stdlib/test_cfhttp.cfm">

<!--- --- Function References --- --->
<cf_runtest file="functions/test_function_references.cfm">

<!--- --- Member Functions --- --->
<cf_runtest file="members/test_string_members.cfm">
<cf_runtest file="members/test_string_member_regex.cfm">
<cf_runtest file="members/test_array_members.cfm">
<cf_runtest file="members/test_struct_members.cfm">
<cf_runtest file="members/test_number_members.cfm">

<!--- --- OOP --- --->
<cf_runtest file="oop/test_components.cfm">
<!--- - component_internals_serialize_leak: iterating a component (for(k in obj)) --->
<!--- or SerializeJSON(obj) must expose only data members, never engine --->
<!--- internals. RustCFML leaks __name/__source_file/__variables into both, and --->
<!--- SerializeJSON emits methods as null keys. Breaks Wheels model --->
<!--- properties() (for-in over `this`) and renderWith(data=modelObject) — a --->
<!--- single-record JSON response comes back as ~379 internal keys. Runtime-safe. --->
<cf_runtest file="oop/test_overflow_arg_no_leak.cfm">
<cf_runtest file="oop/test_component_internals_serialize_leak.cfm">
<cf_runtest file="oop/test_component_method_builtin_name.cfm">
<cf_runtest file="oop/test_component_return_type.cfm">
<cf_runtest file="oop/test_inheritance.cfm">
<cf_runtest file="oop/test_super_case_insensitive_this.cfm">
<cf_runtest file="oop/test_inherited_helpers.cfm">
<cf_runtest file="oop/test_interfaces.cfm">
<cf_runtest file="oop/test_metadata.cfm">
<cf_runtest file="oop/test_mock_mixin_injection.cfm">
<cf_runtest file="oop/test_dotted_function_names.cfm">
<cf_runtest file="oop/test_static.cfm">
<cf_runtest file="oop/test_soft_keyword_function_name.cfm">
<cf_runtest file="oop/test_preside_serve_fixes.cfm">
<cf_runtest file="oop/test_property_attributes.cfm">
<cf_runtest file="oop/test_struct_method_dispatch.cfm">
<cf_runtest file="oop/test_external_prop.cfm">
<cf_runtest file="oop/test_repeated_instantiation.cfm">
<cf_runtest file="oop/test_component_mapping_paths.cfm">
<cf_runtest file="oop/test_component_method_named_args.cfm">
<cf_runtest file="oop/test_component_method_precedence.cfm">
<cf_runtest file="oop/test_method_ref_binding.cfm">
<cf_runtest file="oop/test_returned_service_chain.cfm">
<cf_runtest file="oop/test_mixin_writeback.cfm">
<cf_runtest file="oop/test_property_method_name_collision.cfm">
<cf_runtest file="oop/test_new_named_args.cfm">
<cf_runtest file="oop/test_dynamic_lhs_assign.cfm">
<cf_runtest file="oop/test_getmetadata_properties.cfm">
<cf_runtest file="oop/test_component_bool_attr.cfm">
<cf_runtest file="oop/test_chained_writeback_clobber.cfm">
<cf_runtest file="oop/test_unscoped_compound_variables_write.cfm">
<cf_runtest file="oop/test_method_return_name_collision.cfm">
<!--- Bare component name resolves relative to the CALLING CFC's package. From --->
<!--- inside oop.relcomp.Maker, createObject("component","Sibling") must find --->
<!--- oop.relcomp.Sibling. Was the deepest blocker for the Wheels migrator --->
<!--- TableDefinition DSL (createTable/t.string/t.integer/changeTable all route --->
<!--- through relative createObject of sibling migrator components). The --->
<!--- `new Sibling()` spelling is uncatchable on a miss, so only the runner-safe --->
<!--- createObject form is asserted. Credit bpamiri (PR #132). --->
<cf_runtest file="oop/test_relative_component_resolution.cfm">
<!--- Inherited-method sibling of #132: a bare CreateObject inside an inherited method must --->
<!--- resolve against the PARENT (defining) component's package, not the concrete subclass's dir. --->
<cf_runtest file="oop/test_inherited_bare_component_resolution.cfm">
<!--- cfinvoke method="<name>" must dispatch an unknown method to onMissingMethod (as a --->
<!--- direct dot-call does). RustCFML 0.161.0 threw "Method not found"; breaks Wheels --->
<!--- hasMany dependent=delete/deleteAll/removeAll cascade (deleteAll<assoc> via cfinvoke). --->
<cf_runtest file="oop/test_cfinvoke_onmissingmethod.cfm">
<!--- A named exclusive lock must be reentrant within the same request/thread. --->
<!--- RustCFML 0.161.0 self-deadlocked on re-entry (inner timed out + threw). --->
<cf_runtest file="core/test_named_lock_reentrant.cfm">
<!--- `new Comp(args)` must propagate an exception thrown by init() (constructor-guard --->
<!--- validation). RustCFML 0.161.0 swallowed it under the `new` sugar and returned a --->
<!--- half-built object; createObject(...).init() propagates correctly. --->
<cf_runtest file="oop/test_new_constructor_init_throw.cfm">
<!--- - inherited_bare_component_via_child_method: follow-on to #133. A bare --->
<!--- CreateObject("component","X") in an inherited method must resolve to the --->
<!--- DEFINING component's package even when reached via a CHILD-defined method --->
<!--- on a subclass in a different dir. #133 fixed the direct-call case (the --->
<!--- CONTROL here passes); RustCFML still resolves against the outermost child --->
<!--- frame's dir for the indirect case. This is the exact Wheels migrator shape --->
<!--- (migration up() -> inherited createTable() -> CreateObject("TableDefinition")), --->
<!--- the sole remaining migrator blocker. Runtime-level, runner-safe. --->
<cf_runtest file="oop/test_inherited_bare_component_via_child_method.cfm">

<!--- --- Tags --- --->
<cf_runtest file="tags/test_cfdump_tag.cfm">
<cf_runtest file="tags/test_tags_basic.cfm">
<cf_runtest file="tags/test_tags_control.cfm">
<cf_runtest file="tags/test_tags_include.cfm">
<cf_runtest file="tags/test_cfinclude_css.cfm">
<cf_runtest file="tags/test_tags_cffunction_hoisting.cfm">
<cf_runtest file="tags/test_tags_savecontent.cfm">
<cf_runtest file="tags/test_tags_param.cfm">
<cf_runtest file="tags/test_tags_param_dynamic.cfm">
<cf_runtest file="tags/test_tags_cfoutput_query.cfm">
<cf_runtest file="tags/test_tags_misc.cfm">
<cf_runtest file="tags/test_tags_cfsleep.cfm">
<cf_runtest file="tags/test_tags_cfhtmlhead_body.cfm">
<cf_runtest file="tags/test_tags_cfexit.cfm">
<cf_runtest file="tags/test_tags_customtag.cfm">
<cf_runtest file="tags/test_custom_tag_attribute_collection.cfm">
<cf_runtest file="tags/test_tags_customtag_lifecycle.cfm">
<cf_runtest file="tags/test_tags_buffer_recovery.cfm">
<cf_runtest file="tags/test_tags_cfexecute.cfm">
<cf_runtest file="tags/test_tags_cfmail.cfm">
<cf_runtest file="tags/test_tags_cfcache.cfm">
<cf_runtest file="tags/test_tags_cfstoredproc.cfm">
<cf_runtest file="tags/test_tags_cfqueryparam_attribute_collection.cfm">
<cf_runtest file="tags/test_cfhttp_multiparam_url.cfm">
<cf_runtest file="tags/test_cfqueryparam_interpolated_value.cfm">
<cf_runtest file="tags/test_cfqueryparam_in_transaction.cfm">
<!--- - cfqueryparam_script_form: cfqueryparam must be callable as a script --->
<!--- statement (positional AND attributeCollection) inside a script --->
<!--- cfquery(){} block, like the <cfqueryparam> tag. RustCFML throws --->
<!--- "Variable 'cfqueryparam' is undefined". vendor/wheels/databaseAdapters/ --->
<!--- Base.cfc emits exactly this shape for every bound value, so INSERT, --->
<!--- UPDATE, soft-delete, and parameterized WHERE all throw on RustCFML — --->
<!--- the ORM persistence layer is blocked. Runtime gap (undefined-identifier --->
<!--- throw, catchable since v0.125), runner-safe. --->
<cf_runtest file="tags/test_cfqueryparam_script_form.cfm">
<cf_runtest file="tags/test_pg_temporal_param_binds.cfm">
<cf_runtest file="tags/test_pg_jsonb_param_binds.cfm">
<cf_runtest file="tags/test_pg_vector_param_binds.cfm">
<cf_runtest file="tags/test_pg_error_cause_chain.cfm">
<!--- PostgreSQL DML with RETURNING returns rows and must use the query path, --->
<!--- not the execute path. Lucee supports atomic UPDATE ... RETURNING patterns. --->
<cf_runtest file="tags/test_pg_dml_returning.cfm">
<!--- SQL Server OUTPUT and MariaDB RETURNING are the same class of bug: DML --->
<!--- that returns rows must use the query path, else the rows are silently lost. --->
<cf_runtest file="tags/test_mssql_dml_output.cfm">
<cf_runtest file="tags/test_mysql_dml_returning.cfm">
<cf_runtest file="tags/test_pg_extended_param_binds.cfm">
<cf_runtest file="tags/test_pg_pool_checkout_validation.cfm">
<cf_runtest file="tags/test_pg_pool_stale_connection_retry.cfm">
<cf_runtest file="tags/test_mssql_pool_stale_connection_retry.cfm">
<cf_runtest file="tags/test_cfquery_quoted_identifier.cfm">
<cf_runtest file="tags/test_cfquery_sql_line_comments.cfm">
<cf_runtest file="tags/test_cte_with_query.cfm">
<cf_runtest file="tags/test_tags_cfquery_control_tags.cfm">
<cf_runtest file="tags/test_cfquery_result_delivery.cfm">
<cf_runtest file="tags/test_cfdbinfo.cfm">
<cf_runtest file="tags/test_cfdirectory_mapping_path.cfm">
<cf_runtest file="tags/test_cfdirectory_function_form.cfm">
<cf_runtest file="tags/test_cfdirectory_recurse_symlink.cfm">
<cf_runtest file="tags/test_cfdirectory_listinfo_name_relative.cfm">
<cf_runtest file="tags/test_cfdirectory_attrcoll_name.cfm">
<cf_runtest file="tags/test_cffile_script_form.cfm">
<cf_runtest file="tags/test_cfhttpparam_runtime_body.cfm">
<cf_runtest file="tags/test_cfmail_runtime_body.cfm">
<cf_runtest file="tags/test_cfmailpart_script_form.cfm">
<!--- cfhtmlhead exists as a tag (v0.186) but must ALSO be script-callable (cfhtmlhead(text=)); RustCFML threw "undefined". --->
<cf_runtest file="tags/test_cfhtmlhead_script_callable.cfm">
<cf_runtest file="tags/test_cfstoredproc_runtime_body.cfm">
<cf_runtest file="tags/test_tags_cfimport.cfm">
<cf_runtest file="tags/test_tags_cfthread.cfm">
<cf_runtest file="tags/test_tags_cfthread_concurrency.cfm">
<cf_runtest file="tags/test_tags_cfscript_statements.cfm">
<cf_runtest file="tags/test_tags_cfhttp_interpolation.cfm">
<cf_runtest file="tags/test_cfhttp_attribute_collection.cfm">
<cf_runtest file="tags/test_throw_object_rootcause.cfm">
<cf_runtest file="tags/test_cfloop_file_and_includes.cfm">
<cf_runtest file="tags/test_tags_cfhttp_multipart.cfm">
<cf_runtest file="tags/test_cfhttp_timeout_interpolation.cfm">
<cf_runtest file="tags/test_cfexecute_interpolation.cfm">
<cf_runtest file="tags/test_tag_attribute_interpolation_sweep.cfm">
<cf_runtest file="tags/test_tags_cfhttpparam_interpolation.cfm">
<cf_runtest file="tags/test_tag_string_interpolation.cfm">
<cf_runtest file="tags/test_tag_attribute_interpolation.cfm">
<cf_runtest file="tags/test_cffinally_tag_body.cfm">
<cf_runtest file="tags/test_cflog_cfmail_attribute_interpolation.cfm">
<cf_runtest file="tags/test_tags_cfzip.cfm">
<cf_runtest file="tags/test_tags_tld.cfm">
<cf_runtest file="tags/test_tags_whitespace.cfm">

<!--- --- Includes --- --->
<cf_runtest file="includes/test_variables_scope_includes.cfm">
<cf_runtest file="includes/test_named_args_includes.cfm">
<cf_runtest file="includes/test_closure_in_swapped_program.cfm">

<!--- --- Lifecycle / server request fixtures --- --->
<cf_runtest file="lifecycle/test_session_app_namespace.cfm">
<cf_runtest file="lifecycle/test_application_mapping_coverage.cfm">
<cf_runtest file="lifecycle/test_application_lifecycle_case_override.cfm">
<cf_runtest file="lifecycle/test_application_load_errors.cfm">
<cf_runtest file="lifecycle/test_application_scope_custom_tag.cfm">
<cf_runtest file="lifecycle/test_application_onerror_onabort.cfm">
<cf_runtest file="server/test_front_controller_fallback.cfm">
<cf_runtest file="server/test_location_redirect.cfm">

<!--- --- Java Shims --- --->
<cf_runtest file="java_shims/test_all.cfm">
<cf_runtest file="java_shims/test_comprehensive.cfm">
<cf_runtest file="java_shims/test_more.cfm">
<cf_runtest file="java_shims/test_security.cfm">
<cf_runtest file="java_shims/test_stringbuilder.cfm">
<cf_runtest file="java_shims/test_system.cfm">
<cf_runtest file="java_shims/test_concurrent_map.cfm">

<!--- --- Engine Compatibility --- --->
<cf_runtest file="compat_engine/test_math_functions.cfm">
<cf_runtest file="compat_engine/test_string_functions.cfm">
<cf_runtest file="compat_engine/test_struct_functions.cfm">
<cf_runtest file="compat_engine/test_array_functions.cfm">
<cf_runtest file="compat_engine/test_list_functions.cfm">
<cf_runtest file="compat_engine/test_query_functions.cfm">
<cf_runtest file="compat_engine/test_date_functions.cfm">
<cf_runtest file="compat_engine/test_type_checking.cfm">
<cf_runtest file="compat_engine/test_json.cfm">
<cf_runtest file="compat_engine/test_type_casting.cfm">
<cf_runtest file="compat_engine/test_language_operators.cfm">
<cf_runtest file="compat_engine/test_language_controlflow.cfm">
<cf_runtest file="compat_engine/test_language_closures.cfm">
<cf_runtest file="compat_engine/test_file_functions.cfm">
<cf_runtest file="compat_engine/test_encoding_functions.cfm">
<cf_runtest file="compat_engine/test_collection_functions.cfm">
<cf_runtest file="compat_engine/test_edge_cases.cfm">
<cf_runtest file="compat_engine/test_scope_behavior.cfm">
<cf_runtest file="native/test_native_fn.cfm">
<cf_runtest file="native/test_native_class.cfm">
<cf_runtest file="native/test_native_thread.cfm">
<cf_runtest file="native/test_cfc_extends_rust.cfm">
<!--- S3 tests live under tests/s3/ but are excluded from the default runner — --->
<!--- they need live credentials (AWS / R2 / MinIO) to pass. Run the full S3 --->
<!--- harness via /tmp/rustcfml-s3-harness/run_live.sh (see docs/s3.md), or --->
<!--- invoke a single file directly: --->
<!--- cargo run -- tests/s3/test_s3_functions.cfm --->

<!--- --- Cross-engine compatibility (Wheels framework gaps) --- --->
<!--- These tests exercise CFML behaviors Wheels depends on that pass on Lucee --->
<!--- 7 but are (or were) gaps in RustCFML. Registered last as a cluster; --->
<!--- none of them aborts the run on RustCFML, so ordering here is not --->
<!--- load-bearing -- each one fails its own assertions in isolation and the --->
<!--- run still reaches printSummary(). --->

<!--- - local_at_template_scope, metadata_name_value, script_syntax_body: --->
<!--- parse/behavioral gaps that 0.20.x has since closed; kept as --->
<!--- regression tests -- they pass on both engines now. --->
<!--- - expandpath_trailing_slash: behavioral gap, still open on 0.20.2 -- --->
<!--- for an EXISTING path, expandPath canonicalizes and drops the trailing --->
<!--- slash, so the Wheels "appDir & '../plugins'" shape fuses into a --->
<!--- malformed path. Fails its assertions but does NOT abort the run. --->
<!--- - forin_member_loop_var: two distinct for-in gaps on 0.20.2, both --->
<!--- non-fatal here. (1) A plain member-path loop var (ctx.item) PARSES --->
<!--- but never iterates -- the body is silently skipped. (2) A `this`- --->
<!--- headed loop var fails to PARSE, but that parse error is CONTAINED --->
<!--- inside a runtime-instantiated fixture CFC (ForInThisLoopFixture), --->
<!--- which degrades to a non-object silently instead of aborting. Both --->
<!--- modes fail their assertions without taking down the run. --->
<cf_runtest file="core/test_local_at_template_scope.cfm">
<!--- - local_scope_absence_leak: a callee that never declares `local.rv` must --->
<!--- get a fresh, EMPTY local — StructKeyExists(local, "rv") false and --->
<!--- isNull(local.rv) true even when the caller holds a same-named local.rv. --->
<!--- READ-side residual of the v0.92.0 per-frame fix (PR #77, --->
<!--- test_local_scope_frame_isolation.cfm): declarations are isolated, but --->
<!--- absence-checks/reads still see the caller's slot. Surfaced booting --->
<!--- Wheels: the $callback() default-true tail --->
<!--- `if (!StructKeyExists(local, "rv")) { local.rv = true; }` inherited the --->
<!--- caller's false, so every model callback chain failed and save() aborted --->
<!--- before its INSERT, silently. Runtime-level (wrong values, no parse --->
<!--- error), so registration is safe. --->
<cf_runtest file="core/test_local_scope_absence_leak.cfm">
<cf_runtest file="core/test_local_arguments_scope_independence.cfm">
<cf_runtest file="core/test_local_key_read.cfm">
<cf_runtest file="oop/test_metadata_name_value.cfm">
<!--- A parent's displayName attribute must NOT be copied onto a child's leaf metadata. --->
<!--- RustCFML 0.161.0 propagates it; Lucee/ACF/BoxLang leave it absent on the leaf. --->
<cf_runtest file="oop/test_getmetadata_inherited_displayname.cfm">
<cf_runtest file="tags/test_tags_script_syntax_body.cfm">
<cf_runtest file="functions/test_expandpath_trailing_slash.cfm">
<cf_runtest file="core/test_forin_member_loop_var.cfm">
<cf_runtest file="core/test_forin_keyword_member_access.cfm">
<!--- - forin_string_list: for-in over a comma-delimited STRING must iterate --->
<!--- the LIST ITEMS ("id","title","body"), not the CHARACTERS. RustCFML --->
<!--- iterates character-by-character (commas included), so Wheels' --->
<!--- $queryRowToStruct -- for (local.column in query.columnList) -- gave --->
<!--- every finder-hydrated model object junk single-character property --->
<!--- keys instead of its real columns. Runtime gap: wrong values, no --->
<!--- parse error. --->
<cf_runtest file="core/test_forin_string_list.cfm">

<!--- --- v0.34.3 round: Wheels now parses, constructs, and boots its full app --->
<!--- lifecycle + DI on RustCFML. This is the remaining language gap found --->
<!--- on the way to serving a request. --- --->
<!--- - undefined_var_autovivify: assigning to a member path of an UNDECLARED --->
<!--- variable (initArgs.path = "wheels" in Application.cfc.onApplicationStart) --->
<!--- must auto-create it as a struct. RustCFML throws "Variable is undefined" --->
<!--- for everything except the `local` scope. Wrapped in try/catch so the --->
<!--- throw fails its assertions without aborting the run. --->
<cf_runtest file="core/test_undefined_var_autovivify.cfm">
<!--- - multiword_operators: RustCFML rejects multi-word comparison operators --->
<!--- (IS NOT, DOES NOT CONTAIN, GREATER THAN, ...) while accepting all --->
<!--- single-word forms. A CFC using one fails to parse -> non-object. --->
<!--- wheels.Global uses IS NOT and DOES NOT CONTAIN on the boot path. --->
<!--- - mapping_include: RustCFML does not resolve this.mappings for cfinclude --->
<!--- template paths (reads the literal "/tags/..." -> ENOENT). wheels.Global's --->
<!--- pseudo-constructor does `include "/app/global/functions.cfm"`, so it --->
<!--- throws at instantiation -> non-object -> empty dispatch. --->
<cf_runtest file="core/test_multiword_operators.cfm">
<cf_runtest file="tags/test_mapping_include.cfm">
<!--- - expandpath_leading_double_slash: a leading "//" must normalize to a --->
<!--- single "/" before this.mappings resolution — expandPath("//x") == --->
<!--- expandPath("/x"), resolving the same mapping. RustCFML resolved --->
<!--- expandPath("/wheelsmapprobe") via the mapping but let "//wheelsmapprobe" --->
<!--- fall through to a docroot-relative path, missing the mapping. The stock --->
<!--- `wheels new` Application.cfc declares a /plugins mapping and boot joins --->
<!--- webPath("/") & "/plugins" = "//plugins", so Plugins.cfc's --->
<!--- cfdirectory(ExpandPath("//plugins")) hits a nonexistent dir, throws, and --->
<!--- $init aborts — every request 500s on a pristine Wheels app. --->
<cf_runtest file="tags/test_expandpath_leading_double_slash.cfm">
<!--- - component_soft_keyword: `component` is a SOFT keyword on Lucee/ACF/BoxLang --->
<!--- (a CFC introducer only when it begins a declaration; otherwise an ordinary --->
<!--- identifier). RustCFML used to treat it as a HARD reserved keyword, so a --->
<!--- bare `component = x` (and the `component` attribute of cfinvoke) failed to --->
<!--- PARSE. Now soft; genuine declarations still parse. --->
<cf_runtest file="core/test_component_soft_keyword.cfm">
<!--- - cfinvoke_statement: `invoke` as a CFScript statement (attributes + optional --->
<!--- invokeargument block) is now compiled to __cfinvoke(...). RustCFML previously --->
<!--- only supported the <cfinvoke> tag and the invoke(...) call forms. (RustCFML --->
<!--- also accepts the ACF-style cf-prefixed `cfinvoke` spelling, but Lucee --->
<!--- rejects it, so the cross-engine test uses the cf-less `invoke`.) --->
<cf_runtest file="tags/test_cfinvoke_statement.cfm">
<!--- - cfinvoke_call_form_marshaling: the cf-PREFIXED parenthesized CALL form --->
<!--- cfinvoke(...) — the spelling Lucee accepts and Wheels' Global.cfc --->
<!--- $invoke() uses — must marshal attributeCollection, deliver --->
<!--- returnVariable scope-aware (plain/dotted, page/function level), and --->
<!--- dispatch the componentless form as a SIBLING method on the current --->
<!--- component. cfquery(attributeCollection) honors the same spelling --->
<!--- (in-suite control). --->
<cf_runtest file="tags/test_cfinvoke_tag_marshaling.cfm">
<!--- - script_transaction_attrs: cfscript `transaction action="begin" { ... }` --->
<!--- (the attribute form of the transaction tag, with a body). --->
<cf_runtest file="tags/test_script_transaction_attrs.cfm">
<!--- - transaction_action_statement: the body-less cfscript transaction --->
<!--- STATEMENT form `transaction action="commit";` / `="rollback";` / --->
<!--- `="begin";` (no `{ ... }` block) — the spelling every Wheels migration --->
<!--- template emits inside up()/down(). Distinct from script_transaction_attrs --->
<!--- above (which has a body). A bare transaction{} block is the in-suite --->
<!--- control. --->
<cf_runtest file="tags/test_transaction_action_statement.cfm">
<!--- - nested_transaction: a transaction{} block nested inside another --->
<!--- transaction{} block. Lucee/ACF/BoxLang run the inner as a savepoint and --->
<!--- complete; RustCFML throws "nested transactions are not supported". Wheels --->
<!--- model save()/create() inside an outer app transaction hits this (84 specs --->
<!--- in the core suite). --->
<cf_runtest file="tags/test_nested_transaction.cfm">
<!--- - component_declaration_attributes: follow-on to component_soft_keyword. --->
<!--- Component-header metadata attributes are order-independent and may be --->
<!--- written quoted or unquoted on Lucee/ACF/BoxLang. Two shapes the Wheels --->
<!--- boot cascade relies on used to fail to parse on RustCFML: (A) `extends` --->
<!--- placed AFTER another attribute (component output="false" extends="..."), --->
<!--- and (B) an unquoted boolean attribute value (component output=false). --->
<!--- Failing headers live in fixtures (parse errors escape try/catch); via --->
<!--- createObject they degrade to a non-object, so the assertions show the gap. --->
<cf_runtest file="core/test_component_declaration_attributes.cfm">
<!--- - interface_extends_attribute: an interface may declare its parent in the --->
<!--- attribute form (interface extends="Foo") and order-independently with --->
<!--- other header attributes, not just the bareword `extends Foo`. RustCFML --->
<!--- used to reject the `=` ("Expected LBrace, found Equal"). Used across --->
<!--- vendor/wheels/interfaces/. --->
<cf_runtest file="core/test_interface_extends_attribute.cfm">
<!--- - isinstanceof_interface_chain: isInstanceOf must recognise interfaces --->
<!--- inherited via an interface's own `extends` (IDeclDog extends --->
<!--- IDeclCreature), for both `new X()` and createObject("component", ...). --->
<cf_runtest file="core/test_isinstanceof_interface_chain.cfm">
<!--- - dotted_param_type: a function/method parameter may carry a dotted FQN --->
<!--- type (function f( wheels.system.TestResult x )). RustCFML used to reject --->
<!--- the first `.` ("Expected RParen, found Dot"). Parse-only (Lucee enforces --->
<!--- the type at call time, so the test never calls with a mismatched value). --->
<cf_runtest file="core/test_dotted_param_type.cfm">
<!--- - typed_toplevel_function_return_type: a TOP-LEVEL (non-component) cfscript --->
<!--- function may carry a return-type annotation (`struct function f()`), like --->
<!--- a component method. RustCFML misparsed the leading type token at the top --->
<!--- level as a bare expression statement ("Variable 'struct' is undefined"). --->
<!--- Surfaced booting Wheels (vendor/wheels/public/helpers.cfm:293). --->
<cf_runtest file="core/test_typed_toplevel_function_return_type.cfm">
<!--- - for_increment_compound: the for-loop increment clause accepts compound --->
<!--- assignment (for (i=1; i<=10; i+=2)). RustCFML used to reject it --->
<!--- ("Expected RParen, found PlusEqual"). Used in vendor/wheels/model/bulk.cfc. --->
<cf_runtest file="core/test_for_increment_compound.cfm">
<!--- - script_fn_post_paren_attr: a script function may carry metadata --->
<!--- attributes after its () with quoted OR unquoted values --->
<!--- (function f() output=true { ... }). RustCFML used to misparse the body as --->
<!--- a struct literal for the unquoted form. Used in the wheelstest BaseSpec. --->
<cf_runtest file="core/test_script_fn_post_paren_attr.cfm">
<!--- - invoke_canonical_forms: pins the two cross-engine invoke() forms — the --->
<!--- positional BIF invoke(objOrName, method, args) and the statement form --->
<!--- invoke component=.. method=.. {invokeargument ..}. The named-arg --->
<!--- function-call form invoke(component=..)/cfinvoke(..) is intentionally NOT --->
<!--- tested: Lucee rejects it at compile time, so it is not a portable contract. --->
<cf_runtest file="core/test_invoke_canonical_forms.cfm">
<!--- - reserved_word_identifiers / quoted_catch_type: follow-on to PR #32 — `new` --->
<!--- as a method name, `extends`/`implements` as parameter names, and a quoted --->
<!--- string catch type (`catch ("My.Type" e)`) are all soft constructs on --->
<!--- Lucee/ACF/BoxLang now accepted by RustCFML. --->
<cf_runtest file="core/test_reserved_word_identifiers.cfm">
<cf_runtest file="core/test_quoted_catch_type.cfm">
<!--- Multi-catch must select exactly ONE clause by declared type (was: every --->
<!--- clause body ran unconditionally, type ignored); unmatched types propagate. --->
<cf_runtest file="core/test_multi_catch_type_dispatch.cfm">
<!--- A `return` from inside an open try block must not leak its handler onto --->
<!--- the VM try-stack (a later throw was misrouted to it and looped — the --->
<!--- TestBox CoverageServiceTest fatal recursion). --->
<cf_runtest file="core/test_return_inside_try_handler_leak.cfm">
<!--- - chained_compound_assignment: `a = b &= c` (compound assignment as the RHS --->
<!--- of an assignment); switch_braced_case: a compound-assignment statement --->
<!--- inside a braced `case`/`default` body. Both surfaced while booting Wheels. --->
<cf_runtest file="core/test_chained_compound_assignment.cfm">
<cf_runtest file="core/test_switch_braced_case.cfm">
<!--- - switch_fallthrough: CFML switch is C-style — stacked empty labels share --->
<!--- the next body and a case without `break` falls through. Surfaced porting --->
<!--- WireBox (Builder.cfc's `case "model": case "id":` DSL dispatch). --->
<cf_runtest file="core/test_switch_fallthrough.cfm">
<cf_runtest file="core/test_application_scope_persist.cfm">
<cf_runtest file="core/test_session_scope_persist.cfm">
<cf_runtest file="core/test_session_commit.cfm">
<cf_runtest file="core/test_application_metadata.cfm">
<!--- - named_args_no_numeric_alias: a named-argument call to a function that --->
<!--- DECLARES params must populate the arguments scope by name only. When a --->
<!--- named arg lands in a declared positional slot, RustCFML leaks a spurious --->
<!--- numeric key (e.g. "2"), inflating StructCount and poisoning for-in / --->
<!--- option-forwarding. Surfaced while booting Wheels (contentFor section --->
<!--- detection reads StructKeyList(arguments)). --->
<cf_runtest file="core/test_named_args_no_numeric_alias.cfm">
<!--- - named_args_array_view: the arguments scope of a NAMED-argument call must --->
<!--- stay array-addressable (hybrid array/struct) exactly like a positional --->
<!--- call — ArrayLen(arguments) counts the args and arguments[1] reads the --->
<!--- first declared slot / first value. RustCFML returned ArrayLen == 0 for --->
<!--- every named call (and null for arguments[1] on paramless fns), so --->
<!--- Wheels' paramless config setter $set() — which branches on --->
<!--- ArrayLen(arguments) > 1 and reads arguments[1] — silently wrote every --->
<!--- setting as an empty value and the ORM introspected the wrong (default --->
<!--- in-memory) database. --->
<cf_runtest file="core/test_named_args_array_view.cfm">
<!--- - param_dotted_lhs: the cfscript `param` shorthand must accept a dotted / --->
<!--- scoped lvalue (`param arguments.obj.key = default`), not just a bare --->
<!--- identifier. Surfaced while booting WireBox (Injector.cfc uses --->
<!--- `param arguments.target.$wbDelegateMap = {}`). --->
<cf_runtest file="core/test_param_dotted_lhs.cfm">
<cf_runtest file="core/test_param_as_identifier.cfm">
<!--- - as_string_cycle_safety: stringifying a self-referential struct (now --->
<!--- possible since structs are reference types) must terminate rather than --->
<!--- overflow the native stack. --->
<cf_runtest file="core/test_as_string_cycle_safety.cfm">
<!--- - lock_finally_semantics: try/finally + lock { } must run the finally on a --->
<!--- `return` (release the lock) and re-propagate exceptions thrown inside. --->
<cf_runtest file="core/test_lock_finally_semantics.cfm">
<!--- - hof_member_writeback: a higher-order struct member fn (some/every/...) --->
<!--- run inside a CFC method must not leak the closure's captured `this` onto --->
<!--- the receiver variable (the WireBox `binder.hasAspects()` bug). --->
<cf_runtest file="core/test_hof_member_writeback.cfm">
<!--- - logical_short_circuit: AND/OR (and &&/||) must skip RHS evaluation --->
<!--- once the left determines the result — matches Lucee/ACF. Surfaced as --->
<!--- `Variable 'defaultValue' is undefined` while booting WireBox. --->
<cf_runtest file="core/test_logical_short_circuit.cfm">
<!--- - getFunctionCalledName: a UDF injected under multiple aliases reports the --->
<!--- alias it was called by — the primitive WireBox delegation dispatches on. --->
<cf_runtest file="core/test_get_function_called_name.cfm">
<!--- - new_udf_dispatch_and_null_call: a bare sibling call to a UDF literally --->
<!--- named `new` (the Wheels model-create shape) must dispatch to the UDF, --->
<!--- and a method call on a null receiver must throw — composed, the two --->
<!--- gaps turn Wheels' create() into a silent no-op that reports success. --->
<cf_runtest file="core/test_new_udf_dispatch_and_null_call.cfm">
<!--- - struct_key_case_parity: struct keys are case-insensitive on WRITE, not --->
<!--- just read — a differently-cased write must update the existing key in --->
<!--- place (one key; first-written casing wins the key list), never fork a --->
<!--- second physical key. Surfaced booting Wheels (params / option structs --->
<!--- written under one casing and read under another). --->
<cf_runtest file="core/test_struct_key_case_parity.cfm">
<!--- - serializejson_arguments_sentinels: SerializeJSON must filter the internal --->
<!--- __arguments_scope/__arguments_params sentinels (structKeyList/Count/Exists/ --->
<!--- for-in already do). A struct built via structAppend(s, arguments) otherwise --->
<!--- leaks them into JSON; breaks Wheels helpers that serialize copied arg structs. --->
<cf_runtest file="core/test_serializejson_arguments_sentinels.cfm">
<cf_runtest file="core/test_closure_finally_isolation.cfm">
<!--- - delegate annotation metadata: bare + arbitrary-named property annotations --->
<!--- are captured, and component-level annotations surface top-level in --->
<!--- getComponentMetadata (Lucee parity) — the surface WireBox delegation reads. --->
<cf_runtest file="oop/test_delegate_annotation_metadata.cfm">
<!--- - javadoc & inline parameter annotations: `@arg.inject ...` javadoc and --->
<!--- inline `arg inject="..."` attributes surface on the parameter struct in --->
<!--- getMetadata()/getComponentMetadata() — the surface WireBox DI reads for --->
<!--- constructor-argument injection (Preside FeatureService boot). --->
<cf_runtest file="oop/test_javadoc_param_annotations.cfm">

<!--- --- Lucee-compat regression tests (PRs #153/#154/#155/#156) --- --->
<cf_runtest file="comments/test_cfset_expression_comments.cfm">
<cf_runtest file="tags/test_cfloop_list_literal.cfm">
<cf_runtest file="tags/test_script_loop.cfm">
<cf_runtest file="tags/test_tag_attribute_escaped_hash.cfm">

<!--- --- Query of Queries --- --->
<cf_runtest file="qoq/test_qoq_select.cfm">
<cf_runtest file="qoq/test_qoq_aggregates.cfm">
<cf_runtest file="qoq/test_qoq_joins.cfm">
<cf_runtest file="qoq/test_qoq_subqueries_union.cfm">
<cf_runtest file="qoq/test_qoq_custom_functions.cfm">
<cf_runtest file="qoq/test_qoq_rustcfml_ext.cfm">

<cfscript> printSummary(); </cfscript>
