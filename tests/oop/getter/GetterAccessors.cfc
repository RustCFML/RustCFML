component accessors="true" {
	property name="levelMax" type="numeric";
	property name="label";
	function init() { variables.levelMax = 4; return this; }
	function readBare() { return getLevelMax(); }
}
