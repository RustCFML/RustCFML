component {
	variables.who = "def2";
	function mkProbe() { return function() { return "callerLocal=" & isDefined("callerLocal") & " callerArg=" & isDefined("arguments.x") & " defVar=" & variables.who; }; }
	function later() { var f = function() { return isDefined("afterVar") ? "sees:" & afterVar : "undef"; }; var afterVar = 5; return f(); }
	function seesArgs( a ) { var f = function() { return isDefined("arguments.a") ? "outerArg:" & arguments.a : "noOuterArg"; }; return f(); }
	function seesArgsBare( a ) { var f = function() { return isDefined("a") ? "bare:" & a : "noBare"; }; return f(); }
	function mutateLater() { var n = 1; var f = function() { return n; }; n = 2; return f(); }
	function closureWritesLocal() { var n = 1; var f = function() { n = 7; return n; }; f(); return n; }
}
