component {
	variables.secret = "from-target";
	variables.calls = 0;
	public function own( x ) { return "own:" & arguments.x & ":" & variables.secret; }
	public function inject( name, fn ) { this[ name ] = fn; variables[ name ] = fn; return this; }
	public function callInjectedBare( x ) { return injected( x ); }
	public function callInjectedThis( x ) { return this.injected( x ); }
	public function callClosureBare( x ) { return clo( x ); }
	public function callClosureThis( x ) { return this.clo( x ); }
	public function getCalls() { return variables.calls; }
}
