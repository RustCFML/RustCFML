<cfscript>
suiteBegin("Scope writeback + nested param (surfaced booting Masa CMS)");

// --- 1. Nested dynamic param auto-vivifies intermediate structs ------------
// `param name="request.a.#dyn#.leaf"` (dynamic name, deep path) walks/creates
// the intermediate structs and sets only the missing leaf — Lucee/ACF param
// semantics. Mura/Masa bean.cfc params dozens of
// `application.objectMappings.<entity>.<key>` during entity registration.
// (Masa creates `application.objectMappings` first, then params the per-entity
// nested keys — mirror that: the parent container exists before the deep params.)
request.mm = {};
en = "approvalChain";
param name="request.mm.#en#" default={};
param name="request.mm.#en#.synthedFunctions" default={};
param name="request.mm.#en#.hasMany" default=[];
assert("nested dynamic param created the entity struct",
    structKeyExists(request.mm, "approvalChain"), true);
assert("nested dynamic param created a nested leaf key",
    structKeyExists(request.mm.approvalChain, "synthedFunctions"), true);
// A pre-existing intermediate struct is REUSED, not clobbered.
request.mm.approvalChain.marker = "kept";
param name="request.mm.#en#.another" default="x";
assert("nested param reuses existing intermediate struct (no clobber)",
    request.mm.approvalChain.marker, "kept");
</cfscript>

<!--- 2. Case-insensitive result-name writeback into a function-local var.
     A `cfquery`/`cfdbinfo name="RsX"` delivery whose casing differs from a
     `var rsx` declaration must update the SAME (case-insensitive) local, not
     fork a second key. Mura/Masa dbUtility.version() declares `var rscheck`
     then `cfdbinfo name="rsCheck"`. Exercised DB-free via a QoQ cfquery. --->
<cffunction name="deliverMixedCase" output="false">
    <cfset var rsx = "">
    <cfset var src = queryNew("id,nm", "integer,varchar", [[1, "a"], [2, "b"]])>
    <cfquery name="RsX" dbtype="query">
        SELECT nm FROM src WHERE id = 2
    </cfquery>
    <cfreturn isQuery(rsx) ? "delivered:" & rsx.nm : "MISSING">
</cffunction>

<cfscript>
assert("mixed-case query name delivered into the case-insensitive local var",
    deliverMixedCase(), "delivered:b");

suiteEnd();
</cfscript>
