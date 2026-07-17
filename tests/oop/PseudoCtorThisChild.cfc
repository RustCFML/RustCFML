component extends="PseudoCtorThisParent" output="false" {
	// Reads inherited this.* members set by the parent pseudo-constructor
	// (Masa/Mura contentRenderer pattern: site child reads core parent's
	// this.formInputClass). These would throw "Variable is undefined" before
	// the fix, because inheritance merged parent members only AFTER the body.
	this.commentSortSelectClass = this.formInputClass;
	this.commentSubmitButtonClass = this.formButtonClass;
	// A child override wins over the inherited value.
	this.formButtonClass = "btn-child";

	public string function whoAmI() {
		// super still resolves to the parent method during/after construction
		return "child-of-" & super.whoAmI();
	}
}
