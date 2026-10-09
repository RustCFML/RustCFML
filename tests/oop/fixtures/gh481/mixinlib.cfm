<cfscript>
function probeLocal() {
	return isDefined( "methodLocal" ) ? methodLocal : "NO-METHOD-LOCAL";
}
function probeVars() {
	return isDefined( "instanceVar" ) ? instanceVar : "NO-INSTANCE-VAR";
}
function probeWrite() {
	writeCount = ( isDefined( "writeCount" ) ? writeCount : 0 ) + 1;
	return writeCount;
}
</cfscript>
