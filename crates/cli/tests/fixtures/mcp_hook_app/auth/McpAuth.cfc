component {

    // Resolves the credentials the static authToken list does not know.
    function authenticate( required string token, struct headers = {}, string transport = "" ) {
        if ( arguments.token == "throws" ) {
            throw( type = "HookFailure", message = "secret internal detail" );
        }
        if ( arguments.token == "reader" ) {
            return { scopes = "read", principalId = "r1" };
        }
        if ( arguments.token == "disabled" ) {
            return { authenticated = false };
        }
        if ( left( arguments.token, 5 ) == "user-" ) {
            return {
                  roles       = [ "member" ]
                , principalId = mid( arguments.token, 6, 100 )
                , tenantId    = "t1"
                , via         = arguments.transport
                , sawHeader   = structKeyExists( arguments.headers, "x-probe" ) ? arguments.headers[ "x-probe" ] : ""
            };
        }
        return;
    }
}
