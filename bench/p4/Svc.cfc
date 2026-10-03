component {
	variables.states = {};
	public any function init( required any state ) { variables.states["s1"] = arguments.state; return this; }
	public any function processState( required any state, any interceptData=structNew(), boolean async=false, boolean asyncAll=false, boolean asyncAllJoin=true, string asyncPriority="NORMAL", numeric asyncJoinTimeout=0 ) {
		if ( structKeyExists( variables.states, arguments.state ) ) {
			arguments.event  = variables.ctx;
			arguments.buffer = getLazyBuffer();
			var results = variables.states.find( arguments.state ).process( argumentCollection=arguments );
			if ( arguments.buffer.keyExists( "builder" ) ) { writeOutput( arguments.buffer.getString() ); }
		}
		if ( !isNull( results ) ) { return results; }
	}
	public any function processStateNoBuffer( required any state, any interceptData=structNew() ) {
		arguments.event  = variables.ctx; arguments.buffer = {};
		return variables.states.find( arguments.state ).process( argumentCollection=arguments );
	}
	public any function processStateDirect( required any state, any interceptData=structNew() ) {
		return variables.states.find( arguments.state ).process( event=variables.ctx, interceptData=arguments.interceptData, buffer={} );
	}
	struct function getLazyBuffer(){
		var buffer = {
			get = function(){ if( !buffer.keyExists( 'builder' ) ){ buffer.builder = createObject( "java", "java.lang.StringBuilder" ).init( '' ); } return buffer.builder; },
			clear = function(){ buffer.get().setLength( 0 ); },
			append = function( required str ){ buffer.get().append( arguments.str ); return this; },
			length = function(){ return buffer.get().length(); },
			getString = function(){ return buffer.get().toString(); }
		};
		return buffer;
	}
	public void function setCtx( required any ctx ) { variables.ctx = arguments.ctx; }
}
