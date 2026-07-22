// Child of FlyweightSurfaceProbe — inheritance-aware surface for the parity test.
component extends="FlyweightSurfaceProbe" accessors="true" {
	property name="extra";

	public function childOnly() {
		return "child";
	}
}
