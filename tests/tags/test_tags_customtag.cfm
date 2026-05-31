<cfscript>suiteBegin("Tags: Custom Tags");</cfscript>

<!--- Test 1: Self-closing cf_ prefix tag --->
<cfsavecontent variable="ct1"><cf_greeting name="World"></cfsavecontent>
<cfscript>assertTrue("cf_ self-closing greeting", findNoCase("Hello, World!", ct1) GT 0);</cfscript>

<!--- Test 2: cfmodule template= form --->
<cfsavecontent variable="ct2"><cfmodule template="customtags/greeting.cfm" name="Test"></cfsavecontent>
<cfscript>assertTrue("cfmodule template greeting", findNoCase("Hello, Test!", ct2) GT 0);</cfscript>

<!--- Test 3: caller write-back with cf_ prefix --->
<cf_setter value="hello from tag">
<cfscript>assert("cf_ caller writeback", result, "hello from tag");</cfscript>

<!--- Test 4: caller write-back with cfmodule --->
<cfmodule template="customtags/setter.cfm" value="cfmodule setter">
<cfscript>assert("cfmodule caller writeback", result, "cfmodule setter");</cfscript>

<!--- Test 5: Body tag with cf_ prefix --->
<cfsavecontent variable="ct5"><cf_wrapper>inner</cf_wrapper></cfsavecontent>
<cfscript>assertTrue("cf_ body tag has div", findNoCase("<div", ct5) GT 0);</cfscript>
<cfscript>assertTrue("cf_ body tag has content", findNoCase("inner", ct5) GT 0);</cfscript>

<!--- Test 6: Body tag with cfmodule --->
<cfsavecontent variable="ct6"><cfmodule template="customtags/wrapper.cfm">module body</cfmodule></cfsavecontent>
<cfscript>assertTrue("cfmodule body tag has div", findNoCase("<div", ct6) GT 0);</cfscript>
<cfscript>assertTrue("cfmodule body tag has content", findNoCase("module body", ct6) GT 0);</cfscript>

<!--- Test 7: Missing custom tag throws error --->
<cfset errorThrown = false>
<cftry>
    <cf_nonexistent_tag_xyz>
    <cfcatch type="any">
        <cfset errorThrown = true>
    </cfcatch>
</cftry>
<cfscript>assertTrue("missing custom tag throws error", errorThrown);</cfscript>

<!--- Test 8-11: quoted attribute values with operator-like chars must be treated as strings --->
<cfset echoed = "">
<cf_echo_attr value="caller-ok">
<cfscript>assert("quoted attr with hyphen", echoed, "caller-ok");</cfscript>

<cfset echoed = "">
<cf_echo_attr value="a.b.c">
<cfscript>assert("quoted attr with dots", echoed, "a.b.c");</cfscript>

<cfset echoed = "">
<cf_echo_attr value="one+two">
<cfscript>assert("quoted attr with plus", echoed, "one+two");</cfscript>

<cfset echoed = "">
<cf_echo_attr value="path/to/thing">
<cfscript>assert("quoted attr with slashes", echoed, "path/to/thing");</cfscript>

<!--- Test 12: hash interpolation inside quoted attribute --->
<cfset suffix = "world">
<cfset echoed = "">
<cf_echo_attr value="hello-#suffix#">
<cfscript>assert("quoted attr with hash interpolation", echoed, "hello-world");</cfscript>

<!--- Test 13: caller write-back to a nested struct --->
<cfset user = { name: "old", age: 99 }>
<cf_nested_setter name="new">
<cfscript>
    assert("caller.user.name nested mutation", user.name, "new");
    assert("caller.user.age sibling preserved", user.age, 99);
</cfscript>

<!--- Test 14: cfmodule with hyphenated attribute value --->
<cfset echoed = "">
<cfmodule template="customtags/echo_attr.cfm" value="mod-hyphen-ok">
<cfscript>assert("cfmodule attr with hyphen", echoed, "mod-hyphen-ok");</cfscript>

<!--- Test 15: cfmodule template supports hash interpolation --->
<cfset echoed = "">
<cfset modulePath = "customtags/echo_attr.cfm">
<cfset dynamicModuleError = "">
<cftry>
    <cfmodule template="#modulePath#" value="dynamic-module">
    <cfcatch type="any">
        <cfset dynamicModuleError = cfcatch.message>
    </cfcatch>
</cftry>
<cfscript>
    assert("cfmodule dynamic template error", dynamicModuleError, "");
    assert("cfmodule dynamic template", echoed, "dynamic-module");
</cfscript>

<cfscript>suiteEnd();</cfscript>
