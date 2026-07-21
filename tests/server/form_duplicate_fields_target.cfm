<cfsetting enablecfoutputonly="true" />
<!---
    Target for test_form_duplicate_fields.cfm. Echoes the received `dup` value
    from whichever scope the caller exercised, so the caller can assert how
    duplicate keys were merged (comma-join, Lucee semantics) rather than
    last-one-wins.
--->
<cfoutput>form=[#form.dup ?: "(missing)"#];url=[#url.dup ?: "(missing)"#]</cfoutput>
