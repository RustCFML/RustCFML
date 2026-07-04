/**
 * Regression fixture: a component with BOTH `property name="after"` (accessors)
 * AND a same-named method `after()`, where the value is written via the
 * AUTO-GENERATED setter `setAfter()` at runtime (not `variables.after = ...`).
 * This is the exact shape of Sticker's util/Asset.cfc (`property name="after"` +
 * method `after()` + `dependsOn()` calling `this.after(argumentCollection=...)`).
 *
 * The generated setAfter() must NOT clobber the same-named method held on
 * `this.after`: getAfter() reads the `variables` backing, while `this.after()`
 * (and bare `after()`) must stay callable. Lucee parity.
 */
component accessors="true" {

	property name="after" type="array";

	function init(){
		setAfter( [] );   // runtime generated setter — must not clobber method after()
		return this;
	}

	// same-named method — must remain callable after setAfter() writes the backing
	function after( val = "" ){
		var a = getAfter();
		a.append( arguments.val );
		setAfter( a );
		return this;
	}

	// internal `this.method()` call (Asset.dependsOn shape)
	function callInternal( val ){
		this.after( arguments.val );
		return this;
	}
}
