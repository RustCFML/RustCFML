<cfscript>
suiteBegin("Custom tag body inherits the caller's cfoutput context (GH ##339)");
</cfscript>
<!---
    GH #339: a custom tag's BODY belongs to the calling template, so an
    enclosing <cfoutput> around the invocation must interpolate #expr# inside
    that body. Both invocation syntaxes (<cfmodule> and a <cfimport prefix>
    tag) behave the same way; the discriminator is only where the cfoutput
    sits. Expected values measured on Lucee 7.1.0.204 (box server, 2026-08-23).

    The last two cases are the CONTROLS that keep the fix honest: with NO
    enclosing cfoutput the body must still emit #plain# literally. Do not
    "simplify" them away — without them a fix that unconditionally interpolates
    tag bodies would pass every other leg here.
--->
<cfimport prefix="ct339" taglib="customtags">
<cfset plain = 42>

<cfsavecontent variable="ct339_a"><cfoutput>A[#plain#]</cfoutput></cfsavecontent>
<cfsavecontent variable="ct339_d"><ct339:cfoutput_body_probe>D<cfoutput>[#plain#]</cfoutput></ct339:cfoutput_body_probe></cfsavecontent>
<cfsavecontent variable="ct339_e"><cfmodule template="customtags/cfoutput_body_probe.cfm">E<cfoutput>[#plain#]</cfoutput></cfmodule></cfsavecontent>
<cfsavecontent variable="ct339_f"><cfoutput><cfmodule template="customtags/cfoutput_body_probe.cfm">F[#plain#]</cfmodule></cfoutput></cfsavecontent>
<cfsavecontent variable="ct339_g"><cfoutput><ct339:cfoutput_body_probe>G[#plain#]</ct339:cfoutput_body_probe></cfoutput></cfsavecontent>
<cfsavecontent variable="ct339_h"><cfmodule template="customtags/cfoutput_body_probe.cfm">H[#plain#]</cfmodule></cfsavecontent>
<cfsavecontent variable="ct339_i"><ct339:cfoutput_body_probe>I[#plain#]</ct339:cfoutput_body_probe></cfsavecontent>

<cfscript>
assert( "A: plain cfoutput on the page interpolates",              trim( ct339_a ), "A[42]" );
assert( "D: cfoutput INSIDE a cfimport tag body interpolates",     trim( ct339_d ), "D[42]" );
assert( "E: cfoutput INSIDE a cfmodule body interpolates",         trim( ct339_e ), "E[42]" );
assert( "F: enclosing cfoutput reaches a cfmodule body",           trim( ct339_f ), "F[42]" );
assert( "G: enclosing cfoutput reaches a cfimport tag body",       trim( ct339_g ), "G[42]" );
assert( "H: CONTROL - no cfoutput anywhere, cfmodule body is literal", trim( ct339_h ), "H[##plain##]" );
assert( "I: CONTROL - no cfoutput anywhere, cfimport body is literal", trim( ct339_i ), "I[##plain##]" );
suiteEnd();
</cfscript>
