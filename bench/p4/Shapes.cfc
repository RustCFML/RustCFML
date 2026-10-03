component {
	public void function zero() {}
	public void function five( a, b, c, d, e ) {}
	public void function fiveTyped( required any a, required any b, required any c, required any d, required any e ) {}
	public void function fiveDefaults( any a, boolean b=false, boolean c=false, string d="NORMAL", numeric e=0 ) {}
	public void function withLocal() { var refLocal = {}; refLocal.r = 1; }
	public void function dyn( required any o ) { arguments.o[ "zero" ](); }
	public void function dynNamed( required any o ) { arguments.o[ "five" ]( a=1, b=2, c=3, d=4, e=5 ); }
	public void function passColl( a, b, c, d, e ) { five( argumentCollection=arguments ); }
	public void function passCollExtra( a, b, c ) { arguments.d = 4; arguments.e = 5; five( argumentCollection=arguments ); }
	public void function callNamed() { five( a=1, b=2, c=3, d=4, e=5 ); }
	public void function callPos() { five( 1, 2, 3, 4, 5 ); }
	public void function mCallNamed( required any o ) { arguments.o.five( a=1, b=2, c=3, d=4, e=5 ); }
	public void function mPassColl( required any o, a, b, c, d, e ) { arguments.o.five( argumentCollection=arguments ); }
	public void function mCallPos( required any o ) { arguments.o.five( 1, 2, 3, 4, 5 ); }
	public void function argsDot( a, b ) { var x = arguments.a; var y = arguments.b; }
}
