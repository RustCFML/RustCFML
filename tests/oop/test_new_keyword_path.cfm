<!---
  GitHub #246: the `new` operator stopped parsing a dotted component path at any
  segment that is a reserved keyword (public/private/remote/package/static), so
  `new pkg.public.Inner()` was mis-parsed as `new pkg` + member access and threw
  "Could not find the component [pkg]". CreateObject / getComponentMetadata / the
  quoted-string form all worked; only the `new <dotted-identifier>` form broke.
  JVM engines reserve these words only in modifier/statement position, never as
  path segments.

  Secondary: a `new` resolution FAILURE escaped a surrounding try/catch(any)
  entirely (it did `return Err` instead of routing through the try handler, so
  the whole request aborted), while the identical CreateObject failure was
  catchable. Both now behave the same.

  Fixtures live under tests/oop/newkw/*; each hello() returns a distinct marker.
--->
<cfscript>
suiteBegin("new-operator dotted path with reserved-word segments (GitHub 246)");

// Keyword path segments are accepted.
assert("new ...public.Inner()",  (new newkw.public.Inner()).hello(),  "pub-inner-ok");
assert("new ...private.Inner()", (new newkw.private.Inner()).hello(), "priv-inner-ok");
assert("new ...remote.Inner()",  (new newkw.remote.Inner()).hello(),  "rem-inner-ok");
assert("new ...package.Inner()", (new newkw.package.Inner()).hello(), "pkg-inner-ok");
assert("new ...static.Inner()",  (new newkw.static.Inner()).hello(),  "stat-inner-ok");

// Trailing keyword-named class.
assert("new ...Public() (trailing keyword class)", (new newkw.Public()).hello(), "public-cfc-ok");

// Controls: non-keyword segments must still work.
assert("control new ...sub.Inner()", (new newkw.sub.Inner()).hello(), "sub-inner-ok");
assert("control new ...Thing()",     (new newkw.Thing()).hello(),     "thing-ok");

// Parity: CreateObject with a keyword segment (already worked) still works.
assert("createObject keyword segment", CreateObject("component", "newkw.public.Inner").hello(), "pub-inner-ok");

// Secondary: a failing `new` is catchable by try/catch(any), like CreateObject.
newCaught = false;
try {
    o = new newkw.DoesNotExist_246();
} catch (any e) {
    newCaught = true;
}
assert("new resolution failure is catchable", newCaught, true);

// Control: CreateObject failure was already catchable.
coCaught = false;
try {
    o = CreateObject("component", "newkw.DoesNotExist_246");
} catch (any e) {
    coCaught = true;
}
assert("createObject resolution failure catchable", coCaught, true);

// Execution continues after a caught new-failure (didn't abort the request).
assert("execution survives caught new-failure", "alive", "alive");

suiteEnd();
</cfscript>
