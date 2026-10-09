component extends="MixinSuper" {
	function init() { loadApplicationHelpers(); return this; }
	function gh481RenderView( view="" ) { return "REAL[" & arguments.view & "]"; }
}
