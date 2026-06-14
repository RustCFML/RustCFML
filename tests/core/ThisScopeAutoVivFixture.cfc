// Fixture for the THIS-scope nested auto-vivification test. Each probe runs
// inside a CFC method (the Wheels Migrator.cfc init() context) and reports the
// exact state the `this` scope ended up in, so a silent-loss engine fails with
// a diagnosis instead of a bare value mismatch.
//
// Mirrors ScopedAutoVivFixture.cfc (the variables/local/request residual from
// PR #111) -- this fixture isolates the one scope #111 MISSED: `this`.
component {

	// The exact Wheels Migrator.cfc init() shape: this.paths.migrate = "..."
	// where this.paths is never pre-seeded. Reports container state + value.
	public string function vivThisNested() {
		try {
			this.zzpThisViv.migrate = "viv-this";
		} catch (any e) {
			return "THREW: " & e.message;
		}
		if (!StructKeyExists(this, "zzpThisViv")) {
			return "NO-CONTAINER";
		}
		if (!IsStruct(this.zzpThisViv)) {
			return "NOT-A-STRUCT";
		}
		if (!StructKeyExists(this.zzpThisViv, "migrate")) {
			return "KEY-LOST";
		}
		return "migrate=[" & this.zzpThisViv.migrate & "]";
	}

	// Two-level chain (Migrator stores several path keys; a deep mixin shape):
	// this.X.a.b must vivify EVERY missing level as a struct.
	public string function vivThisDeep() {
		try {
			this.zzpThisDeep.a.b = "deep-this";
		} catch (any e) {
			return "THREW: " & e.message;
		}
		if (!StructKeyExists(this, "zzpThisDeep")) {
			return "NO-CONTAINER";
		}
		if (!IsStruct(this.zzpThisDeep)) {
			return "OUTER-NOT-A-STRUCT";
		}
		if (!StructKeyExists(this.zzpThisDeep, "a") || !IsStruct(this.zzpThisDeep.a)) {
			return "INNER-NOT-A-STRUCT";
		}
		return "b=[" & this.zzpThisDeep.a.b & "]";
	}

	// CONTROL A -- pins that PR #111 still holds inside a CFC method: a
	// variables-scope nested write on an undeclared key must auto-vivify
	// (this is the fix that shipped in v0.136 and survives on 0.153.0).
	public string function vivVariablesControl() {
		try {
			variables.zzvThisCtl.k = "viv-vars";
		} catch (any e) {
			return "THREW: " & e.message;
		}
		if (!StructKeyExists(variables, "zzvThisCtl") || !IsStruct(variables.zzvThisCtl)) {
			return "NOT-A-STRUCT";
		}
		return "k=[" & variables.zzvThisCtl.k & "]";
	}

	// CONTROL B -- pins that the failure is auto-VIVIFICATION, not this-scope
	// writes in general: a PRE-INITIALIZED this container takes a nested write
	// just fine. Isolates the missing implicit `this.X = {}` step.
	public string function vivThisPreInit() {
		this.zzqThisPre = {};
		try {
			this.zzqThisPre.k = "pre-this";
		} catch (any e) {
			return "THREW: " & e.message;
		}
		if (!StructKeyExists(this.zzqThisPre, "k")) {
			return "KEY-LOST";
		}
		return "k=[" & this.zzqThisPre.k & "]";
	}

}
