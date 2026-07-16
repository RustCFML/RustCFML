component {
	// Minimal probe used by test_scope_member_mutators.cfm to prove a COMPONENT
	// stored through a returned scope handle survives as a live object.
	public function ping() {
		return "pong";
	}
}
