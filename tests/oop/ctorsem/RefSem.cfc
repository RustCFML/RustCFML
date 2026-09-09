component {
	// Values assigned in the pseudo-constructor must stay LIVE REFERENCES to the
	// caller's structs — not private copies taken when the instance is assembled.
	variables.cfg  = request.ctorsemCfg;
	this.cfg2      = request.ctorsemCfg;
	variables.both = this.both = {};
	function init() { return this; }
	function mutate() {
		variables.cfg.x  = 2;
		this.cfg2.y      = 3;
		variables.both.z = 4;
		return this.both.z;
	}
	function varKeys() { return structKeyList( variables ); }
}
