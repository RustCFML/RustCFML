<cfscript>suiteBegin("Tags: namespaced <cfargument> attribute (colddoc:generic)");</cfscript>

<!--- A namespaced tag attribute (e.g. `colddoc:generic="..."`) on <cfargument>
      must parse — it previously failed with "Expected RParen, found Colon"
      while the same attribute on <cffunction> parsed. Lucee accepts it on both,
      and ColdBox 5.4.0 relies on it. GitHub #226. --->

<cffunction name="inlineFn">
    <cfargument name="a" colddoc:generic="x.Y">
    <cfreturn 1>
</cffunction>

<cfscript>
    assert("namespaced attr on <cfargument> parses and calls", inlineFn(1), 1);

    // The annotation survives into getMetadata() like Lucee, alongside the
    // structural attributes and the ordinary `inject` annotation.
    obj = createObject("component", "tags.NamespacedArgAttr");
    params = getMetadata(obj).functions[1].parameters;
    assert("param name preserved", params[1].name, "input");
    assert("namespaced annotation surfaced in metadata", params[1]["colddoc:generic"], "my.package.Widget");
    assert("ordinary annotation still surfaced", params[1].inject, "coldbox");
    assert("calling the fixture method works", obj.process("hi"), 42);
</cfscript>

<cfscript>suiteEnd();</cfscript>
