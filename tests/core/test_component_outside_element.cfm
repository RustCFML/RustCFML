<cfscript>
// Content outside a component's own <cfcomponent>...</cfcomponent> element
// belongs to no component: Lucee 7.1.0+204 neither emits its text nor runs its
// code, before the open tag or after the close tag. RustCFML did both, so the
// trailing newline every editor leaves after </cfcomponent> was written into
// the response on every instantiation of a tag-based CFC -- invisible in HTML,
// one stray leading byte for text/plain, CSV and JSON (issue #375).
//
// Both fixtures declare output="true" and keep their bodies on ONE line, so
// there is no inter-tag whitespace to suppress and nothing here can pass just
// because <cfcomponent output="false"> silences the whole body (issue #373,
// covered by test_component_output_attribute.cfm). What these assert is
// specifically that the OUTSIDE of the element contributes nothing.
suiteBegin("Content outside the component element");

request.outsideBefore = "untouched";
savecontent variable="beforeOut" {
    ob = createObject("component", "core.component_output.OutsideBeforeCfc");
}
assert("text before <cfcomponent> is not emitted", len(beforeOut), 0);
assert("code before <cfcomponent> does not run", request.outsideBefore, "untouched");
assert("...and the component itself still works", ob.m(), "M");

request.outsideAfter = "untouched";
savecontent variable="afterOut" {
    oa = createObject("component", "core.component_output.OutsideAfterCfc");
}
assert("text after </cfcomponent> is not emitted", len(afterOut), 0);
assert("code after </cfcomponent> does not run", request.outsideAfter, "untouched");
assert("...and the component itself still works", oa.m(), "M");

suiteEnd();
</cfscript>
