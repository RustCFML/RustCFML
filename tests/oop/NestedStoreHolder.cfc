component {
	function init() { variables.count = 0; return this; }
	function bump() { variables.count = variables.count + 1; variables.last = "b" & variables.count; return this; }
	function nest() { variables.cache.inner.k = variables.count; return this; }
	function read() { return variables.count & "/" & variables.last & "/" & (structKeyExists(variables, "cache") ? variables.cache.inner.k : "-"); }
	function viaClosure() { var f = function() { variables.fromClosure = "c"; }; f(); return variables.fromClosure; }
}
