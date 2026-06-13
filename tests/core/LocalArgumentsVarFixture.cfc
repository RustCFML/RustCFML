component {

	// A local-scoped variable that happens to be NAMED `arguments` must behave
	// like any other local struct. This is the shape a Moopa route dispatcher
	// uses: it builds a working struct `local.arguments = {}`, fills it with
	// the matched route params, and hands it to the endpoint. The name is
	// incidental — `local.arguments` is a variable in the `local` scope, not
	// the `arguments` scope.
	public string function build() {
		local.arguments = {};
		local.arguments["route"]    = "tracks/abc";
		local.arguments["track_id"] = "THE-ID";
		structAppend(local.arguments, { "extra": "Z" }, true);

		return "keys=[" & structKeyList(local.arguments) & "]"
			& " | track_id=[" & (local.arguments.track_id ?: "NULL") & "]"
			& " | extra=[" & (local.arguments.extra ?: "NULL") & "]";
	}
}
