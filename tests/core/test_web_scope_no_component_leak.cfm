<cfscript>
suiteBegin("Web request scopes never persist on a component (Masa redirect loop)");

// Root cause of the Masa CMS front-controller infinite redirect loop: an
// application-scoped singleton (contentServer) wrote `url.path` inside a method;
// the scoped write vivified a `url` key onto the component's `variables` scope
// (and reads then resolved that stale shadow before the live globals), so across
// requests url.path accumulated (`/index.cfm/default/default/default/...`).
// url/form/cgi/cookie are request-global: writes go to the live request scope,
// reads resolve to it, and NOTHING lands on the component variables scope.

obj = createObject("component", "core.web_scope_leak_helper");

// 1. A scoped write from inside the method reaches the LIVE page-level url scope.
obj.writeUrl("marker", "hello");
assert("scoped url write inside CFC method reaches page url scope",
    structKeyExists(url, "marker") ? url.marker : "(missing)", "hello");

// 2. The write did NOT vivify a `url` key onto the component variables scope.
assert("scoped url write did NOT leak onto component variables scope",
    obj.urlKeyLeakedIntoVariables(), false);

// 3. A bare read of `url` inside the method sees the live page scope value,
//    not a component-local shadow.
url.fromPage = "pageValue";
assert("bare url read inside CFC method resolves to the live request scope",
    obj.readUrl("fromPage"), "pageValue");

// 4. The method observes writes made to the page url scope AFTER construction
//    (i.e. it is not bound to a stale snapshot).
url.late = "seenLate";
assert("CFC method sees url keys added after the component was built",
    obj.readUrl("late"), "seenLate");

// 5. Dotted scoped write (`url.path = x`) — the exact Masa idiom — reaches the
//    live scope and does not leak onto variables.
obj.writeUrlDotted("/some/path/");
assert("dotted url.path write inside CFC method reaches page url scope",
    structKeyExists(url, "path") ? url.path : "(missing)", "/some/path/");
assert("dotted url.path write did NOT leak onto component variables scope",
    obj.urlKeyLeakedIntoVariables(), false);

suiteEnd();
</cfscript>
