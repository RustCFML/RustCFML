component {

	// Fixture for core/test_bare_call_caller_stack_leak.cfm.
	//
	// bclTarget() calls two component methods by BARE NAME while ancestor
	// frames on the call stack hold same-named DATA (struct params / a var).
	// On a conforming engine the ancestor frames are invisible to the
	// callee's function-name resolution; the data never shadows the methods.

	// The two methods the deepest frame calls by bare name.
	public string function bclFnA() {
		return "FN_A_RESULT";
	}

	public string function bclFnB() {
		return "FN_B_RESULT";
	}

	// Immediate caller of bclTarget(): declares a STRUCT param named bclFnA.
	// The param is pure data; it must be invisible to bclTarget()'s bare call.
	public struct function viaMid(struct bclFnA = {}) {
		return bclTarget();
	}

	// Grandparent of bclTarget(): declares a STRUCT param named bclFnB, then
	// calls viaMid() (which itself shadows bclFnA) — both ancestor frames
	// shadow at once, two different depths.
	public struct function viaDeep(struct bclFnB = {}) {
		return viaMid();
	}

	// Ancestor shadows via a LOCAL (var) instead of a param.
	public struct function viaLocalShadow() {
		var bclFnA = {iAmData = true};
		return bclTarget();
	}

	// Deepest frame: bare-name calls + scoped controls. Each call's result or
	// error message is collected into a struct (never re-thrown) so the test
	// can assert every case independently.
	public struct function bclTarget() {
		var res = {
			bareA   = "", bareAError   = "",
			bareB   = "", bareBError   = "",
			scopedA = "", scopedAError = "",
			thisA   = "", thisAError   = ""
		};
		try { res.bareA   = bclFnA(); }           catch (any e) { res.bareAError   = e.message; }
		try { res.bareB   = bclFnB(); }           catch (any e) { res.bareBError   = e.message; }
		try { res.scopedA = variables.bclFnA(); } catch (any e) { res.scopedAError = e.message; }
		try { res.thisA   = this.bclFnA(); }      catch (any e) { res.thisAError   = e.message; }
		return res;
	}
}
