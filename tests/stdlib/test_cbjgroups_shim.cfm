<cfscript>
suiteBegin( "cbjgroups: CbJGroupsClusterWrapper shim and its OSGi loading (RustCFML)" );
// The Java object behind the cbjgroups module, backed by RustCFML's cluster.
// With no `cluster` configured it is a one-node cluster, as a JGroups channel
// that finds no peers is. The two-node behaviour (views, coordinator failover,
// delivery to other nodes) is exercised by scripts/cluster-smoke.sh.
_cbjDir = getDirectoryFromPath( getCurrentTemplatePath() ) & "cbjgroups_fixture/";
_cbjEngine = createObject( "java", "lucee.loader.engine.CFMLEngineFactory" ).getInstance();

// The module's jar goes through the engine's inert OSGi layer (as every bundled
// library's does); the class it then asks for is what has to exist.
assert( "expandPath keeps an existing absolute path", expandPath( _cbjDir & "lib/cbjgroups.jar" ), _cbjDir & "lib/cbjgroups.jar" );
_cbjRes = _cbjEngine.getResourceUtil().toResourceExisting( getPageContext(), expandPath( _cbjDir & "lib/cbjgroups.jar" ) );
createObject( "java", "lucee.runtime.osgi.OSGiUtil" ).installBundle( _cbjEngine.getBundleContext(), _cbjRes, true );
assertTrue( "the bundle install completes", true );
assertThrows( "an unmodelled class from the bundle still fails", function(){
	createObject( "java", "org.pixl8.cbjgroups.NotAClass", "org.pixl8.cbjgroups" );
} );

application.__cbjReceived = []; application.__cbjViews = 0; application.__cbjLastView = "";
w = createObject( "java", "org.pixl8.cbjgroups.CbJGroupsClusterWrapper", "org.pixl8.cbjgroups" ).init( "", false, new stdlib.cbjgroups_fixture.Listener(), {}, "/" );
assertFalse( "not connected before connect()", w.isConnected() );
w.connect( "rustcfml-test-cluster" );
assertTrue( "connected", w.isConnected() );
assertTrue( "a lone node is its own coordinator", w.isCoordinator() );
for ( i = 1; i <= 5; i++ ) { w.sendMessage( serializeJson( { n = i } ) ); }
for ( t = 1; t <= 100 && arrayLen( application.__cbjReceived ) < 5; t++ ) { sleep( 50 ); }
assert( "discardOwnMessages=false loops back, in send order", arrayToList( application.__cbjReceived ), "1,2,3,4,5" );
assertTrue( "a view is delivered on connect", application.__cbjViews >= 1 );
st = w.getStats();
assert( "stats: sent", st.sent_msgs, 5 );
assert( "stats: received", st.received_msgs, 5 );
assert( "stats: one member, this node", arrayLen( st.members ), 1 );
assert( "stats: self is that member", st.members[ 1 ], st.self );
assert( "stats: connection", st.connection, "CONNECTED" );
assertTrue( "stats: is_coordinator", st.is_coordinator );
w.close();
assertFalse( "close() disconnects", w.isConnected() );
assertThrows( "sendMessage after close throws", function(){ w.sendMessage( "x" ); } );

application.__cbjReceived = [];
w2 = createObject( "java", "org.pixl8.cbjgroups.CbJGroupsClusterWrapper" ).init( "", true, new stdlib.cbjgroups_fixture.Listener(), {}, "/" );
w2.connect( "rustcfml-test-cluster-2" );
w2.sendMessage( serializeJson( { n = 1 } ) );
sleep( 300 );
assert( "discardOwnMessages=true does not hear itself", arrayLen( application.__cbjReceived ), 0 );
assert( "but the send is counted", w2.getStats().sent_msgs, 1 );
w2.close();
suiteEnd();
</cfscript>
