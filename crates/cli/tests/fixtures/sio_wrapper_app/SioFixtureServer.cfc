/**
 * A cut-down stand-in for socket.io-lucee's SocketIoServer.cfc: the server model
 * lives here in CFML, a Java object carries the wire, and that object calls this
 * component back. It holds the wrapper in its OWN variables scope, exactly as
 * the real library holds `variables._javaServer` — which is what makes the
 * wrapper reachable from a dispatch that has no request context.
 */
component {

	public any function init() {
		variables._javaServer = createObject(
			  "java"
			, "com.pixl8.socketiolucee.SocketIoServerWrapper"
			, "com.pixl8.socketio-lucee"
		).init(
			  this               // handlerCfc — the callback target
			, expandPath( "/" )
			, {}                 // appContext
			, "0.0.0.0"
			, 3000
			, false
			, 25000
			, 60000
			, 10
			, javacast( "null", "" )
		);

		variables._javaServer.registerNamespace( "/im" );
		variables._javaServer.startServer();

		return this;
	}

	public boolean function isRunning() {
		return variables._javaServer.isServerRunning();
	}

	public string function state() {
		return variables._javaServer.getServerState();
	}

	// --- the listener interface the Java server calls back on ---

	public void function onConnect( required string namespace, required string socketId, required any initialRequest ) {
		// The library reads its session cookie straight out of this shape.
		var psid = "";
		for( var c in initialRequest.get( "cookies" ) ) {
			if ( ( c.name ?: "" ) == "PSID" ) {
				psid = c.value ?: "";
			}
		}

		variables._javaServer.socketSend( arguments.namespace, arguments.socketId, "welcome", [ {
			  id    = arguments.socketId
			, psid  = psid
			, query = initialRequest.get( "querystring" )
		} ] );
	}

	public void function onSocketEvent( required string namespace, required string socketId, required string event, array args=[] ) {
		if ( arguments.event == "say" ) {
			variables._javaServer.socketSend( arguments.namespace, arguments.socketId, "sayEcho", [ {
				  routed = "say"
				, text   = ( arguments.args[ 1 ].text ?: "" )
			} ] );
		}
	}

	public void function onDisconnecting( required string namespace, required string socketId ) {}

	public void function onDisconnect( required string namespace, required string socketId ) {}
}
