component {

	function init() {
		variables.data = { a = 1 };
		return this;
	}

	boolean function existsLocked( required name ) {
		lock name="finallyReturnFixtureLock" type="readonly" timeout="20" {
			return StructKeyExists( variables.data, arguments.name );
		}
	}

	string function labelLocked( required name ) {
		lock name="finallyReturnFixtureLock" type="readonly" timeout="20" {
			sleep( 1 );
			return "label-" & arguments.name;
		}
	}

	string function labelTryFinally( required name ) {
		try {
			sleep( 1 );
			return "label-" & arguments.name;
		} finally {
			var cleanedUp = true;
		}
	}

	array function variablesKeys() {
		return StructKeyArray( variables );
	}
}
