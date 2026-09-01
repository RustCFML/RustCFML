<cfscript>
writeOutput("============================================================" & chr(10));
writeOutput("RustCFML Test Suite" & chr(10));
writeOutput("============================================================" & chr(10) & chr(10));

include "harness.cfm";
</cfscript>


<!--- --- cfconfig --- --->
<cf_runtest file="config/test_cfconfig_loading.cfm">
<cf_runtest file="config/test_cfconfig_datasource.cfm" rustcfmlOnly="true">
<cf_runtest file="config/test_cfconfig_security.cfm">
<cf_runtest file="config/test_app_datasources.cfm">

<!--- --- Core Language --- --->
<cf_runtest file="core/test_variables.cfm">
<cf_runtest file="core/test_keyword_loop_variables.cfm">
<cf_runtest file="core/test_parser_lucee_shapes.cfm">
<cf_runtest file="core/test_throw_mixed_args_superset.cfm" rustcfmlOnly="true" why="Lucee refuses to COMPILE a file containing throw( type=.., 'msg' ); RustCFML accepts it and raises at the call, so the assertion cannot run there">
<cf_runtest file="core/test_elvis_error_scope.cfm">
<cf_runtest file="core/test_arguments_scope_dispatch.cfm">
<cf_runtest file="core/test_unscoped_scope_cascade.cfm">
<cf_runtest file="core/test_return_assignment_expression.cfm">
<cf_runtest file="core/test_var_chained_assignment.cfm">
<cf_runtest file="core/test_local_scope_name_locals.cfm">
<cf_runtest file="core/test_access_identifiers.cfm">
<cf_runtest file="core/test_reserved_word_param_names.cfm">
<cf_runtest file="core/test_preside_boot_lang_fixes.cfm">
<cf_runtest file="core/test_gcm_leading_dot_path.cfm">
<cf_runtest file="core/test_var_indexed_assignment.cfm">
<cf_runtest file="core/test_function_scope_capture.cfm">
<cf_runtest file="core/test_component_method_named_url_scope.cfm">
<cf_runtest file="core/test_component_output_attribute.cfm">
<cf_runtest file="core/test_component_outside_element.cfm">
<cf_runtest file="core/test_cfthread_component_method_scope.cfm">
<cf_runtest file="core/test_bare_call_caller_stack_leak.cfm">
<cf_runtest file="core/test_defaulted_param_variables_clobber.cfm">
<cf_runtest file="core/test_scope_named_default_param.cfm">
<cf_runtest file="core/test_boolean_literal_equality.cfm">
<cf_runtest file="core/test_null_return_no_key.cfm">
<cf_runtest file="core/test_undefined_member_throws.cfm">
<cf_runtest file="core/test_cfcatch_standard_members.cfm">
<cf_runtest file="core/test_bare_call_shadowing_semantics.cfm">
<cf_runtest file="core/test_closure_env_leak.cfm" rustcfmlOnly="true" why="chained call mk(5)(3) in an expression; Lucee's parser rejects it">
<cf_runtest file="functions/test_closure_env_writeback_leak.cfm">
<!--- - closure_captures_local_function (PR #198): a closure captures its --->
<!--- enclosing fn's var-scoped values AND var-scoped FUNCTION expressions. --->
<!--- RustCFML captured plain values but not a `var fn = function(){}` helper --->
<!--- — a bare call from inside a nested closure threw "Variable 'fn' is --->
<!--- undefined". Surfaced in the Wheels suite (helper at top of run() called --->
<!--- from nested it()/describe() closures). --->
<cf_runtest file="core/test_closure_captures_local_function.cfm">
<cf_runtest file="core/test_recursive_closure_var.cfm">
<cf_runtest file="core/test_caller_local_no_leak.cfm">
<cf_runtest file="core/test_scope_member_mutators.cfm">
<cf_runtest file="core/test_cgi_scope_readonly.cfm">
<cf_runtest file="core/test_struct_stored_closure_dotcall.cfm">
<cf_runtest file="core/test_closure_unscoped_reset.cfm">
<cf_runtest file="core/test_closure_param_defaults.cfm">
<cf_runtest file="core/test_server_scope_write.cfm">
<cf_runtest file="core/test_compound_assignment.cfm">
<cf_runtest file="core/test_undeclared_named_args.cfm">
<cf_runtest file="core/test_paren_wrapped_named_args.cfm">
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
<cf_runtest file="core/test_strict_equality.cfm">
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
<cf_runtest file="core/test_closure_loopvar_in_cfc.cfm" rustcfmlOnly="true" why="calls an array element directly (arr[1]()); Lucee reads it as a member function">
<cf_runtest file="core/test_isdefined_component_scope.cfm">
<cf_runtest file="core/test_returnvar_local_leak.cfm">
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
<cf_runtest file="core/test_arg_ref_writeback_deferred.cfm">
<cf_runtest file="core/test_arrow_functions.cfm">
<cf_runtest file="core/test_comma_less_params.cfm" rustcfmlOnly="true">
<cf_runtest file="core/test_required_param_with_default.cfm">
<cf_runtest file="core/test_self_named_default_arg.cfm">
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
<cf_runtest file="core/test_local_write_argumentcollection_collision.cfm">
<cf_runtest file="core/test_rethrow_after_nested_catch.cfm">
<cf_runtest file="core/test_unscoped_nested_autoviv.cfm">
<cf_runtest file="core/test_construction_ordering.cfm">
<cf_runtest file="core/test_variables_this_live_alias.cfm">
<cf_runtest file="core/test_isdefined_variables_scope.cfm">
<cf_runtest file="core/test_argument_reference_nested.cfm">
<cf_runtest file="core/test_language_features.cfm">
<cf_runtest file="core/test_scopes.cfm">
<cf_runtest file="core/test_cgi_magic_scope.cfm">
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
<cf_runtest file="core/test_server_scope.cfm" rustcfmlOnly="true">
<cf_runtest file="core/test_scope_writeback_masa_fixes.cfm">
<cf_runtest file="core/test_web_scope_no_component_leak.cfm">
<cf_runtest file="core/test_setencoding_masa.cfm">
<cf_runtest file="core/test_pagecontext_request_response.cfm">
<cf_runtest file="core/test_localmode.cfm" rustcfmlOnly="true">
<cf_runtest file="core/test_error_context.cfm">
<cf_runtest file="core/test_null_coalescing.cfm">

