<cfscript>
suiteBegin( "unscoped variable cascade (bare name -> cgi/url/form/cookie)" );

// CFML resolves a bare (unscoped) name through the scope cascade:
// local -> arguments -> variables -> cgi -> url -> form -> cookie. RustCFML
// previously stopped at variables/globals, so a bare name that lived only in
// the url/form scope threw "Variable 'x' is undefined". This is exactly how
// Preside's admin DataManager.viewRecord reads a bare `id` (recordId=id) and
// resolves it to url.id — the crash that motivated this suite.

// --- bare name falls through to the url scope ---
url.myUrlOnlyKey = "url-value";
assert( "bare name resolves via url scope", myUrlOnlyKey, "url-value" );

// --- bare name falls through to the form scope ---
form.myFormOnlyKey = "form-value";
assert( "bare name resolves via form scope", myFormOnlyKey, "form-value" );

// --- variables scope OUTRANKS url (correct cascade order) ---
url.shadowed = "from-url";
variables.shadowed = "from-variables";
assert( "variables scope wins over url in the cascade", shadowed, "from-variables" );

// --- a genuinely undefined name still errors (cascade must not mask it) ---
// (critically: adding cgi to the cascade must NOT make every unknown bare
//  name resolve to "" via the cgi empty-default magic scope)
threw = false;
try {
    tmp = aNameThatExistsInNoScopeWhatsoever_9f3;
} catch ( any e ) {
    threw = true;
}
assertTrue( "undefined bare name still throws (cgi empty-default must not mask)", threw );

// --- bare read inside a function still cascades past local/arguments ---
url.fnCascadeKey = "fn-url-value";
function readsBareUrlKey() {
    return fnCascadeKey;  // not a local, not an arg -> cascade to url
}
assert( "bare name cascades to url from inside a function", readsBareUrlKey(), "fn-url-value" );

suiteEnd();
</cfscript>
