<cfscript>
suiteBegin("Custom tags: attributeCollection");

control_attrs = {
    model: "current_record.full_name",
    placeholder: "Full name",
    class: "input w-full"
};
</cfscript>

<cfsavecontent variable="moduleOutput">
    <cfmodule template="customtags/showattrs.cfm" attributeCollection="#control_attrs#" model="override.model">
</cfsavecontent>

<cfscript>
assert("cfmodule merges attributeCollection and explicit attrs override", trim(moduleOutput), "override.model|Full name|input w-full");
assert("cfmodule attributeCollection source struct is not mutated", control_attrs.model, "current_record.full_name");

control_attrs = {
    model: "current_record.email",
    label: "Email",
    class: "input w-full"
};
</cfscript>

<cfsavecontent variable="prefixOutput">
    <cf_showattrs attributeCollection="#control_attrs#" model="override.email"></cf_showattrs>
</cfsavecontent>

<cfscript>
assert("cf_ custom tag merges attributeCollection and explicit attrs override", trim(prefixOutput), "override.email|Email|input w-full");
assert("cf_ custom tag attributeCollection source struct is not mutated", control_attrs.model, "current_record.email");

// attributeCollection whose #expr# contains a NESTED #...# must still arrive as
// a STRUCT (native type), not be string-coerced. Regression for Masa's
// <cfdbinfo attributeCollection="#getQueryAttrs(name='rs',table='#f(t)#',...)#">
// which broke with "Missing attribute [name]" when the value was stringified.
function buildAttrs(model, label, class) { return arguments; }
function ident(s) { return arguments.s; }
</cfscript>
<cfsavecontent variable="nestedOutput"><cf_showattrs attributeCollection="#buildAttrs(model='#ident('nested.model')#', label='Nested', class='c1')#"></cf_showattrs></cfsavecontent>
<cfscript>
assert("attributeCollection with nested ##...## stays a struct (not string-coerced)",
       trim(nestedOutput), "nested.model|Nested|c1");

suiteEnd();
</cfscript>