<!--- --- Data Types --- --->
<cf_runtest file="types/test_null.cfm">
<cf_runtest file="types/test_null_struct_key_absent.cfm">
<cf_runtest file="types/test_boolean.cfm">
<cf_runtest file="types/test_complex_string_coercion.cfm">
<cf_runtest file="types/test_numeric.cfm">
<cf_runtest file="types/test_string.cfm">
<cf_runtest file="types/test_array.cfm">
<cf_runtest file="types/test_array_append_grow.cfm">
<cf_runtest file="types/test_array_reference_semantics.cfm">
<cf_runtest file="types/test_struct.cfm">
<cf_runtest file="types/test_struct_literal_yesno_null_enum.cfm" rustcfmlOnly="true">
<cf_runtest file="types/test_struct_reference_semantics.cfm">
<cf_runtest file="types/test_ordered_struct_literals.cfm">
<cf_runtest file="types/test_dotted_key_struct_literals.cfm">
<cf_runtest file="types/test_nested_writeback.cfm">
<cf_runtest file="types/test_query.cfm">
<cf_runtest file="types/test_query_column.cfm">
<cf_runtest file="types/test_query_columndata_member.cfm">
<!--- A query cell must be a SIMPLE value: IsSimpleValue()=true and SerializeJSON of a --->
<!--- struct holding it preserves the value. RustCFML 0.161.0 returned boxed cells. --->
<cf_runtest file="types/test_query_cell_simple_value.cfm">
<cf_runtest file="types/test_query_null_cell_scalar.cfm">
<cf_runtest file="types/test_query_bare_column_scalar_type.cfm">
<cf_runtest file="types/test_query_reference.cfm">
<cf_runtest file="types/test_query_cell_assignment.cfm">
<cf_runtest file="types/test_java_map_digest_reference.cfm">
<cf_runtest file="types/test_binary.cfm">
<cf_runtest file="types/test_hash_in_strings.cfm">
<cf_runtest file="types/test_string_interpolation_nested_strings.cfm">
<cf_runtest file="types/test_integer_literal_member_access.cfm">
<cf_runtest file="comments/test_hash_in_comments.cfm">
<cf_runtest file="comments/test_tags_in_block_comments.cfm">

