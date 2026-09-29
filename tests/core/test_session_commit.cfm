<cfscript>
suiteBegin("SessionCommit");

session.sessionCommitProbe = "before";
sessionCommitError = "";

try {
    SessionCommit();
} catch (any e) {
    sessionCommitError = e.message;
}

// Lucee 7.1.0 defect: with memory sessions SessionCommit() throws a
// ClassCastException (SessionMemory -> IKStorageScopeSupport); 7.0.4 is fine.
// Skip only that exact engine failure.
if ( !( !isRustCFML() && findNoCase( "IKStorageScopeSupport", sessionCommitError ) ) ) {
	assert("SessionCommit is callable", sessionCommitError, "");
}
assert("SessionCommit leaves session data intact", session.sessionCommitProbe, "before");

suiteEnd();
</cfscript>
