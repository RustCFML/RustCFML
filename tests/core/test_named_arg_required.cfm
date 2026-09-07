<!---
  GitHub #413: a named-argument label that matches no declared parameter was
  silently dropped, and a `required` parameter thereby left unfilled was passed
  as EMPTY with no error. Lucee throws.

    installPackage( packageID="coldbox" )   // declared param is `id`
    -> RustCFML: runs with arguments.id empty
    -> Lucee:    "The parameter [id] to function [installPackage] is required
                  but was not passed in."

  Cause: the required-parameter check tested `args.get(i).is_none()`, but the
  named-argument rebinder pads omitted slots with Null so later named args land
  at the right index. A padded Null read as "supplied", so the check was skipped
  for ANY call that used a named argument. That is why the plain positional case
  below has always been correct and only the named case failed.

  The mismatched LABEL is not itself an error on either engine — it lands in the
  arguments scope as an extra key. The missing required parameter is.
--->
<cfscript>
suiteBegin("Required parameters are enforced on the named-argument path (GitHub 413)");

function installPackage( required string id, string directory="" ) {
    return "id=[" & arguments.id & "]";
}
function twoRequired( required string a, required string b ) {
    return arguments.a & "/" & arguments.b;
}

function callAndCatch( required any fn, struct args={} ) {
    try { return arguments.fn( argumentCollection=arguments.args ); }
    catch (any e) { return "ERR:" & e.message; }
}

// 1. The reported repro. A mismatched label must not leave `id` empty.
assert("mismatched label raises the missing required param",
       callAndCatch(installPackage, {packageID="coldbox"}),
       "ERR:The parameter [id] to function [installPackage] is required but was not passed in.");

// 2. The positional path was always correct — keep it that way.
assert("no arguments at all still raises",
       callAndCatch(installPackage, {}),
       "ERR:The parameter [id] to function [installPackage] is required but was not passed in.");

// 3. A correct named call still works, and an EXTRA unknown label alongside it
//    is not an error — it is just an extra key in the arguments scope.
assert("correct named call works", installPackage(id="coldbox"), "id=[coldbox]");
assert("unknown label alongside a satisfied required param is fine",
       installPackage(id="coldbox", bogusLabel="x"), "id=[coldbox]");

// 4. The padded-Null bug only showed when SOME named arg was present, so cover
//    the case where an earlier required param is named and a later one is not.
assert("second required param missing, first named",
       callAndCatch(twoRequired, {a="x"}),
       "ERR:The parameter [b] to function [twoRequired] is required but was not passed in.");
assert("first required param missing, second named",
       callAndCatch(twoRequired, {b="y"}),
       "ERR:The parameter [a] to function [twoRequired] is required but was not passed in.");
assert("both named works", twoRequired(a="x", b="y"), "x/y");
assert("both positional works", twoRequired("x", "y"), "x/y");

// 5. `required` WITH a default is still not enforced — the default satisfies it
//    (Lucee/ACF/BoxLang parity; TestBox's TestResult.cfc depends on this).
function requiredWithDefault( required string s="fallback" ) { return arguments.s; }
assert("required-with-default is satisfied by the default", requiredWithDefault(), "fallback");
assert("required-with-default still accepts a named value",
       requiredWithDefault(s="given"), "given");
assert("required-with-default is not tripped by an unknown label",
       requiredWithDefault(other="x"), "fallback");

suiteEnd();
</cfscript>
