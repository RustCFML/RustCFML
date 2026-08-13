component extends="oop.GcmCacheL2" {
	property name="l1prop" type="string" default="one";

	public string function l1One() { return "l1One"; }
	public any function l1Two( struct s={} ) { return s; }
	public any function init() { return this; }
}
