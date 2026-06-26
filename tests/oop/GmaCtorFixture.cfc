component {
	// Read the component's own metadata name DURING the pseudo-constructor.
	variables.nameAtCtor = getMetadata(this).name;
	public function getNameAtCtor() {
		return variables.nameAtCtor;
	}
}
