<cfscript>
suiteBegin("OOP: component mapping paths");

widget = createObject("component", "/oop/LeadingSlashMappingWidget").init();

assert("leading slash component path resolves via mapping", widget.ready, "ok");

suiteEnd();
</cfscript>
