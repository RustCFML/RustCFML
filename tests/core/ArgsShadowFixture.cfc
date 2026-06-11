component {

	// Mirrors the vendor/wheels/Global.cfc URLFor() shape that surfaced the
	// local-shadows-arguments gap: a declared `string params = ""` argument, a
	// same-named route-params struct built in the local scope, and a late
	// `Len(arguments.params)` check that appends the query string. With the
	// gap, Len() sees the local struct (always truthy) and the struct itself
	// is concatenated into the URL.
	public string function buildUrl(string controller = "posts", string params = "") {
		local.params = {controller: arguments.controller, action: "index"};
		local.rv = "/" & local.params.controller;
		if (Len(arguments.params)) {
			local.rv = local.rv & "?" & arguments.params;
		}
		return local.rv;
	}
}
