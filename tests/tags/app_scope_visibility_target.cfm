<cfsilent>
<cfscript>
// Target for test_application_scope_concurrency.cfm. Three modes, selected by
// url.mode, all operating on the LIVE application scope of the calling request's
// application (same webroot ⇒ same Application.cfc ⇒ same application).
//
//   read   — echo application[url.key], or "(MISSING)" if absent. Used to prove a
//            write made by a still-running request is visible to another request.
//   bump   — increment application[url.key] under an exclusive lock. Used to prove
//            concurrent read-modify-writes do not lose updates (a snapshot-per-
//            request scope silently drops all but the last writer's).
//   hold   — write application[url.key], then sleep url.ms BEFORE returning, so the
//            caller can observe the value mid-request.
mode = url.mode ?: "read";
key  = url.key  ?: "appvis";

switch ( mode ) {
    case "read":
        result = structKeyExists( application, key ) ? application[ key ] : "(MISSING)";
        break;

    case "bump":
        lock name="appvis_bump_#key#" type="exclusive" timeout=20 {
            application[ key ] = ( structKeyExists( application, key ) ? application[ key ] : 0 ) + 1;
            result = application[ key ];
        }
        break;

    case "hold":
        application[ key ] = url.value ?: "held";
        sleep( val( url.ms ?: 1500 ) );
        result = application[ key ];
        break;

    // delete — remove the key from the shared scope. With a live scope this is
    // visible to the still-running caller; with a per-request snapshot it is not.
    // Covers the `StructDelete( application, … )` / `StructClear( application )`
    // side of the semantics, which became genuinely destructive to in-flight
    // requests once the scope is shared.
    case "delete":
        structDelete( application, key );
        result = structKeyExists( application, key ) ? "still-there" : "deleted";
        break;

    default:
        result = "(bad mode)";
}
</cfscript>
</cfsilent><cfoutput>#result#</cfoutput>