<!--- --- Standard Library --- --->
<cf_runtest file="stdlib/test_hash_unknown_algorithm.cfm">
<cf_runtest file="stdlib/test_java_shim_silent_noops.cfm">
<cf_runtest file="stdlib/test_string_functions.cfm">
<cf_runtest file="stdlib/test_string_functions_regex.cfm">
<cf_runtest file="functions/test_string_position_unicode.cfm">
<cf_runtest file="stdlib/test_string_split_member.cfm">
<cf_runtest file="stdlib/test_regex_backspace_in_class.cfm">
<cf_runtest file="stdlib/test_regex_lookaround.cfm" rustcfmlOnly="true">
<cf_runtest file="stdlib/test_inetaddress_loopback.cfm">
<cf_runtest file="stdlib/test_java_fileoutputstream_shim.cfm">
<cf_runtest file="stdlib/test_java_files_shims.cfm">
<cf_runtest file="stdlib/test_java_bytebuffer_baos.cfm">
<cf_runtest file="stdlib/test_path_sanitization.cfm">
<cf_runtest file="stdlib/test_structdelete_return_and_gettime.cfm">
<cf_runtest file="stdlib/test_numberformat_mask_padding.cfm">
<cf_runtest file="stdlib/test_jwt.cfm">
<!--- java.security.Signature/KeyFactory shim surface (RS256 verify+sign) — the vendored jwt-cfml path; RustCFML's java.security stops at MessageDigest. --->
<cf_runtest file="stdlib/test_java_security_signature.cfm">
<!--- - html_sanitize: the org.owasp.validator.html.AntiSamy shim and the --->
<!--- sanitizeHtml() BIF over the same policy-driven sanitiser (GH #325). --->
<cf_runtest file="stdlib/test_html_sanitize.cfm">
<cf_runtest file="stdlib/test_image.cfm" rustcfmlOnly="true">
<cf_runtest file="stdlib/test_xmp.cfm" rustcfmlOnly="true">

<!--- HtmlDocument(): the mutable HTML DOM behind the jsoup adapter. --->
<cf_runtest file="stdlib/test_html_document.cfm" rustcfmlOnly="true" why="HtmlDocument() is a RustCFML builtin — Lucee has no equivalent (its callers reach for the jsoup jar instead)">
<cf_runtest file="stdlib/test_qrcode_and_svg.cfm" rustcfmlOnly="true" why="qrCodeGenerate() and imageReadSvg() are RustCFML builtins — Lucee has neither (its callers reach for the qrgen and batik jars)">
<cf_runtest file="stdlib/test_pdf.cfm" rustcfmlOnly="true" why="the Pdf* builtins are RustCFML-only — Lucee has no equivalent (its callers reach for the PDFBox jar)">
<cf_runtest file="stdlib/test_arithmetic_numeric_strings.cfm">
<cf_runtest file="stdlib/test_encode_for_html_esapi.cfm">
<cf_runtest file="stdlib/test_string_functions_encoding.cfm">
<!--- EncodeForHTMLAttribute must encode attribute-dangerous chars (space, =) per OWASP/Lucee; RustCFML leaves them raw. --->
<cf_runtest file="functions/test_encodeforhtmlattribute_space_equals.cfm">
<cf_runtest file="stdlib/test_array_functions.cfm">
<cf_runtest file="stdlib/test_array_find_complex.cfm">
<cf_runtest file="stdlib/test_arrayfind_closure.cfm">
<cf_runtest file="stdlib/test_arraycontains_circular.cfm">
<cf_runtest file="stdlib/test_arraycontains_returns_index.cfm">
<cf_runtest file="stdlib/test_serializejson_binary.cfm">
<cf_runtest file="stdlib/test_mail_component.cfm">
<cf_runtest file="stdlib/test_arraysort_callback.cfm">
<cf_runtest file="stdlib/test_dateadd_invalid_datepart.cfm">
<cf_runtest file="stdlib/test_file_bif_errors.cfm">
<cf_runtest file="stdlib/test_bcrypt.cfm">
<cf_runtest file="stdlib/test_yaml.cfm">
<cf_runtest file="stdlib/test_validatejson.cfm">
<cf_runtest file="stdlib/test_struct_putall.cfm">
<cf_runtest file="stdlib/test_struct_ci_index.cfm">
<cf_runtest file="stdlib/test_array_higher_order.cfm">
<cf_runtest file="stdlib/test_struct_functions.cfm">
<cf_runtest file="stdlib/test_struct_higher_order.cfm">
<cf_runtest file="stdlib/test_duplicate_deepcopy_flag.cfm">
<cf_runtest file="stdlib/test_math_functions.cfm">
<cf_runtest file="stdlib/test_date_functions.cfm">
<cf_runtest file="stdlib/test_timezone.cfm">
<cf_runtest file="stdlib/test_list_functions.cfm">
<cf_runtest file="stdlib/test_list_rest_literal_remainder.cfm">
<cf_runtest file="stdlib/test_list_higher_order.cfm">
<cf_runtest file="stdlib/test_query_functions.cfm">
<cf_runtest file="stdlib/test_taffy_compat_fixes.cfm" rustcfmlOnly="true" why="throw(attributeCollection=...); Lucee rejects that argument">
<cf_runtest file="stdlib/test_query_null_column.cfm">
<cf_runtest file="stdlib/test_echo_and_mysql_sysvars.cfm">
<cf_runtest file="stdlib/test_query_higher_order.cfm">
<cf_runtest file="stdlib/test_type_checking.cfm">
<cf_runtest file="stdlib/test_conversion.cfm">
<cf_runtest file="stdlib/test_timespan_type.cfm">
<cf_runtest file="stdlib/test_json.cfm">
<cf_runtest file="stdlib/test_serialize.cfm">
<cf_runtest file="stdlib/test_serializejson_control_chars.cfm">
<cf_runtest file="stdlib/test_json_query_roundtrip.cfm">
<cf_runtest file="stdlib/test_arraysort_callback_reverse.cfm">
<cf_runtest file="stdlib/test_file_io.cfm">
<cf_runtest file="stdlib/test_exists_memo_invalidation.cfm">
<!--- Request-scoped fileExists()/directoryExists() memo: positives only, and --->
<!--- every mutating file BIF drops it (GH #299). --->
<cf_runtest file="stdlib/test_existence_cache.cfm" rustcfmlOnly="true" why="environment-specific: server temp path layout">
<!--- Relative file-BIF paths resolve against the BASE TEMPLATE dir (ExpandPath --->
<!--- parity), verified on Lucee 7 — supersedes the CFC-dir behaviour of #171. --->
<cf_runtest file="stdlib/test_file_relative_path.cfm" rustcfmlOnly="true" why="environment-specific: resolves files against the CLI working directory">
<!--- java.util.concurrent shim + createDynamicProxy (ColdBox/Preside async). --->
<cf_runtest file="stdlib/test_java_concurrent.cfm">
<!--- Async kernel: runAsync/Future and _schedule (one-shot + periodic). These --->
<!--- two suites existed but were never wired into the runner, so the GH #314 --->
<!--- periodic-schedule no-op had no coverage. test_schedule self-skips on --->
<!--- engines without `_schedule` (Lucee). --->
<cf_runtest file="async_kernel/test_run_async.cfm">
<cf_runtest file="async_kernel/test_schedule.cfm">
<!--- Engine fixes surfaced by booting ColdBox (case-insensitive locals, --->
<!--- structAppend-from-component, component hashCode, java.time shim). --->
<cf_runtest file="stdlib/test_coldbox_engine_fixes.cfm">
<!--- More ColdBox/Preside boot fixes: comma-less dates, stackTrace key, array --->
<!--- argumentCollection, list-HOF member forms, injected-method receiver --->
<!--- binding, runtime mapping via application action="update". --->
<cf_runtest file="stdlib/test_coldbox_preside_boot_fixes.cfm">
<cf_runtest file="stdlib/test_security.cfm">
<cf_runtest file="stdlib/test_encrypt_lucee_vectors.cfm">
<cf_runtest file="stdlib/test_password_hashing.cfm">
<cf_runtest file="stdlib/test_xml.cfm">
<cf_runtest file="stdlib/test_utility.cfm">
<cf_runtest file="stdlib/test_encoding_functions.cfm">
<cf_runtest file="stdlib/test_url_encoder_policies.cfm">
<cf_runtest file="stdlib/test_binarydecode_excess_padding.cfm">
<cf_runtest file="stdlib/test_base64_hex_codec.cfm">
<cf_runtest file="stdlib/test_query_mutations.cfm">
<cf_runtest file="stdlib/test_query_new_empty_addcolumn_metadata.cfm">
<cf_runtest file="database/test_lucee_query_builder.cfm" rustcfmlOnly="true" why="datasource-less cfquery via the RustCFML VFS/QoQ overlay">
<cf_runtest file="database/test_datasource_list_maxrows.cfm" rustcfmlOnly="true">
<cf_runtest file="database/test_sqlite_datasource_paths.cfm" rustcfmlOnly="true">
<cf_runtest file="database/test_cfquery_tag_forwards_columnkey_maxrows.cfm" rustcfmlOnly="true">
<cf_runtest file="database/test_duplicate_result_columns.cfm" rustcfmlOnly="true">
<cf_runtest file="database/test_update_affected_rows_matched.cfm" rustcfmlOnly="true">

<!--- A non-SELECT statement returns an empty query (docs/known-issues.md §37). --->
<cf_runtest file="database/test_dml_returns_empty_query.cfm">
<cf_runtest file="database/test_query_error_catch_type_database.cfm">
<cf_runtest file="database/test_query_error_sqlstate_members.cfm">
<cf_runtest file="database/test_pg_comment_placeholder_scan.cfm">
<cf_runtest file="database/test_cfqueryparam_shared_instance_threads.cfm">
<cf_runtest file="stdlib/test_date_functions_extra.cfm">
<cf_runtest file="stdlib/test_lucee_json_date_form.cfm">
<cf_runtest file="stdlib/test_locale_functions.cfm">
<cf_runtest file="stdlib/test_java_i18n_shims.cfm">
<cf_runtest file="stdlib/test_cache_functions.cfm">
<cf_runtest file="stdlib/test_cache_properties.cfm" rustcfmlOnly="true">
<cf_runtest file="stdlib/test_higher_order_functions.cfm">
<cf_runtest file="stdlib/test_bitmask_functions.cfm">
<cf_runtest file="stdlib/test_xml_dom_functions.cfm">
<cf_runtest file="stdlib/test_xml_tostring.cfm">
<cf_runtest file="stdlib/test_misc_functions.cfm">
<cf_runtest file="stdlib/test_len_scalar_coercion.cfm">
<cf_runtest file="stdlib/test_len_multibyte_and_output_buffer.cfm">
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
<cf_runtest file="stdlib/test_objectsave.cfm">
<cf_runtest file="stdlib/test_writedump.cfm">
<cf_runtest file="stdlib/test_cfdirectory_type_filter.cfm">
<cf_runtest file="stdlib/test_cfhttp.cfm">
<cf_runtest file="stdlib/test_cfhttp_binary_response.cfm">
<cf_runtest file="stdlib/test_spreadsheet.cfm">
<cf_runtest file="stdlib/test_native_named_args.cfm" rustcfmlOnly="true" why="exercises RustCFML native-class (Rust-backed) method dispatch">
<cf_runtest file="stdlib/test_member_function_coverage.cfm">
<cf_runtest file="stdlib/test_locale_state.cfm">
<cf_runtest file="stdlib/test_binary_as_array.cfm">

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
<cf_runtest file="oop/test_relative_component_metadata.cfm">
<cf_runtest file="oop/test_pseudo_constructor_errors.cfm">
<cf_runtest file="oop/test_flyweight_instance.cfm">
<cf_runtest file="oop/test_reserved_key_visibility.cfm">
<cf_runtest file="oop/test_component_introspection_surface.cfm">
<cf_runtest file="oop/test_property_no_accessors_onmissing.cfm">
<cf_runtest file="oop/test_unquoted_property_accessors.cfm">
<cf_runtest file="oop/test_called_name_alias.cfm">
<cf_runtest file="oop/test_new_returns_init.cfm">
<cf_runtest file="oop/test_pseudo_ctor_parent_this.cfm">
<cf_runtest file="oop/test_variables_this_alias.cfm">
<cf_runtest file="oop/test_variables_scope_struct_copy.cfm">
<cf_runtest file="oop/test_undefined_read_type_in_function.cfm">
<cf_runtest file="oop/test_instance_serialize_cycle.cfm">
<cf_runtest file="oop/test_include_rewrite_freshness.cfm" rustcfmlOnly="true" why="environment-specific: writes and re-reads through a server temp path">
<cf_runtest file="oop/test_accessor_private_iteration.cfm">
<cf_runtest file="oop/test_method_access_gate.cfm">
<cf_runtest file="oop/test_arg_type_validation.cfm">
<cf_runtest file="oop/test_new_keyword_path.cfm">
<!--- - component_internals_serialize_leak: iterating a component (for(k in obj)) --->
<!--- or SerializeJSON(obj) must expose only data members, never engine --->
<!--- internals. RustCFML leaks __name/__source_file/__variables into both, and --->
<!--- SerializeJSON emits methods as null keys. Breaks Wheels model --->
<!--- properties() (for-in over `this`) and renderWith(data=modelObject) — a --->
<!--- single-record JSON response comes back as ~379 internal keys. Runtime-safe. --->
<cf_runtest file="oop/test_overflow_arg_no_leak.cfm">
<cf_runtest file="oop/test_component_internals_serialize_leak.cfm">
<cf_runtest file="oop/test_component_method_name_leak.cfm">
<cf_runtest file="oop/test_component_method_builtin_name.cfm">
<cf_runtest file="oop/test_component_name_builtin_collision.cfm">
<cf_runtest file="oop/test_component_return_type.cfm">
<cf_runtest file="oop/test_inheritance.cfm">
<cf_runtest file="oop/test_super_case_insensitive_this.cfm">
<cf_runtest file="oop/test_inherited_helpers.cfm" rustcfmlOnly="true" why="reads __variables, a RustCFML-internal backing marker">
<cf_runtest file="oop/test_mixin_self_dispatch.cfm">
<cf_runtest file="oop/test_component_body_chained_assignment_aliasing.cfm">
<cf_runtest file="oop/test_inherited_chained_assignment_aliasing.cfm">
<cf_runtest file="oop/test_implicit_accessor_constructor.cfm">
<cf_runtest file="oop/test_interfaces.cfm">
<cf_runtest file="oop/test_implements_unqualified_sibling.cfm">
<cf_runtest file="oop/test_inherited_relative_implements.cfm">
<cf_runtest file="oop/test_unqualified_new_package_relative.cfm">
<cf_runtest file="oop/test_unqualified_new_inherited_package.cfm">
<cf_runtest file="oop/test_inherited_new_mapping_qualified_fqn.cfm">
<cf_runtest file="oop/test_component_ctor_method_hoist.cfm">
<cf_runtest file="gc/test_incremental_cycle_sweep.cfm">
<cf_runtest file="oop/test_runtime_added_method_via_structappend.cfm">
<cf_runtest file="oop/test_injected_method_instance_isolation.cfm">
<cf_runtest file="oop/test_metadata.cfm">
<cf_runtest file="oop/test_mock_mixin_injection.cfm">
<cf_runtest file="oop/test_mock204_backref_writeback.cfm">
<cf_runtest file="oop/test_dotted_function_names.cfm">
<cf_runtest file="oop/test_static.cfm">
<cf_runtest file="oop/test_soft_keyword_function_name.cfm">
<cf_runtest file="oop/test_preside_serve_fixes.cfm">
<cf_runtest file="oop/test_preside_testbox_fixes.cfm">
<cf_runtest file="oop/test_injected_method_this_binding.cfm">
<cf_runtest file="oop/test_property_attributes.cfm">
<cf_runtest file="oop/test_struct_method_dispatch.cfm">
<cf_runtest file="oop/test_external_prop.cfm">
<cf_runtest file="oop/test_repeated_instantiation.cfm">
<cf_runtest file="oop/test_component_mapping_paths.cfm">
<cf_runtest file="functions/test_dynamic_named_args.cfm">
<cf_runtest file="functions/test_builtin_name_ci_index.cfm">
<cf_runtest file="functions/test_direct_builtin_lowering.cfm">
<cf_runtest file="functions/test_fused_variables_property.cfm">
<cf_runtest file="functions/test_param_scope_ownership.cfm">
<cf_runtest file="functions/test_arguments_scope_lever_c.cfm">
<cf_runtest file="oop/test_component_method_named_args.cfm">
<cf_runtest file="oop/test_component_method_precedence.cfm">
<cf_runtest file="oop/test_method_ref_binding.cfm">
<cf_runtest file="oop/test_returned_service_chain.cfm">
<cf_runtest file="oop/test_mixin_writeback.cfm">
<cf_runtest file="oop/test_nested_cfc_method_writeback.cfm">
<cf_runtest file="oop/test_closure_writeback_cfc_boundary.cfm">
<cf_runtest file="oop/test_property_method_name_collision.cfm">
<cf_runtest file="oop/test_new_named_args.cfm">
<cf_runtest file="oop/test_dynamic_lhs_assign.cfm">
<cf_runtest file="oop/test_getmetadata_properties.cfm">
<cf_runtest file="oop/test_function_return_type_metadata.cfm">
<cf_runtest file="oop/test_function_annotation_metadata.cfm">
<cf_runtest file="oop/test_getmetadata_extends_functions.cfm">
<cf_runtest file="oop/test_getmetadata_fidelity.cfm">
<cf_runtest file="oop/test_metadata_implements_extends.cfm">
<cf_runtest file="oop/test_gcm_path_metadata_cache.cfm">
<cf_runtest file="oop/test_component_bool_attr.cfm">
<cf_runtest file="oop/test_chained_writeback_clobber.cfm">
<cf_runtest file="oop/test_instance_shares_injected_component.cfm">
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
<!--- An actively-held named lock must never be evicted from the >1024-entry registry --->
<!--- cap while its 'static guard is live. Pre-fix the eviction sweep freed the held --->
<!--- lock's RwLock and dropping the dangling guard SIGSEGV'd serve mode (found via --->
<!--- Preside's PresideObjectServiceTest, which acquires thousands of per-object locks). --->
<cf_runtest file="core/test_named_lock_eviction_uaf.cfm">
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
<cf_runtest file="tags/test_script_child_tags.cfm">
<cf_runtest file="tags/test_zip_tag_params.cfm">
<cf_runtest file="tags/test_tag_preprocessor_masa_fixes.cfm">
<cf_runtest file="tags/test_cfmodule_script_form.cfm">
<cf_runtest file="tags/test_seethrough_udf_variables.cfm">
<cf_runtest file="tags/test_tags_basic.cfm">
<cf_runtest file="tags/test_tags_control.cfm">
<cf_runtest file="tags/test_tags_include.cfm">
<cf_runtest file="tags/test_cfinclude_css.cfm">
<cf_runtest file="tags/test_tags_cffunction_hoisting.cfm">
<cf_runtest file="tags/test_tags_cfargument_namespaced_attr.cfm">
<cf_runtest file="tags/test_tags_savecontent.cfm">
<cf_runtest file="tags/test_tags_param.cfm" rustcfmlOnly="true">
<cf_runtest file="tags/test_tags_param_dynamic.cfm">
<!--- - cfparam_bracket_key_colon: cfparam name= is an lvalue path whose --->
<!--- bracket segment holds a quoted string key -- including one with a --->
<!--- colon (name="item['x-bind:href']", the Alpine.js attribute shape). --->
<!--- RustCFML rejects it at PARSE time ("Expected RBracket, found Colon") --->
<!--- though the same key works in ordinary expressions (inline control). --->
<!--- Fixture-contained (parse errors escape try/catch); dotted-name control --->
<!--- fixture guards the wiring. Reduced from the titan (Moopa) codebase port. --->
<cf_runtest file="tags/test_cfparam_bracket_key_colon.cfm">
<cf_runtest file="tags/test_tags_cfoutput_query.cfm">
<cf_runtest file="tags/test_tags_misc.cfm">
<cf_runtest file="tags/test_tags_cfsleep.cfm">
<cf_runtest file="tags/test_tags_cfhtmlhead_body.cfm">
<cf_runtest file="tags/test_tags_cfexit.cfm">
<cf_runtest file="tags/test_tags_customtag.cfm">
<cf_runtest file="tags/test_custom_tag_caller_cfc_method.cfm">
<cf_runtest file="tags/test_customtag_caller_semantics.cfm">
<cf_runtest file="tags/test_customtag_body_cfoutput.cfm">
<cf_runtest file="tags/test_getbasetag_instance_numbers.cfm">
<cf_runtest file="tags/test_cffunction_output_true_body.cfm">
<cf_runtest file="tags/test_getbasetag_functions.cfm">
<cf_runtest file="tags/test_custom_tag_attribute_collection.cfm">
<cf_runtest file="tags/test_cfthread_module_attributes.cfm">
<cf_runtest file="tags/test_tags_customtag_lifecycle.cfm">
<!--- - customtag_path_deep_search: a custom tag in a SUBDIRECTORY of a --->
<!--- this.customtagpaths dir must resolve (Lucee with customTagDeepSearch --->
<!--- on, which is how Lucee-based projects ship). RustCFML resolves the --->
<!--- path root (in-suite control) but never descends: "Custom tag --->
<!--- 'cf_ctpath_deep' not found". Runtime-level (catchable), runner-safe. --->
<!--- Fixture dirs under tests/tags/ctpathroot/; the path itself is declared --->
<!--- in tests/Application.cfc. Reduced from the titan (Moopa) codebase port. --->
<cf_runtest file="tags/test_customtag_path_deep_search.cfm">
<cf_runtest file="tags/test_customtag_hyphenated_attributes.cfm">
<cf_runtest file="tags/test_customtag_prefix_body_output.cfm">
<cf_runtest file="tags/test_loop_file_lines.cfm">
<cf_runtest file="tags/test_tags_buffer_recovery.cfm">
<cf_runtest file="tags/test_tags_cfexecute.cfm">
<cf_runtest file="tags/test_cfexecute_argument_quoting.cfm">
<cf_runtest file="tags/test_cfqueryparam_list_array_value.cfm">
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
<cf_runtest file="tags/test_db_null_column_empty_string.cfm">
<cf_runtest file="tags/test_pg_extended_param_binds.cfm">
<cf_runtest file="tags/test_pg_pool_checkout_validation.cfm">
<cf_runtest file="tags/test_pg_pool_stale_connection_retry.cfm">
<cf_runtest file="tags/test_mssql_pool_stale_connection_retry.cfm">
<cf_runtest file="tags/test_cfquery_quoted_identifier.cfm">
<!--- - cfquery_escaped_hash_interpolation: an escaped hash immediately --->
<!--- followed by an interpolation inside a cfquery-body string literal --->
<!--- ('Order ###x#' -> "Order #5") must lower like it does in cfoutput/cfset --->
<!--- (inline control). RustCFML rejects the body at PARSE time ("Expected --->
<!--- RParen, found Identifier"). Fixture-contained; QoQ keeps the fixtures --->
<!--- executable with no datasource, so the VALUE is asserted, not just --->
<!--- parseability. Reduced from the titan (Moopa) codebase port. --->
<cf_runtest file="tags/test_cfquery_escaped_hash_interpolation.cfm">
<cf_runtest file="tags/test_cfquery_sql_line_comments.cfm">
<cf_runtest file="tags/test_cfquery_cfml_comments.cfm">
<cf_runtest file="tags/test_cte_with_query.cfm">
<cf_runtest file="tags/test_tags_cfquery_control_tags.cfm">
<cf_runtest file="tags/test_cfquery_result_delivery.cfm">
<cf_runtest file="tags/test_cfdbinfo.cfm">
<cf_runtest file="tags/test_cfdirectory_mapping_path.cfm">
<cf_runtest file="tags/test_cfdirectory_function_form.cfm">
<cf_runtest file="tags/test_cfdirectory_sort.cfm">
<cf_runtest file="tags/test_cfdirectory_recurse_symlink.cfm">
<cf_runtest file="tags/test_cfdirectory_listinfo_name_relative.cfm">
<cf_runtest file="tags/test_mapping_dotdot_normalization.cfm">
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
<cf_runtest file="tags/test_cfthread_shared_component.cfm">
<cf_runtest file="functions/test_extends_missing_parent.cfm">
<cf_runtest file="tags/test_application_scope_concurrency.cfm">
<cf_runtest file="tags/test_cflock_timeout_semantics.cfm">
<cf_runtest file="tags/test_tags_cflock_scope.cfm">
<cf_runtest file="tags/test_tags_cfscript_statements.cfm">
<cf_runtest file="tags/test_static_conditional_get.cfm">
<cf_runtest file="tags/test_cfcookie_path_samesite.cfm">
<cf_runtest file="tags/test_cfcookie_inrequest_readback.cfm">
<cf_runtest file="tags/test_tags_cfhttp_interpolation.cfm">
<cf_runtest file="tags/test_cfhttp_attribute_collection.cfm">
<cf_runtest file="tags/test_tags_cfhttp_name_file.cfm">
<cf_runtest file="tags/test_tags_cfdump_output_abort.cfm">
<cf_runtest file="tags/test_tags_cffile_nameconflict.cfm">
<cf_runtest file="tags/test_tags_cffile_charset.cfm">
<cf_runtest file="tags/test_tags_unclosed_body_tags.cfm">
<cf_runtest file="tags/test_tags_cfinvoke_webservice.cfm">
<cf_runtest file="tags/test_throw_object_rootcause.cfm">
<cf_runtest file="tags/test_cfloop_file_and_includes.cfm">
<cf_runtest file="tags/test_tags_cfhttp_multipart.cfm">
<cf_runtest file="tags/test_cfhttp_timeout_interpolation.cfm">
<cf_runtest file="tags/test_cfexecute_interpolation.cfm">
<cf_runtest file="tags/test_tag_attribute_interpolation_sweep.cfm">
<cf_runtest file="tags/test_tags_cfhttpparam_interpolation.cfm">
<cf_runtest file="tags/test_tag_string_interpolation.cfm">
<cf_runtest file="tags/test_tag_attribute_interpolation.cfm">
<cf_runtest file="tags/test_tag_return_nested_quote_interpolation.cfm">
<cf_runtest file="tags/test_tags_cfoutput_nested_interp.cfm">
<cf_runtest file="tags/test_cffinally_tag_body.cfm">
<cf_runtest file="tags/test_cflog_cfmail_attribute_interpolation.cfm">
<cf_runtest file="tags/test_cflog_file_logging.cfm">
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

<!--- Application.cfc this.timezone / this.locale (docs/known-issues.md §1). --->
<cf_runtest file="lifecycle/test_application_timezone_locale.cfm">
<cf_runtest file="lifecycle/test_application_pseudo_include.cfm">
<cf_runtest file="lifecycle/test_application_shared_variables.cfm">

<!--- super.method() from an Application.cfc body with no materialized `this`:
     the parent method must still see the component method table (GH #360 fallout). --->
<cf_runtest file="lifecycle/test_application_super_pseudoctor_vars.cfm">

<!--- ExpandPath and the file BIFs must agree when a mapping target is a symlink. --->
<cf_runtest file="lifecycle/test_mapping_symlink_paths.cfm">
<cf_runtest file="lifecycle/test_application_lifecycle_case_override.cfm">
<cf_runtest file="lifecycle/test_application_load_errors.cfm">

<!--- onApplicationStart must gate ALL concurrent requests, not just the one that
     triggers it: bypassers observe the half-built application scope (boot-window
     500s/404s under live traffic). --->
<cf_runtest file="lifecycle/test_application_start_serialisation.cfm">
<cf_runtest file="lifecycle/test_application_scope_custom_tag.cfm">
<cf_runtest file="lifecycle/test_application_onerror_onabort.cfm">
<cf_runtest file="lifecycle/test_appscope_receiver_no_resurrect.cfm">
<cf_runtest file="server/test_front_controller_fallback.cfm">
<cf_runtest file="server/test_location_redirect.cfm">
<cf_runtest file="server/test_form_duplicate_fields.cfm">

<!--- --- Java Shims --- --->
<cf_runtest file="java_shims/test_all.cfm">
<cf_runtest file="java_shims/test_java_regex_quote_and_replacement.cfm">
<cf_runtest file="java_shims/test_java_date_shim_date_bifs.cfm">
<cf_runtest file="java_shims/test_java_util_base64.cfm">
<cf_runtest file="java_shims/test_comprehensive.cfm">
<cf_runtest file="java_shims/test_more.cfm">
<cf_runtest file="java_shims/test_security.cfm">
<cf_runtest file="java_shims/test_stringbuilder.cfm">
<cf_runtest file="java_shims/test_system.cfm">
<cf_runtest file="java_shims/test_system_properties.cfm">
<cf_runtest file="java_shims/test_file_mkdirs_separator.cfm">
<cf_runtest file="java_shims/test_concurrent_map.cfm">
<cf_runtest file="java_shims/test_map_higher_order.cfm">
<cf_runtest file="java_shims/test_soft_reference.cfm">
<cf_runtest file="java_shims/test_java_url.cfm">
<cf_runtest file="java_shims/test_property_bundle.cfm">
<cf_runtest file="java_shims/test_classloader_shims.cfm">
<cf_runtest file="java_shims/test_commons_imaging.cfm">
<cf_runtest file="java_shims/test_esapi_security_config.cfm" rustcfmlOnly="true" why="asserts RustCFML's ESAPI class-name shim; a CommandBox Lucee has no ESAPI on the classpath, so createObject fails there at the first line">

<!--- Adobe XMPCore, the JCE crypto surface, opencsv + java.io writers,
     java.util.StringTokenizer/Properties and JavaMail's connection probe. --->
<cf_runtest file="java_shims/test_xmp_core.cfm" rustcfmlOnly="true" why="asserts RustCFML's XMPCore adapter; Lucee needs the real xmpcore.jar, which the test deliberately does not supply">
<cf_runtest file="java_shims/test_javax_crypto.cfm" rustcfmlOnly="true" why="asserts the JCE adapter and the randomBytes() BIF, neither of which exists on Lucee (it has the real JVM classes)">
<cf_runtest file="java_shims/test_opencsv_writer.cfm" rustcfmlOnly="true" why="asserts the opencsv adapter and the csvFormatRow() BIF; Lucee needs the real opencsv jar">
<cf_runtest file="java_shims/test_stringtokenizer_properties.cfm" rustcfmlOnly="true" why="asserts RustCFML's StringTokenizer/Properties shims; Lucee has the real java.util classes and differs on null handling">
<cf_runtest file="java_shims/test_javax_mail_probe.cfm" rustcfmlOnly="true" why="asserts the JavaMail adapter and the smtpConnectionTest() BIF, which is RustCFML-only">
<cf_runtest file="java_shims/test_poi_spreadsheet.cfm" rustcfmlOnly="true" why="asserts the POI-over-native-spreadsheet adapter; Lucee has no POI on the classpath here">
<cf_runtest file="java_shims/test_osgi_inert.cfm" rustcfmlOnly="true" why="asserts that RustCFML's OSGi plumbing is inert; on Lucee it is real and installs bundles">
<cf_runtest file="java_shims/test_jsoup_html.cfm" rustcfmlOnly="true" why="asserts the jsoup-over-HtmlDocument adapter; Lucee needs the real jsoup jar, which the test does not supply">
<cf_runtest file="java_shims/test_qrgen_batik.cfm" rustcfmlOnly="true" why="asserts the qrgen and batik adapters; Lucee needs the real jars, which the tests deliberately do not supply">
<cf_runtest file="java_shims/test_pdfbox.cfm" rustcfmlOnly="true" why="asserts the PDFBox-over-hayro adapter; Lucee needs the real PDFBox jar, which the test does not supply">
<cf_runtest file="java_shims/test_optional.cfm">
<cf_runtest file="java_shims/test_masa_io_text_shims.cfm" rustcfmlOnly="true" why="passes a locale shim to java.text.MessageFormat; real Java has no such constructor">

<!--- --- Engine Compatibility --- --->
<cf_runtest file="compat_engine/test_math_functions.cfm">
<cf_runtest file="compat_engine/test_string_functions.cfm">
<cf_runtest file="compat_engine/test_struct_functions.cfm">
<cf_runtest file="compat_engine/test_array_functions.cfm">
<cf_runtest file="compat_engine/test_list_functions.cfm">
<cf_runtest file="compat_engine/test_query_functions.cfm">
<cf_runtest file="compat_engine/test_wheels_batch_v0294.cfm">
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
<!--- Exceptions: test_s3_presigned_url_lucee_compat.cfm and --->
<!--- test_s3_app_config.cfm are PURE — presigning is offline computation, so --->
<!--- they need no credentials, no bucket and no network, and run the same way --->
<!--- with no setup at all. They stay in this directory with their siblings. --->

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
<cf_runtest file="core/test_classic_localmode_writeback.cfm">
<cf_runtest file="core/test_local_at_template_scope.cfm">
<cf_runtest file="core/test_local_at_page_scope.cfm">
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
<cf_runtest file="oop/test_struct_keys_component.cfm">
<!--- A parent's displayName attribute must NOT be copied onto a child's leaf metadata. --->
<!--- RustCFML 0.161.0 propagates it; Lucee/ACF/BoxLang leave it absent on the leaf. --->
<cf_runtest file="oop/test_getmetadata_inherited_displayname.cfm">
<cf_runtest file="tags/test_tags_script_syntax_body.cfm">
<cf_runtest file="functions/test_expandpath_trailing_slash.cfm">
<cf_runtest file="functions/test_expandpath_leading_slash_caller_independent.cfm">
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
<cf_runtest file="core/test_abort_soft_keyword.cfm">
<cf_runtest file="core/test_param_attr_order.cfm">
<cf_runtest file="core/test_date_arithmetic_serial.cfm">
<!--- - cfinvoke_statement: `invoke` as a CFScript statement (attributes + optional --->
<!--- invokeargument block) is now compiled to __cfinvoke(...). RustCFML previously --->
<!--- only supported the <cfinvoke> tag and the invoke(...) call forms. (RustCFML --->
<!--- also accepts the ACF-style cf-prefixed `cfinvoke` spelling, but Lucee --->
<!--- rejects it, so the cross-engine test uses the cf-less `invoke`.) --->
<cf_runtest file="tags/test_cfinvoke_statement.cfm">
<cf_runtest file="tags/test_cfinvoke_argument_children.cfm">
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
<cf_runtest file="tags/test_transaction_block_exit.cfm">
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
<cf_runtest file="core/test_isinstanceof_native_types.cfm">
<cf_runtest file="core/test_isinstanceof_component_base.cfm">
<cf_runtest file="core/test_session_holds_component.cfm">
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
<cf_runtest file="core/test_catch_reserved_dotted_type.cfm">
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
<cf_runtest file="core/test_cfloop_list_item_with_index.cfm">
<cf_runtest file="core/test_switch_continue_in_loop.cfm">
<cf_runtest file="core/test_application_scope_persist.cfm">
<cf_runtest file="core/test_application_name_implicit.cfm">
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
<cf_runtest file="core/test_finally_on_break_continue.cfm">
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
<cf_runtest file="core/test_argumentcollection_array_and_throw_expr.cfm">
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
<cf_runtest file="core/test_closure_finally_isolation.cfm" rustcfmlOnly="true" why="LUCEE DEFECT: `variables.x` is unreachable inside a finally block there (unscoped works); nothing to fix here">
<!--- - closures read the LIVE variables scope: a SCOPED write (variables.x = …) --->
<!--- must reach closures created earlier, exactly as the unscoped form already --->
<!--- did, and a closure must be able to call itself by the name it was assigned --->
<!--- to (the scheduling idiom). GitHub #316. --->
<cf_runtest file="core/test_closure_live_variables_scope.cfm">
<!--- - delegate annotation metadata: bare + arbitrary-named property annotations --->
<!--- are captured, and component-level annotations surface top-level in --->
<!--- getComponentMetadata (Lucee parity) — the surface WireBox delegation reads. --->
<cf_runtest file="oop/test_delegate_annotation_metadata.cfm">
<!--- - javadoc & inline parameter annotations: `@arg.inject ...` javadoc and --->
<!--- inline `arg inject="..."` attributes surface on the parameter struct in --->
<!--- getMetadata()/getComponentMetadata() — the surface WireBox DI reads for --->
<!--- constructor-argument injection (Preside FeatureService boot). --->
<cf_runtest file="oop/test_javadoc_param_annotations.cfm">
<cf_runtest file="oop/test_cfinvoke_sibling_scope.cfm">
<cf_runtest file="oop/test_cfinvoke_overlay_writeback.cfm">
<cf_runtest file="oop/test_mixin_private_scope_dispatch.cfm">
<cf_runtest file="oop/test_struct_get_vs_component_method.cfm">
<cf_runtest file="oop/test_fluent_return_this_chain.cfm">
<cf_runtest file="oop/test_super_ctor_this_writes.cfm">
<!--- Component-path resolution cache keying: caller DIR, case-insensitive --->
<!--- class name, verified hits (GH #298). --->
<cf_runtest file="oop/test_component_resolution_cache.cfm">

<!--- Metadata executed-template cache: shared parents derive independently, a --->
<!--- mutating caller cannot poison the memo, and instantiation still runs the --->
<!--- pseudo-constructor every time (the cache is metadata-only by design). --->
<cf_runtest file="oop/test_component_metadata_template_cache.cfm">

<!--- --- Lucee-compat regression tests (PRs #153/#154/#155/#156) --- --->
<cf_runtest file="comments/test_cfset_expression_comments.cfm">
<cf_runtest file="tags/test_cfloop_list_literal.cfm">
<cf_runtest file="tags/test_cfloop_times.cfm">
<cf_runtest file="tags/test_cfoutput_encodefor.cfm">
<cf_runtest file="tags/test_script_loop.cfm" rustcfmlOnly="true">
<cf_runtest file="tags/test_cfloop_query_currentrow.cfm">
<cf_runtest file="tags/test_tags_cfloop_query_window_group.cfm">
<!--- - cfloop_query_bare_column: inside <cfloop query>, a BARE column --->
<!--- reference (expression read or interpolated output) must resolve to the --->
<!--- current row like the scoped q.col form -- the same row-into-scope merge --->
<!--- <cfoutput query> already does (§13). RustCFML throws "Variable 'grp' is --->
<!--- undefined" for both bare forms; the scoped spelling is the in-suite --->
<!--- control. Runtime-level (catchable), runner-safe. Reduced from the titan --->
<!--- (Moopa) codebase port. --->
<cf_runtest file="tags/test_cfloop_query_bare_column.cfm">
<cf_runtest file="tags/test_tags_cfexit_loop.cfm">
<cf_runtest file="tags/test_cfcookie_attributecollection.cfm">
<cf_runtest file="tags/test_tag_attribute_escaped_hash.cfm">
<cf_runtest file="tags/test_tag_attribute_escaped_quotes.cfm">
<cf_runtest file="tags/test_tag_unquoted_attr_literal.cfm">
<!--- - cfargument_hash_struct_default: an UNQUOTED hash-wrapped tag --->
<!--- attribute may hold a STRUCT LITERAL (default=#{ "type": "json_object" }#); --->
<!--- Lucee evaluates it as the argument default. RustCFML rejects it at --->
<!--- PARSE time ("Unterminated '#' interpolation", reported away from the --->
<!--- attribute) while the simple-expression form default=#lCase(..)# parses --->
<!--- (control fixture). Fixture-contained (parse errors escape try/catch). --->
<!--- Reduced from the titan (Moopa) codebase port. --->
<cf_runtest file="tags/test_cfargument_hash_struct_default.cfm">
<!--- - tag_mode_arrow_function: an arrow function is an expression, so it --->
<!--- may appear in a TAG-MODE expression (<cfset t = arr.reduce((s, r) => --->
<!--- s + r.count, 0)>), expression or block body. RustCFML rejects both at --->
<!--- PARSE time ("Expected RParen, found Comma") while the identical code --->
<!--- in cfscript and the classic function(){} spelling in tag mode both --->
<!--- work (in-suite controls). Fixture-contained (parse errors escape --->
<!--- try/catch). Found running titan (Moopa) on v0.574.0. --->
<cf_runtest file="tags/test_tag_mode_arrow_function.cfm">
<cf_runtest file="tags/test_cfspreadsheet.cfm" rustcfmlOnly="true">

<!--- --- Query of Queries --- --->
<cf_runtest file="qoq/test_qoq_select.cfm">
<cf_runtest file="qoq/test_qoq_dotted_table.cfm">
<cf_runtest file="qoq/test_qoq_aggregates.cfm">
<cf_runtest file="qoq/test_qoq_joins.cfm">
<cf_runtest file="qoq/test_qoq_subqueries_union.cfm">
<cf_runtest file="qoq/test_qoq_custom_functions.cfm">
<cf_runtest file="qoq/test_qoq_rustcfml_ext.cfm">
<cf_runtest file="qoq/test_queryexecute_maxrows_list.cfm">
<cf_runtest file="qoq/test_queryexecute_returntype_result.cfm">
<cf_runtest file="qoq/test_qoq_component_variables_scope.cfm">
<cf_runtest file="qoq/test_qoq_arguments_scope.cfm">

<!--- WebSocket / realtime: connection-free harness coverage (live-socket --->
<!--- behaviour is covered by crates/cli/tests/websocket_raw.rs). --->
<cf_runtest file="websocket/test_ws_harness.cfm">

<!--- Observability: classic CF debug-footer BIFs. Footer activation/render --->
<!--- (serve-only) is covered by Rust gate tests in cfml-vm/src/lib.rs. --->
<cf_runtest file="observe/test_debug_footer.cfm" rustcfmlOnly="true">
<cf_runtest file="observe/test_profiler.cfm">

<!--- Declared parameter/return types are enforced (docs/known-issues.md §29). --->
<cf_runtest file="functions/test_fn_type_enforcement.cfm">

<!--- Writes through the explicit `local.` scope prefix (perf plan T3.1 stage 1.5). --->
<cf_runtest file="functions/test_local_scope_member_writes.cfm">
<!--- Elvis ?: absorbs exceptions from its left operand, not just undefined reads (GH #329). --->
<cf_runtest file="functions/test_elvis_error_scope.cfm">

<!--- Page-scope variables holding a function (docs/known-issues.md §32). --->
<cf_runtest file="functions/test_page_function_vars.cfm">

<!--- Self-typed method called from a pseudo-constructor (docs/known-issues.md §35). --->
<cf_runtest file="functions/test_pc_self_type.cfm">

<!--- Query columns vs declared types (docs/known-issues.md §35). --->
<cf_runtest file="types/test_querycolumn_declared_types.cfm">

<!--- java.lang.Object methods on simple values (docs/known-issues.md §33). --->
<cf_runtest file="functions/test_java_object_methods.cfm">

<!--- createUUID / createUniqueID shape (docs/known-issues.md §34). --->
<cf_runtest file="stdlib/test_uuid_shape.cfm">

<!--- hash() must digest binary BYTES, not a lossy string form of them (GH #376). --->
<cf_runtest file="stdlib/test_hash_binary.cfm">

<!--- java.io.File two-argument (parent, child) constructor (GH #378). --->
<cf_runtest file="java_shims/test_java_file_two_arg.cfm">

<!--- getTempDirectory() ends with a separator, so `& name` joins inside it (GH #380). --->
<cf_runtest file="stdlib/test_gettempdirectory_separator.cfm">

<!--- A keyword-named component-path segment keeps its source case (GH #381). --->
<cf_runtest file="core/test_keyword_component_path_case.cfm">

<!--- <cfapplication> is implemented, not a "tag not implemented" 500 (GH #374). --->
<cf_runtest file="tags/test_cfapplication_tag.cfm">

<cfscript> printSummary(); </cfscript>
