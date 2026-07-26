component {

	public any function init() {
		return this;
	}

	public struct function probe() {
		variables.x    = "cfc-vars-original";
		variables.togo = "delete-me";
		var x = "method-local-original";

		module template="caller_semantics_probe.cfm";

		return {
			  methodLocalX = x
			, cfcVarsX     = variables.x
			, cfcVarsNewk  = variables.newk ?: "(missing)"
			, cfcTogoExists = StructKeyExists( variables, "togo" )
			, tagRead = variables.tagReadXBeforeWrite ?: "(missing)"
		};
	}
}
