// Gap A fixture: `extends` appears AFTER another attribute (`output="false"`).
// On Lucee/Adobe CF/BoxLang component attributes are order-independent, so this
// parses and instantiates exactly like ExtendsFirstFixture. On RustCFML 0.36.0
// the parser only accepts `extends` as the FIRST attribute; placed later it
// fails to parse and the component degrades to a non-object.
//
// This is the dominant header shape in the Wheels framework — every CFC in the
// boot cascade is written `component output="false" ... extends="wheels.Global" {`
// (Controller, Model, Dispatch, Migrator, Plugins, Public, Test).
component output="false" extends="DeclAttrBase" {
	public string function ping() {
		return "pong";
	}
}
