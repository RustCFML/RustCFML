component {

	// The canonical CFML lazy-init-by-exception idiom (as used by Preside/ColdBox
	// Controller.getRenderer). The first call reads an undefined component-scope
	// member, which must THROW so the catch builds and caches it.
	public any function getRenderer() {
		try {
			return variables._renderer;
		} catch (any e) {
			variables._renderer = "BUILT";
		}
		return variables._renderer;
	}

	// A bare read of a component member that is never set — must throw.
	public any function readMissing() {
		return variables._neverSet;
	}

}
