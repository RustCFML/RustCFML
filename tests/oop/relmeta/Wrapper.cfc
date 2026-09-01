component {
	// The shape WireBox-managed models use: a sibling package addressed
	// RELATIVELY from inside another component.
	public any function viaNew()          { return new algorithms.Rsa(); }
	public any function viaCreateObject() { return createObject( "component", "algorithms.Rsa" ); }
}
