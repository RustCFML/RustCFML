component output="false" {
	// Explicit this.* data members set in the parent pseudo-constructor.
	// A subclass reads these on `this` while ITS pseudo-constructor runs —
	// CFML runs the parent pseudo-ctor first, on the same `this` object.
	this.formGeneralControlClass = "form-control";
	this.formInputClass = this.formGeneralControlClass;
	this.formButtonClass = "btn";

	public string function whoAmI() {
		return "parent";
	}
}
