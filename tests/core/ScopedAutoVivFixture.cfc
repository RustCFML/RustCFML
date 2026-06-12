// Fixture for the scope-qualified nested auto-vivification test. Each probe
// performs the write inside a CFC method (the Wheels $initControllerClass
// context) and reports exactly what state the variables scope ended up in,
// so a silent-loss engine fails with a diagnosis instead of a bare mismatch.
component {

	// The exact Wheels Controller.cfc shape: variables.$class.name = ...
	// where variables.$class is never pre-seeded ($-prefixed key included).
	public string function vivClassName() {
		try {
			variables.$zzscvivClass.name = "vivified";
		} catch (any e) {
			return "THREW: " & e.message;
		}
		if (!StructKeyExists(variables, "$zzscvivClass")) {
			return "NO-CONTAINER";
		}
		if (!IsStruct(variables.$zzscvivClass)) {
			return "NOT-A-STRUCT";
		}
		return "name=[" & variables.$zzscvivClass.name & "]";
	}

	// Two-level chain (csrf mixin shape: variables.$class.csrf.type = ...):
	// every missing level must vivify as a struct.
	public string function vivDeep() {
		try {
			variables.scvivDeep.a.b = "deep-viv";
		} catch (any e) {
			return "THREW: " & e.message;
		}
		if (!StructKeyExists(variables, "scvivDeep")) {
			return "NO-CONTAINER";
		}
		if (!IsStruct(variables.scvivDeep)) {
			return "OUTER-NOT-A-STRUCT";
		}
		if (!StructKeyExists(variables.scvivDeep, "a") || !IsStruct(variables.scvivDeep.a)) {
			return "INNER-NOT-A-STRUCT";
		}
		return "b=[" & variables.scvivDeep.a.b & "]";
	}

}
