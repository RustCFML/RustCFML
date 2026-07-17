// Declares a RELATIVE implements path ("sub.IIfaceThing") that must resolve
// against THIS component's directory (tests/oop/ifaceinherit/), even when the
// clause is inherited by a subclass that lives in a different directory.
// Mirrors ColdBox's AbstractCacheBoxProvider (system/cache/) declaring
// implements="providers.ICacheProvider".
component implements="sub.IIfaceThing" {
	function doThing() {
		return "done";
	}
}
