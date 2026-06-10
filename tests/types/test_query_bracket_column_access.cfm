<cfscript>
suiteBegin("Types: query bracket-column cell access (Lucee parity)");

// query["columnName"][rowNumber] (bracket-column + index) must return the cell
// value, exactly like query.columnName[rowNumber] (dot form). On Lucee 5/6/7,
// Adobe ColdFusion, and BoxLang both forms return the cell. RustCFML returns an
// EMPTY value for the bracket form while the dot form works.
//
// This matters because frameworks read query cells via the bracket form when
// the column name is dynamic (held in a variable). Wheels' ORM column
// processing does exactly this (vendor/wheels/Model.cfc:
// local.columns["column_name"][local.i]) — so every introspected column comes
// back blank, and the model can't build its property map.
//
// A datasource is intentionally NOT used: queryNew() reproduces the gap with no
// DB dependency, so the test is portable and deterministic.

q = queryNew(
	"column_name,type_name",
	"varchar,varchar",
	[ ["id", "INTEGER"], ["title", "TEXT"] ]
);

// --- CONTROL: dot-form cell access works on BOTH engines (guards the wiring) ---
assert("control: dot q.column_name[1]", q.column_name[1], "id");
assert("control: dot q.column_name[2]", q.column_name[2], "title");

// --- GAP: bracket-column + index must return the same cell value ---
assert("bracket q['column_name'][1]", q["column_name"][1], "id");
assert("bracket q['column_name'][2]", q["column_name"][2], "title");
assert("bracket q['type_name'][1]", q["type_name"][1], "INTEGER");

// --- the realistic framework shape: a dynamic (variable) column-name key ---
colKey = "column_name";
assert("bracket dynamic-key q[colKey][2]", q[colKey][2], "title");

suiteEnd();
</cfscript>
