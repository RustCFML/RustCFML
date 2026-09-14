component mcp="demo" version="1.2.3" description="Fixture MCP server for the integration tests" {

    /**
     * @text.description The text to echo back
     */
    function echo( required string text, numeric times = 1 )
        tool        = "echo"
        description = "Echo the given text"
        readOnly    = true
    {
        var out = "";
        for ( var i = 1; i <= arguments.times; i++ ) {
            out &= arguments.text;
        }
        return out;
    }

    function stats( string label = "counts" )
        tool        = "stats"
        description = "Return a structured result"
    {
        return { label = arguments.label, total = 42, ok = true };
    }

    function explode() tool="explode" description="Always throws" {
        throw( type = "DemoError", message = "tool blew up" );
    }

    function hidden() tool="hidden" secured="admin" description="Needs the admin role" {
        return "you should not see this";
    }

    function rows() tool="rows" description="Returns a query" {
        var q = queryNew( "id,title", "integer,varchar" );
        queryAddRow( q, { id = 1, title = "first" } );
        queryAddRow( q, { id = 2, title = "second" } );
        return q;
    }

    /**
     * @steps.description How many progress notifications to emit
     */
    function slow( numeric steps = 3 )
        tool        = "slow"
        streaming   = true
        description = "Emits progress notifications, then finishes"
    {
        for ( var i = 1; i <= arguments.steps; i++ ) {
            mcp().progress( i, arguments.steps, "step #i#" );
            mcp().log( "info", "completed step #i#" );
        }
        return "finished #arguments.steps# steps";
    }

    function whereAmI() tool="where_am_i" description="Reports the call context" {
        return { server = mcp().server(), streaming = mcp().streaming() };
    }

    function announce( required string text )
        tool        = "announce"
        description = "Pushes a notification to every session of this server"
    {
        return mcpNotify( method = "notifications/announcement", params = { text = arguments.text } );
    }

    // ---- Resources ----

    function readme()
        resource    = "doc://readme"
        mimeType    = "text/markdown"
        title       = "The readme"
        description = "A literal resource"
    {
        return "## Readme#chr(10)#Body text";
    }

    /**
     * @slug.description Which page to read
     */
    function page( string slug, string uri )
        resource    = "doc://pages/{slug}"
        mimeType    = "text/plain"
        description = "A templated resource"
    {
        return "page=#arguments.slug# uri=#arguments.uri#";
    }

    function blob() resource="doc://blob" mimeType="application/octet-stream" {
        return toBinary( toBase64( "bytes" ) );
    }

    function secretDoc() resource="doc://secret" secured="admin" {
        return "should never be readable";
    }

    // ---- Prompts ----

    /**
     * @language.description The language being reviewed
     */
    function review( string language = "CFML" )
        prompt      = "code_review"
        description = "Ask for a code review"
    {
        return [
              { role = "assistant", content = "I review #arguments.language# code." }
            , { role = "user",      content = "Please review my #arguments.language#." }
        ];
    }

    function oneLiner() prompt="one_liner" description="A single-message prompt" {
        return "Summarise the repository";
    }

    function ask( required string question )
        tool        = "ask"
        streaming   = true
        description = "Asks the client's own model a question"
    {
        var answer = mcp().sample(
              messages = [ { role = "user", content = arguments.question } ]
            , options  = { maxTokens = 100 }
        );
        return "model said: " & answer.content.text;
    }

    function askWithoutStreaming( required string question )
        tool        = "ask_unstreamed"
        description = "Sampling from a non-streaming tool, which cannot work"
    {
        return mcp().sample( [ { role = "user", content = arguments.question } ] );
    }

    function noisy() tool="noisy" description="Writes output, which must never reach stdout" {
        writeOutput( "this must not land on stdout" );
        return "quiet";
    }

    private function helper() tool="helper" description="Private: must never be exposed" {
        return "nope";
    }
}
