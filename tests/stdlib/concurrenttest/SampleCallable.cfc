/**
 * Fixture: a Callable-style CFC whose single-abstract-method `call()` returns a
 * value. Submitted to a java.util.concurrent executor via createDynamicProxy —
 * the exact pattern ColdBox's Executor.cfc uses. Works on both Lucee (real JVM
 * proxy + executor) and RustCFML (shim + native async kernel).
 */
component {

	public any function call(){
		return "called!";
	}

}
