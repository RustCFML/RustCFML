// Control fixture: `extends` as the FIRST component attribute, followed by
// another attribute. RustCFML already parses this shape — it is the regression
// guard that proves the fixture wiring (base lookup, createObject) is sound, so
// a failure on the sibling ExtendsAfterAttrFixture isolates to attribute ORDER.
component extends="DeclAttrBase" output="false" {
	public string function ping() {
		return "pong";
	}
}
