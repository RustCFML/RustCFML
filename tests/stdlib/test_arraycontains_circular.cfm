<!---
	Regression: arrayContains / arrayFind on an array holding a self-referential
	(circular) struct must terminate. cfml_deep_equal walks struct keys, so a
	struct that references itself (or a cycle a→b→a) recursed forever and
	stack-overflowed the process. Lucee compares such references by identity;
	we short-circuit when two operands share the same backing handle.

	This is what crashed Wheels' model.errorsSpec "handles circular reference"
	(and thus the whole TestBox suite): allErrors(includeAssociations=true) uses
	arrayContains over visited association objects to break the cycle.
--->
<cfscript>
	suiteBegin("arrayContains/arrayFind on circular structures");

	// Direct self-reference.
	a = { name = "a" };
	a.self = a;
	arr = [ a ];
	assert("arrayContains finds the self-referential struct (by identity)", arrayContains(arr, a), true);
	assert("arrayFind returns its 1-based index", arrayFind(arr, a), 1);

	// Two-node cycle a <-> b.
	x = { id = 1 };
	y = { id = 2 };
	x.peer = y;
	y.peer = x;
	list2 = [ x, y ];
	assert("cycle member x found", arrayContains(list2, x), true);
	assert("cycle member y found", arrayContains(list2, y), true);

	// A structurally-cyclic but DIFFERENT instance is not found (identity, not deep).
	z = { id = 1 };
	z.peer = z;
	assert("distinct instance not matched", arrayContains([x], z), false);

	// Nested arrays that alias the same backing handle compare equal safely.
	inner = [ 1, 2, 3 ];
	holder = [ inner ];
	assert("array holding an aliased inner array", arrayContains(holder, inner), true);

	suiteEnd();
</cfscript>
