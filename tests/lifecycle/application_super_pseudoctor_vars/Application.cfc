component extends="Bootstrap" {
	// The whole body is the super call, so the frame never materializes a `this`
	// — that is the condition the regression needed.
	super.setupApplication( id = "rustcfml-super-pseudoctor-vars-test" );
}
