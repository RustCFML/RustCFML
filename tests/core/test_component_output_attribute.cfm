<cfscript>
// <cfcomponent output="false"> must suppress the PSEUDO-CONSTRUCTOR's output,
// not just its methods'. A tag-based CFC body is mostly inter-tag whitespace,
// and RustCFML used to emit all of it on every instantiation: the attribute was
// parsed into the component's __metadata and then ignored at execution, because
// the pseudo-constructor frame (__cfc_body__, cloned from the file's __main__)
// never carried an `output` entry for finalize() to turn into
// output_suppressed. A tag-based Application.cfc therefore prepended a few
// bytes of whitespace to EVERY response -- invisible in HTML, fatal for
// text/plain, CSV, JSON and anything whose leading bytes are checked (issue
// #373, where it was first mistaken for an enableCFOutputOnly leak).
// Verified against Lucee 7.1.0+204: output="false" emits nothing at all,
// output="true"/no attribute leaks the whitespace on both engines.
suiteBegin("Component output attribute");

savecontent variable="silentBody" {
    o = createObject("component", "core.component_output.SilentTagCfc");
}
assert("a tag CFC with output=false emits nothing while constructing", len(silentBody), 0);
assert("...and is still constructed properly", o.a & "|" & o.b, "1|2");
assert("...and its methods still work", o.m(), "M");

savecontent variable="silentCall" {
    ignored = o.m();
}
assert("a method with output=false emits nothing either", len(silentCall), 0);

savecontent variable="loudBody" {
    p = createObject("component", "core.component_output.LoudTagCfc");
}
// Not asserting the exact bytes: both engines leak here, but they disagree on
// whether text AFTER </cfcomponent> counts. Whitespace-only and non-empty is
// the portable statement, and it is what proves the suppression above is the
// attribute rather than a blanket trim of every CFC body.
assertTrue("a tag CFC with output=true still leaks its inter-tag whitespace", len(loudBody) GT 0);
assert("...and what leaks is only whitespace", len(trim(loudBody)), 0);
assert("...and it too is constructed properly", p.a & "|" & p.b, "1|2");

suiteEnd();
</cfscript>
