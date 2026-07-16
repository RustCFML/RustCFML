/**
 * Engine-bundled compatibility shim for Lucee's built-in `new Query()`
 * programmatic query builder (org.lucee.cfml.Query). Backed by queryExecute.
 *
 * Supports the public builder API that CFML code relies on:
 *   setSQL / getSQL, setDatasource, setName, setReturnType, setMaxRows,
 *   setAttributes, addParam, setParams, clearParams, and execute().
 * execute() returns a Result object exposing getResult() (the query) and
 * getPrefix() (the cfquery result metadata struct), matching Lucee.
 *
 * A user's own component named "query" on disk always shadows this shim
 * (the overlay only serves it when no real file exists at that path).
 */
component output=false {

	public any function init() {
		variables.sql        = "";
		variables.params     = [];
		variables.attributes = {};
		return this;
	}

	public any function setSQL( required string sql ) {
		variables.sql = arguments.sql;
		return this;
	}

	public string function getSQL() {
		return variables.sql ?: "";
	}

	public any function setDatasource( required string datasource ) {
		variables.attributes.datasource = arguments.datasource;
		return this;
	}

	public any function setName( required string name ) {
		variables.attributes.name = arguments.name;
		return this;
	}

	public any function setReturnType( required string returntype ) {
		variables.attributes.returntype = arguments.returntype;
		return this;
	}

	public any function setDbType( required string dbtype ) {
		variables.attributes.dbtype = arguments.dbtype;
		return this;
	}

	public string function getDbType() {
		return variables.attributes.dbtype ?: "";
	}

	public any function setMaxRows( required numeric maxrows ) {
		variables.attributes.maxrows = arguments.maxrows;
		return this;
	}

	public any function setAttributes() {
		structAppend( variables.attributes, arguments, true );
		return this;
	}

	public any function addParam() {
		arrayAppend( variables.params, duplicate( arguments ) );
		return this;
	}

	public any function setParams( required any params ) {
		if ( isArray( arguments.params ) ) {
			for ( var p in arguments.params ) {
				addParam( argumentCollection = p );
			}
		} else {
			for ( var key in arguments.params ) {
				var p = arguments.params[ key ];
				if ( !isStruct( p ) ) {
					p = { value = p };
				}
				p.name = key;
				addParam( argumentCollection = p );
			}
		}
		return this;
	}

	public any function clearParams() {
		variables.params = [];
		return this;
	}

	/**
	 * Execute the accumulated statement. Any arguments are folded into the
	 * tag attributes (Lucee semantics); an `sql` argument overrides setSQL(),
	 * and a `params` argument is appended via setParams().
	 */
	public any function execute() {
		if ( structKeyExists( arguments, "sql" ) && len( arguments.sql ) ) {
			setSQL( arguments.sql );
		}

		var execArgs = duplicate( arguments );
		structDelete( execArgs, "sql" );
		structDelete( execArgs, "params" );
		structAppend( variables.attributes, execArgs, true );

		if ( structKeyExists( arguments, "params" ) ) {
			setParams( arguments.params );
		}

		var options = duplicate( variables.attributes );
		structDelete( options, "name" ); // not a queryExecute option

		var qResult = queryExecute(
			  sql     = variables.sql
			, params  = _buildParams()
			, options = options
		);

		var result = new Result();
		if ( !isNull( qResult ) ) {
			result.setResult( qResult );
			if ( isQuery( qResult ) ) {
				result.setPrefix( {
					  recordcount = qResult.recordCount
					, columnList  = qResult.columnList
				} );
			}
		}
		return result;
	}

	/**
	 * Build the queryExecute params payload: a named struct when any param
	 * carries a `name`, otherwise a positional array (matching the `?`/`:name`
	 * placeholders in the SQL). Empty struct when there are no params.
	 */
	private any function _buildParams() {
		if ( !arrayLen( variables.params ) ) {
			return {};
		}

		var hasNamed = false;
		for ( var p in variables.params ) {
			if ( structKeyExists( p, "name" ) ) {
				hasNamed = true;
				break;
			}
		}

		if ( hasNamed ) {
			var named = {};
			for ( var p in variables.params ) {
				var copy = duplicate( p );
				structDelete( copy, "name" );
				named[ p.name ] = copy;
			}
			return named;
		}

		var positional = [];
		for ( var p in variables.params ) {
			arrayAppend( positional, p );
		}
		return positional;
	}
}
