// Probe for the component introspection / struct-BIF surface.
// See tests/oop/test_component_introspection_surface.cfm.
component accessors="true" {
	property name="title";

	this.pubData = "pub";
	variables.privData = "priv";

	public function init() {
		this.pubData = "pub";
		variables.privData = "priv";
		setTitle( "T" );
		return this;
	}

	public function greet() {
		return "hi";
	}
}
