<cfscript>
suiteBegin("Whitespace Handling");
</cfscript>

<!--- Test 1: Whitespace between tags is preserved --->
<cfsavecontent variable="ws1"><cfoutput>#1#</cfoutput> <cfoutput>#2#</cfoutput></cfsavecontent>
<cfscript>
assert("whitespace between cfoutput tags", ws1, "1 2");
</cfscript>

<!--- Test 2: Newline between closing/opening tags preserved --->
<cfsavecontent variable="ws2"><cfif true>A</cfif>
<cfif true>B</cfif></cfsavecontent>
<cfscript>
assert("newline between cfif blocks", ws2, "A" & chr(10) & "B");
</cfscript>

<!--- Test 3: cfprocessingdirective suppressWhiteSpace --->
<cfsavecontent variable="ws3"><cfprocessingdirective suppresswhitespace="true">
    <cfoutput>hello     world</cfoutput>
</cfprocessingdirective></cfsavecontent>
<cfscript>
assert("suppressWhiteSpace collapses runs", trim(ws3), "hello world");
</cfscript>

<!--- Test 4: suppressWhiteSpace preserves pre tags --->
<cfsavecontent variable="ws4"><cfprocessingdirective suppresswhitespace="true"><cfoutput><pre>  a   b  </pre></cfoutput></cfprocessingdirective></cfsavecontent>
<cfscript>
assertTrue("suppressWhiteSpace preserves pre tag", ws4 contains "<pre>" && ws4 contains "a" && ws4 contains "b");
</cfscript>

<!--- Test 5: cfsetting enableCFOutputOnly --->
<cfsavecontent variable="ws5"><cfsetting enablecfoutputonly="true">HIDDEN<cfoutput>VISIBLE</cfoutput>HIDDEN<cfsetting enablecfoutputonly="false"></cfsavecontent>
<cfscript>
assert("enableCFOutputOnly suppresses text outside cfoutput", ws5, "VISIBLE");
</cfscript>

<!--- Test 6: enableCFOutputOnly counter semantics --->
<cfsavecontent variable="ws6"><cfsetting enablecfoutputonly="true"><cfsetting enablecfoutputonly="true">HID<cfoutput>VIS</cfoutput><cfsetting enablecfoutputonly="false">STILLHID<cfsetting enablecfoutputonly="false">NOW_VISIBLE</cfsavecontent>
<cfscript>
assert("enableCFOutputOnly counter", ws6, "VISNOW_VISIBLE");
</cfscript>

<!--- Test 7: enableCFOutputOnly reset --->
<cfsavecontent variable="ws7"><cfsetting enablecfoutputonly="true"><cfsetting enablecfoutputonly="true">HID<cfsetting enablecfoutputonly="reset">VISIBLE</cfsavecontent>
<cfscript>
assert("enableCFOutputOnly reset", ws7, "VISIBLE");
</cfscript>

<!--- Test 8: cfprocessingdirective suppressWhiteSpace=false is passthrough --->
<cfsavecontent variable="ws8"><cfprocessingdirective suppresswhitespace="false"><cfoutput>  hello  </cfoutput></cfprocessingdirective></cfsavecontent>
<cfscript>
assert("suppressWhiteSpace=false passthrough", ws8, "  hello  ");
</cfscript>

<!--- Test 9: cfprocessingdirective NESTED INSIDE cfoutput keeps interpolation.
      Masa's admin layouts wrap the whole body in
      `<cfoutput><cfprocessingdirective suppresswhitespace="true">…#expr#…`.
      The directive body must inherit the enclosing cfoutput context so `#x#`
      still evaluates (previously it rendered the literal text `#x#`). --->
<cfset wx = "WORLD">
<cfsavecontent variable="ws9"><cfoutput><cfprocessingdirective suppresswhitespace="true">a[#wx#]b</cfprocessingdirective></cfoutput></cfsavecontent>
<cfscript>
assert("cfprocessingdirective inside cfoutput interpolates (suppress=true)", ws9, "a[WORLD]b");
</cfscript>
<cfsavecontent variable="ws10"><cfoutput><cfprocessingdirective suppresswhitespace="false">a[#wx#]b</cfprocessingdirective></cfoutput></cfsavecontent>
<cfscript>
assert("cfprocessingdirective inside cfoutput interpolates (suppress=false)", ws10, "a[WORLD]b");
</cfscript>

<cfscript>
suiteEnd();
</cfscript>
