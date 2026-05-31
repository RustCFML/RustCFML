// Gap B fixture: an UNQUOTED boolean keyword as a component attribute value
// (`output=false`). On Lucee/Adobe CF/BoxLang an unquoted boolean (or bare
// identifier) is a legal attribute value, so this parses and instantiates. On
// RustCFML 0.36.0 the value `false` lexes as a Boolean-literal token and the
// component-attribute parser rejects it ("Expected LBrace, found False"); the
// component degrades to a non-object.
//
// Wheels uses this house style across its database adapters:
// `component extends="wheels.databaseAdapters.Base" output=false {`
// (Base + the MySQL/PostgreSQL/MSSQL/Oracle/SQLite/H2/CockroachDB models).
component output=false {
	public string function ping() {
		return "pong";
	}
}
