component mcp="secure" version="1.0.0" description="Fixture for the MCP auth tests" {

    function open() tool="open_tool" description="Anyone may call this" {
        return "open";
    }

    function adminOnly() tool="admin_tool" secured="admin" description="Needs the admin role" {
        return "admin ok";
    }

    function anyLogin() tool="login_tool" secured description="Needs any authenticated caller" {
        return "login ok";
    }

    function dangerous() tool="delete_everything" description="Should be filtered out" {
        return "should never run";
    }
}
