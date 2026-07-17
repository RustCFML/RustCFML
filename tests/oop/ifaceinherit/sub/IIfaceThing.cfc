// Interface used by the inherited-relative-implements regression test.
// Lives two dirs below the concrete leaf so the declaring abstract's
// directory (../) differs from the leaf's, exercising the resolution path.
interface {
	function doThing();
}
