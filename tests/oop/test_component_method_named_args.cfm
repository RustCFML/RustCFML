<cfscript>
suiteBegin("OOP: component method named arguments");

widget = createObject("component", "oop.NamedArgWidget");
args = { first = "A", second = "B", third = "C" };

assert("named arguments bind by parameter name", widget.combine(third="C", first="A", second="B"), "ABC");
assert("argumentCollection expands by parameter name", widget.combine(argumentCollection=args), "ABC");

suiteEnd();
</cfscript>
