component {

    /*
     * Documentation example of the tag form — this is a comment, NOT markup.
     * The literal tags below must be ignored by the lexer; the component must
     * compile to a normal CFC. (Closed tags only.)
     *
     *   <cfset example = 1>
     *   <cfoutput>#example#</cfoutput>
     */
    public string function ping() {
        return "pong";
    }

}
