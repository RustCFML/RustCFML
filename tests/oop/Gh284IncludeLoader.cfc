/**
 * GitHub #284 fixture: include a .cfm from inside a CFC method and hand back
 * the function(s) it defined. Mirrors Wheels' `$reincludeGlobals` /
 * Loader.loadFunctions pattern — re-including a helper template the developer
 * (or the app itself) just rewrote, within the handling request.
 */
component output="false" {
	public struct function loadFunctions(required string file) {
		var beforeVarKeys = StructKeyList(variables);
		include "#arguments.file#";
		var fns = {};
		for (var varKey in variables) {
			if (!ListFindNoCase(beforeVarKeys, varKey) && IsCustomFunction(variables[varKey])) {
				fns[varKey] = variables[varKey];
			}
		}
		return fns;
	}
}
