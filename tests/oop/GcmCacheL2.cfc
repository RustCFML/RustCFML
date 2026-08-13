component extends="oop.GcmCacheL3" {
	property name="l2prop" type="numeric" default="2";

	public string function l2One( required string a, numeric b=2 ) { return "l2One"; }
	private void function l2Two() {}
}
