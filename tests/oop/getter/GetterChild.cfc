component extends="GetterBase" {
	variables._objects = { a = 1 };
	variables.services = { requestService = "rs" };
	variables._instance = "inst";
	variables.notStruct = "str";
	variables.fn = function() { return "called"; };

	private struct function _getObjects() { return _objects; }
	function getRequestService() { return services.requestService; }
	public any function getV() { return variables._instance; }
	private any function _getInstance() { return variables._instance ?: "none"; }
	function getMissingElvis() { return variables.nope ?: "fallback"; }
	struct function getWrongType() { return notStruct; }
	function getUndefined() { return undefinedThing; }
	function getMissingKey() { return services.nope; }
	function getFn() { return fn; }

	function bareShapes() {
		return [ _getObjects().a, getRequestService(), getV(), _getInstance(), getBaseVal(), getMissingElvis() ];
	}
	function memberShapes() {
		return [ this.getRequestService(), this.getV(), this.getBaseVal(), this.getMissingElvis() ];
	}
	function bareWithShadowingLocal() {
		var _objects = "caller local";
		var _instance = "caller local";
		return [ _getObjects().a, getV(), _getInstance() ];
	}
	function setInstance( required string v ) { variables._instance = arguments.v; }
	function clearInstance() { structDelete( variables, "_instance" ); }
	function privateGetterFromInside() { return _getObjects(); }
	function elvisInstance() { return _getInstance(); }
	public struct function getRequestCtx() {
		return request.__getterTestCtx ?: setRequestCtx( { made = true } );
	}
	public struct function setRequestCtx( required struct ctx ) {
		request.__getterTestCtx = arguments.ctx;
		return request.__getterTestCtx;
	}
	function requestCtxBare() { return getRequestCtx(); }
}
