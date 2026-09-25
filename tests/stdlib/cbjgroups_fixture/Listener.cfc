component {
	function init() { variables.ctx = getPageContext().getApplicationContext(); return this; }
	public void function receive( required any msg ) {
		getPageContext().setApplicationContext( variables.ctx );
		var m = deserializeJson( toString( msg.getBuffer() ) );
		lock name="cbjgroups_fixture" type="exclusive" timeout=5 { arrayAppend( request.__cbjReceived ?: [], "" ); arrayAppend( application.__cbjReceived, m.n ); }
	}
	public void function viewAccepted( required any view ) {
		lock name="cbjgroups_fixture" type="exclusive" timeout=5 { application.__cbjViews++; application.__cbjLastView = view.getMembers().toList(); }
	}
}
