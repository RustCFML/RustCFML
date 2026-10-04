<cfscript>
suiteBegin("template text compiles to a direct buffer write with __writeText's rules");
// Expected values verified on Lucee 7.1.0.204.
o = new oop.WriteTextOutFalse();
savecontent variable="c1" { o.f(); }
savecontent variable="c2" { o.g(); }
savecontent variable="c3" { o.h(); }
assert("output=false suppresses literal text, cfoutput and writeOutput", trim(reReplace(c1, "\s+", " ", "all")), "");
assert("output=true emits literal text", trim(reReplace(c2, "\s+", " ", "all")), "LITERAL_G");
assert("default output emits literal text", trim(reReplace(c3, "\s+", " ", "all")), "LITERAL_H");
assert("savecontent inside an output=false function still captures text and whitespace", new oop.WriteTextSavecontent().s(), "[  A    B    C  ]");
</cfscript>
<cfsavecontent variable="only"><cfsetting enablecfoutputonly="true">HIDDEN<cfoutput>SHOWN</cfoutput><cfsetting enablecfoutputonly="false"></cfsavecontent>
<cfscript>
assert("enableCFOutputOnly suppresses bare text but not cfoutput", trim(only), "SHOWN");
suiteEnd();
</cfscript>
