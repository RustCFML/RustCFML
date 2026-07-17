<cfscript>
// Regression: a parent's explicit `this.*` data members (set in its
// pseudo-constructor) must be visible on `this` while a SUBCLASS's
// pseudo-constructor body runs. CFML runs the parent pseudo-ctor first, on the
// same `this` object, so `this.parentMember` is present when the child body
// executes. RustCFML resolved the parent separately and merged its members only
// AFTER the child body, so `this.formInputClass` threw "Variable is undefined".
// This surfaced in Masa/Mura CMS: sites/default/contentRenderer.cfc reads
// `this.commentInputClass = this.formInputClass` where the core parent
// contentRenderer set `this.formInputClass` — 6 caught exceptions per front-end
// request. Cross-checked on Lucee 7.
suiteBegin("Subclass pseudo-constructor sees parent this.* members");

o = createObject("component", "PseudoCtorThisChild");

// Child body read the parent's this.formInputClass (itself derived from another
// parent this.* member) and stored it on a child member.
assert("child reads inherited this.formInputClass during construction",
	o.commentSortSelectClass, "form-control");

// The inherited member itself survives onto the instance.
assert("inherited this.formInputClass present on instance",
	o.formInputClass, "form-control");
assert("inherited this.formGeneralControlClass present on instance",
	o.formGeneralControlClass, "form-control");

// A child override of an inherited this.* member wins.
assert("child override of inherited this.* member wins",
	o.formButtonClass, "btn-child");

// The child read the parent's this.formButtonClass ("btn") BEFORE overriding it,
// so the derived member captured the parent's value.
assert("child captured parent this.formButtonClass before overriding",
	o.commentSubmitButtonClass, "btn");

// super still dispatches to the parent method.
assert("super.method() resolves to parent", o.whoAmI(), "child-of-parent");

suiteEnd();
</cfscript>
