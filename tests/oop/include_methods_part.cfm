<cfscript>
public string function fromInclude() {
	return "included:" & inlineOne() & ":" & variables.secret;
}
private string function hiddenFromInclude() {
	return "private";
}
public string function callsPrivate() {
	return hiddenFromInclude();
}
</cfscript>
