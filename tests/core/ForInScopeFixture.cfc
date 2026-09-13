component {
    // Fixture for test_forin_loop_variable_scope.cfm.
    public string function classic() { for (i in [1]) {} return "vars=" & structKeyExists(variables,"i") & " local=" & structKeyExists(local,"i"); }
    public string function modern() localmode="modern" { for (i in [1]) {} return "local=" & structKeyExists(local,"i"); }
    public string function viaClosure() { var f = function() { for (c in [1]) {} return "vars=" & structKeyExists(variables,"c") & " local=" & structKeyExists(local,"c"); }; return f(); }
    public string function varForm() { for (var v in [1]) {} return "vars=" & structKeyExists(variables,"v") & " local=" & structKeyExists(local,"v"); }
    public string function afterLoop() { for (x in [7]) {} return isDefined("x") ? x : "undef"; }
}
