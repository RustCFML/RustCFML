/**
 * Fixture for test_chained_writeback_clobber.cfm. Holds an inner CFC and hands
 * it back via getDep(), so `outer.getDep().setMark(x)` is a chained call whose
 * outer mutating method runs on a DIFFERENT CFC than the base variable.
 */
component accessors="true" {

	function init(){
		variables.dep = new ChainClobberInner();
		return this;
	}

	function getDep(){
		return variables.dep;
	}

	function whoAmI(){
		return "Outer";
	}

	// Returns a fresh array — used to prove that `outer.getItems().sort()`
	// (an in-place array member fn chained on a method that returns an array)
	// does not write the sorted array back onto `outer`.
	function getItems(){
		return [ "b", "a", "c" ];
	}

	// Reproduces ColdBox blocker #8: a function-local `var` declared with one
	// casing (`baseVar`) but chain-called with another (`basevar`). The chained
	// outer call runs on a DIFFERENT CFC (the inner dep), and codegen propagates
	// the single-segment write-back path (["basevar"]) to it. The write-back's
	// chained-CFC identity guard did scope_aware_load("basevar") — which was
	// case-SENSITIVE while scope_aware_store is case-INSENSITIVE — missed the
	// `baseVar` local, saw existing=None, disarmed, and let the inner dep clobber
	// the base local. (ColdBox `cbcontroller.getRenderer().layout()` overwrote the
	// `var cbController` ControllerDecorator with the Renderer.)
	function probeCaseMismatch(){
		var baseVar = new ChainClobberOuter();
		basevar.getDep().setMark( "Z" );
		return baseVar.whoAmI();
	}

}
