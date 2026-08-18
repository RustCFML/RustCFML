/**
 * GH #330: an inaccessible method must be treated as ABSENT, so a component with
 * an onMissingMethod routes the refused call there rather than throwing.
 */
component {

	private string function hidden() {
		return "hidden";
	}

	public any function onMissingMethod( required string missingMethodName, required struct missingMethodArguments ) {
		return "omm:" & arguments.missingMethodName;
	}
}
