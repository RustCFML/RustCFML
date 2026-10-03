component mcp="app" version="1.0.0" description="Fixture for the MCP authenticate hook" {

    function whoami() tool="whoami" secured description="The caller's identity" {
        var who = mcp().identity();
        return {
              principalId   = who.principalId ?: ""
            , tenantId      = who.tenantId ?: ""
            , roles         = who.roles
            , via           = who.via ?: ""
            , sawHeader     = who.sawHeader ?: ""
            , transport     = mcp().transport()
            , authorization = mcp().authorization()
        };
    }

    function getStatus() tool="get_status" description="Readable with the read scope" {
        return "ok";
    }

    function members() tool="members_only" secured="member" description="Needs the member role" {
        return "members ok";
    }

    function setStatus() tool="set_status" description="Not in the read scope" {
        return "set";
    }
}
