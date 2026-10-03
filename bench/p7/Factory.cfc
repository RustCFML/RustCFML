component {
	public any function makeRel() { return new Plain20(); }
	public any function makeDotted() { return new bench.p7.Plain20(); }
	public any function makeCreateRel() { return createObject("component", "Plain20"); }
}
