component {
	// A component with its own `label` and no `extra`, used from inside a
	// query loop whose query has both columns (GH #544).
	variables.label = "SERVICE";

	// A one-expression getter: the engine may answer it without a frame, so
	// it covers that shortcut as well as an ordinary method.
	function readLabel() {
		return label;
	}

	function readExtra() {
		try {
			return extra;
		} catch ( any e ) {
			return "UNDEFINED";
		}
	}

	// The issue's case: a script `loop query` in a method, writing back with
	// a bare `currentRow`, over a query passed in by reference.
	function translate( required query q ) {
		loop query="arguments.q" {
			querySetCell( arguments.q, "label", uCase( label ) & currentRow, currentRow );
		}
		return arguments.q;
	}

	// Loops over the query it is given (the caller's, by reference).
	function rowsOf( required query q ) {
		var out = "";
		loop query="arguments.q" {
			out &= currentRow;
		}
		return out;
	}

	// Loops in a method and reports what the loop left on the component.
	function leaves() {
		var q = queryNew( "label,extra", "varchar,varchar", [ [ "a", "e" ] ] );
		var seen = "";
		loop query="q" {
			seen &= label & extra & currentRow;
		}
		cfloop( query="q" ) {
			seen &= label & extra & currentRow;
		}
		return seen & "|" & variables.label & "|"
			& structKeyExists( variables, "extra" ) & structKeyExists( variables, "currentRow" );
	}
}
