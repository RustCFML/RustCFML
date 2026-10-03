component extends="Plain452" {
	function bye() {
		return "bye";
	}

	function makeSibling() {
		return new Sib452();
	}

	function makeNested() {
		return new sub.Deep452();
	}
}
