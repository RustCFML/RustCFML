component {

    function init() {
        variables.sessionStorage       = "SS-OBJ";
        variables.tokenExpiryInSeconds = 5;
        return this;
    }

    // Mirrors Preside's _getSvc(): each param defaults to a same-named member
    // seeded in the pseudo-constructor above. Pre-#240 these resolved to null.
    private function _describe(
          sessionStorage           = sessionStorage
        , tokenExpiryInSeconds     = tokenExpiryInSeconds
        , authenticatedSessionOnly = false
    ) {
        return ( arguments.sessionStorage ?: "<null>" )
             & "|" & ( arguments.tokenExpiryInSeconds ?: "<null>" )
             & "|" & arguments.authenticatedSessionOnly;
    }

    public function run() {
        return _describe( argumentCollection = arguments );
    }

}
