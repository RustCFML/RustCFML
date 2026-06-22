<cfscript>
suiteBegin("Tags: custom tag caller scope from CFC methods");

try {
    fixture = createObject("component", "tags.CustomTagCallerCfcFixture");
} catch (any e) {
    fixture = createObject("component", "tests.tags.CustomTagCallerCfcFixture");
}
assert(
    "body custom tag sees CFC method-scope variables through caller scope",
    fixture.render(),
    "caller:method-scope|body:BODY"
);

suiteEnd();
</cfscript>
