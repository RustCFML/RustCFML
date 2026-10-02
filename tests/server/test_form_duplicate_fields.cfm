<cfscript>
suiteBegin("Server: duplicate form/url keys merge");

// ============================================================
// Background
// ============================================================
// Duplicate field names in a request must merge into a comma-separated list
// in document order (Lucee/ACF semantics), with EMPTY values dropped from the
// merge: `dup=first&dup=&dup=third` -> "first,third". Last-one-wins silently
// loses every earlier value -- real browsers post duplicate fields routinely
// (hidden fallback inputs, multi-select checkboxes), so the loss is invisible
// until production. Verified live against Lucee 7.0.4.
//
// Both the form and url scopes share the same parse path, so both are
// asserted. The POST body is sent raw (type="body") so the test exercises the
// server's request parsing, not cfhttp's own duplicate-formfield encoding.
//
// Like the other HTTP round-trip tests this needs a live server: it discovers
// the port from cgi.server_port and skips when run from the CLI.
// ============================================================

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";
skip = serverPort == "" || serverPort == "0";

if (skip) {
    assertTrue("duplicate form/url keys skipped (no cgi.server_port)", true);
} else {
    baseUrl = "http://127.0.0.1:" & serverPort;
    target = "/tests/server/form_duplicate_fields_target.cfm";
    postError = "";
    getError = "";
}
</cfscript>

<cfif NOT skip>
    <!--- form scope: raw urlencoded body with duplicate keys + an empty middle value --->
    <cftry>
        <cfhttp url="#baseUrl##target#" method="POST" result="postResult">
            <cfhttpparam type="header" name="Content-Type" value="application/x-www-form-urlencoded" />
            <cfhttpparam type="body" value="dup=first&dup=&dup=third" />
        </cfhttp>
        <cfcatch type="any"><cfset postError = cfcatch.message></cfcatch>
    </cftry>

    <!--- form scope, multipart: what a browser posts for a checkbox group in an
          enctype="multipart/form-data" form (every Preside admin form) --->
    <cfset multipartError = "">
    <cftry>
        <cfhttp url="#baseUrl##target#" method="POST" result="multipartResult" multipart="true">
            <cfhttpparam type="formfield" name="dup" value="first" />
            <cfhttpparam type="formfield" name="dup" value="" />
            <cfhttpparam type="formfield" name="dup" value="third" />
        </cfhttp>
        <cfcatch type="any"><cfset multipartError = cfcatch.message></cfcatch>
    </cftry>

    <!--- url scope: same duplicate keys on the query string --->
    <cftry>
        <cfhttp url="#baseUrl##target#?dup=first&dup=&dup=third" method="GET" result="getResult">
        </cfhttp>
        <cfcatch type="any"><cfset getError = cfcatch.message></cfcatch>
    </cftry>

    <cfscript>
        assertTrue("duplicate-key POST round-trip completed", postError == "");
        if (postError == "") {
            assert("form scope: duplicate keys comma-join, empties dropped",
                listFirst(trim(postResult.fileContent), ";"),
                "form=[first,third]");
            assert("form.fieldnames (urlencoded): each field once, upper-cased",
                listGetAt(trim(postResult.fileContent), 3, ";"),
                "fieldnames=[DUP]");
        }

        assertTrue("duplicate-key multipart POST round-trip completed", multipartError == "");
        if (multipartError == "") {
            assert("form scope (multipart): duplicate keys comma-join, empties dropped",
                listFirst(trim(multipartResult.fileContent), ";"),
                "form=[first,third]");
            assert("form.fieldnames (multipart): each field once, upper-cased",
                listGetAt(trim(multipartResult.fileContent), 3, ";"),
                "fieldnames=[DUP]");
        }

        assertTrue("duplicate-key GET round-trip completed", getError == "");
        if (getError == "") {
            assert("url scope: duplicate keys comma-join, empties dropped",
                listGetAt(trim(getResult.fileContent), 2, ";"),
                "url=[first,third]");
            assert("GET: no form fields, so no form.fieldnames",
                listGetAt(trim(getResult.fileContent), 3, ";"),
                "fieldnames=[(missing)]");
        }
    </cfscript>
</cfif>

<cfscript>
suiteEnd();
</cfscript>
