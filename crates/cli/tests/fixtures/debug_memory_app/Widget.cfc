component {
	function init( required numeric n ) {
		variables.n = arguments.n;
		variables.payload = repeatString( "w", 200 );
		return this;
	}
	function build() {
		var rows = [];
		for ( var i = 1; i <= 200; i++ ) {
			arrayAppend( rows, { i = i, label = "row " & i } );
		}
		return rows;
	}
}
