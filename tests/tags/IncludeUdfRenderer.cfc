component {
    // Fixture for test_include_udf_no_caller_locals.cfm: a method includes a
    // template (function-scoped include) which calls a helper UDF that was
    // mixed into this component's variables scope.
    variables.controller = "CTRL";
    include "include_udf_helpers.cfm";
    public string function renderView() {
        var viewpath = "VP";
        var out = "";
        savecontent variable="out" { include "include_udf_view.cfm"; }
        return out;
    }
}
