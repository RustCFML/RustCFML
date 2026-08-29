<cfscript>
suiteBegin("cfapplication tag is implemented (GH 374)");

// <cfapplication> is the pre-Application.cfc way to declare an application. It
// used to fall through to the generic "Tag <X> is not implemented" throw, which
// 500'd the request — so on a page that used it, NOTHING after the tag ran.
// Deliberately declared with THIS suite's own application name and settings, so
// the tag proves it executes without rebinding the application scope out from
// under the tests that follow.
before = getApplicationMetadata().name;
</cfscript>

<cfapplication name="#before#" sessionmanagement="true" clientmanagement="false">

<cfscript>
// The only assertion the reported bug needed: execution continues past the tag.
assert("execution continues past cfapplication", true, true);

meta = getApplicationMetadata();
assert("the application name is unchanged", meta.name, before);
assert("sessionManagement is reported", meta.sessionManagement, true);
assert("clientManagement is reported", meta.clientManagement, false);

// The declared name is the application scope actually in force.
assert("application scope still resolves",
	isStruct(application), true);
assert("applicationName matches the declaration",
	application.applicationName, before);

// The script form lowers to the same intercept and must behave identically.
application name="#before#" sessionmanagement="true";
assert("the script form also runs", getApplicationMetadata().name, before);

suiteEnd();
</cfscript>
