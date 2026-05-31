<cfscript>
suiteBegin("OOP: component method precedence");

service = createObject("component", "oop.MemberPrecedenceService");

assert("component delete method wins over struct helper", service.delete(id="abc"), "deleted:abc");
assert("component count method wins over struct helper", service.count(), "component-count");

suiteEnd();
</cfscript>
