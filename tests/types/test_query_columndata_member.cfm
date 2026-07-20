<cfscript>
// Query member `columnData(col)` must return the column's values as an array
// (Lucee parity), mapping to the queryColumnData builtin. Previously the member
// form fell through the query-method dispatch to a Null return, so
// `var x = q.columnData(col)` silently left `x` UNDEFINED (null-assignment
// unsets the var). That surfaced in Preside's ObjectPicker._removeInvalidValues:
// `var validValues = selectData(...).columnData(id)` then a `.filter()` closure
// referencing validValues threw "Variable 'validValues' is undefined", breaking
// the rich-editor widget/image picker prefill.
suiteBegin("Type: query.columnData() member");

q = queryNew("id,name");
queryAddRow(q); querySetCell(q, "id", "A"); querySetCell(q, "name", "Alice");
queryAddRow(q); querySetCell(q, "id", "B", 2); querySetCell(q, "name", "Bob", 2);

// Direct member call returns the column as an array.
direct = q.columnData("id");
assertTrue("q.columnData() returns an array", isArray(direct));
assert("q.columnData() array length", arrayLen(direct), 2);
assert("q.columnData() values", direct.toList(), "A,B");

// Member and builtin forms agree.
assert("member == queryColumnData builtin", q.columnData("id").toList(), queryColumnData(q, "id").toList());

// var-assignment from the chained call binds the local (regression: was undefined).
function bindsLocal() {
	var vals = queryNew("id","varchar");
	queryAddRow(vals); querySetCell(vals, "id", "A");
	queryAddRow(vals); querySetCell(vals, "id", "B", 2);
	var validValues = vals.columnData("id");
	return isDefined("validValues") && isArray(validValues) ? validValues.toList() : "UNDEFINED";
}
assert("var x = q.columnData() binds the local", bindsLocal(), "A,B");

// Exact ObjectPicker shape: chained columnData feeding a .filter() closure.
function objectPickerShape( required string values ) {
	var initialValues = listToArray( arguments.values );
	var backing = queryNew("id","varchar");
	queryAddRow(backing); querySetCell(backing, "id", "A");
	queryAddRow(backing); querySetCell(backing, "id", "B", 2);
	var validValues = backing.columnData( "id" );
	var cleaned = initialValues.filter( function( value ){
		return validValues.find( value );
	} );
	return cleaned.toList();
}
assert("columnData result captured by a .filter() closure", objectPickerShape("A,X,B"), "A,B");

suiteEnd();
</cfscript>
