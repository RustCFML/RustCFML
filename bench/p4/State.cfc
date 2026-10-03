component {
	instance = { pool=[], poolSize=0, state="s1" };
	public any function init( required array interceptors ) { for ( var i in arguments.interceptors ) { arrayAppend( instance.pool, { key="k#instance.poolSize#", target=i } ); instance.poolSize++; } return this; }
	public any function process( required any event, required any interceptData, required any buffer, boolean async=false, boolean asyncAll=false, boolean asyncAllJoin=true, string asyncPriority="NORMAL", numeric asyncJoinTimeout=0 ) {
		processSync( event=arguments.event, interceptData=arguments.interceptData, buffer=arguments.buffer );
	}
	private any function processSync( any event, any interceptData, any buffer ) {
		for( var i=1; i<=instance.poolSize; i++ ){
			var key = instance.pool[ i ].key; var thisInterceptor = instance.pool[ i ].target;
			if ( invoker( interceptor=thisInterceptor, event=arguments.event, interceptData=arguments.interceptData, interceptorKey=key, buffer=arguments.buffer ) ) { break; }
		}
	}
	private boolean function invoker( required any interceptor, required any event, required any interceptData, required any interceptorKey, required any buffer ) {
		var refLocal = {};
		refLocal.results = arguments.interceptor[ instance.state ]( event=arguments.event, interceptData=arguments.interceptData, buffer=arguments.buffer, rc=arguments.event.getCollection(), prc=arguments.event.getPrivateCollection() );
		if ( StructKeyExists( refLocal, "results" ) && IsBoolean( refLocal.results ) ) { return refLocal.results; }
		return false;
	}
}
