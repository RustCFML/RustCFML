component {
    this.ranDuringConstruction = false;
    function run() {
        ```<cfset local.answer = 42><cfreturn local.answer>```
    }
    function neverCall() {
        ```<cfset this.ranDuringConstruction = true><cfthrow message="uncalled method ran">```
    }
}
