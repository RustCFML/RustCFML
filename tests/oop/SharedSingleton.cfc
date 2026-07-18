component {
	variables.count = 0;
	function increment() {
		variables.count++;
		return this;
	}
	function getCount() {
		return variables.count;
	}
}
