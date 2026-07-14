<cfscript>
suiteBegin("Tag preprocessor fixes (surfaced booting Masa CMS)");
</cfscript>

<!--- 1. Nested CFML comments: comments NEST (unlike HTML), so the inner
      `<!--- --->` must not close the outer one. Mura/Masa comment out whole
      cftry/cfcatch blocks that themselves contain `<!--- --->` notes. If the
      outer comment ended early, "STILL-IN-OUTER" would leak as literal text and
      the cfset below would be mis-parsed. --->
<!--- outer <!--- inner ---> STILL-IN-OUTER ---><cfset commentProbe = "clean">
<cfscript>
assert("nested comment fully stripped (no leak into next statement)", commentProbe, "clean");

// --- 2. Empty <cfcase value=""> ---------------------------------------------
// `<cfcase value="">` matches the empty string — a valid single case value.
function classify(v) {
    switch (v) {
        case "": return "empty";
        case "x": return "ex";
        default: return "other";
    }
}
assert("empty-string case value matches", classify(""), "empty");
assert("non-empty case still works", classify("x"), "ex");
</cfscript>

<cffunction name="classifyTag" output="false">
    <cfargument name="v">
    <cfset var r = "">
    <cfswitch expression="#arguments.v#">
        <cfcase value=""><cfset r = "empty"></cfcase>
        <cfcase value="a,b"><cfset r = "ab"></cfcase>
        <cfdefaultcase><cfset r = "other"></cfdefaultcase>
    </cfswitch>
    <cfreturn r>
</cffunction>

<cfscript>
assert("tag cfcase empty value", classifyTag(""), "empty");
assert("tag cfcase comma-list value", classifyTag("b"), "ab");

// --- 3. Nested #...# interpolation inside a string literal ------------------
// A `#...#` interpolation inside a string is expression context: nested quotes
// must not close the string. (strip_hashes handling for tag-mode cfset.)
gv = function(k) { return k; };
</cfscript>
<cfset nestedInterp = '#gv("fname")# #gv("lname")#'>
<cfset deepNested = '#(gv("a") eq "" ? gv("b") : gv("c"))#'>
<cfscript>
assert("nested interpolation with quoted call args", nestedInterp, "fname lname");
assert("deep nested interpolation resolves", deepNested, "c");

// --- 4. cfdump var as a literal string containing interpolation -------------
who = "world";
</cfscript>
<cfsavecontent variable="dumped"><cfdump var="Hello #who#!"></cfsavecontent>
<cfscript>
assert("cfdump literal-string var interpolates", findNoCase("Hello world!", dumped) GT 0, true);
</cfscript>

<!--- 5. Nested #...# interpolation inside a <cfquery> SQL body. The mere fact
      that this file PARSES (the suite loads) proves the fix — a nested
      interpolation like `#right("IX_#x#",30)#` in cfquery SQL previously split
      at the inner `#` and threw a parse error (Mura/Masa dbCreateIndex). The
      function is defined but never called (no datasource needed). --->
<cffunction name="neverCalled_sqlNestedInterp" output="false">
    <cfargument name="tbl"><cfargument name="col">
    <cfquery name="qx" datasource="__none__">
        CREATE INDEX #right("IX_#arguments.tbl#_#arguments.col#", 30)# ON #arguments.tbl# (#arguments.col#)
    </cfquery>
</cffunction>

<cfscript>
assert("cfquery-with-nested-interpolation SQL parsed (function is defined)",
    isCustomFunction(neverCalled_sqlNestedInterp), true);

// --- 6. Engine version reporting (Lucee-faithful) --------------------------
// server.coldfusion.productversion mimics an ACF version so minimum-version
// gates pass (Mura/Masa require ACF 9.0.1+); server.lucee.version carries the
// real RustCFML version. Engine detection keys on `server.lucee` existing.
assert("server.lucee struct exists (engine detected as Lucee-compatible)",
    structKeyExists(server, "lucee"), true);
assertTrue("coldfusion.productversion passes a >=9 major-version gate",
    val(listFirst(server.coldfusion.productversion)) GTE 9);

suiteEnd();
</cfscript>
