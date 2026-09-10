component {
	// Fixture for test_static_across_requests.cfm: the static scope must survive
	// the request that initialised it.
	static { hits = 0; }
	function bump() { static.hits++; return static.hits; }
}
