<cfscript>
suiteBegin("SessionCommit");

session.sessionCommitProbe = "before";
sessionCommitError = "";

try {
    SessionCommit();
} catch (any e) {
    sessionCommitError = e.message;
}

assert("SessionCommit is callable", sessionCommitError, "");
assert("SessionCommit leaves session data intact", session.sessionCommitProbe, "before");

suiteEnd();
</cfscript>
