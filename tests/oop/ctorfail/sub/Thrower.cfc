component extends="Base" {
	public any function init() {
		throw( type="my.ctor.failure", message="pseudo-constructor dependency failed" );
	}
}
