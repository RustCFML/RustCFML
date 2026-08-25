/**
 * Engine-bundled compatibility shim for Lucee's built-in `new Mail()`
 * (org.lucee.cfml.Mail, which extends org.lucee.cfml.HelperBase). Backed by the
 * same `__cfmail` machinery the <cfmail> tag lowers to.
 *
 * Lucee builds this out of `onMissingMethod` over the tag's declared attribute
 * set: every `setX(v)`/`getX()` pair maps to the tag attribute `X`, plus
 * addParam/addPart/clear* and `send()`, which invokes `<cfmail
 * attributeCollection=…>` with the accumulated parts and params. This mirrors
 * that behaviour, with the attribute names taken from Lucee's mail tag.
 *
 * A user's own component named "mail" on disk always shadows this shim (the
 * overlay only serves it when no real file exists at that path).
 */
component output=false accessors=false {

	public any function init() {
		variables.attributes = {};
		variables.params     = [];
		variables.parts      = [];
		setAttributes( argumentCollection = arguments );
		return this;
	}

	// ---- the HelperBase surface ------------------------------------------

	public any function setAttributes() {
		structAppend( variables.attributes, arguments, true );
		return this;
	}

	public struct function getAttributes() {
		return variables.attributes;
	}

	public any function clearAttributes() {
		variables.attributes = {};
		return this;
	}

	public any function addParam() {
		arrayAppend( variables.params, duplicate( arguments ) );
		return this;
	}

	public array function getParams() {
		return variables.params;
	}

	public any function clearParams() {
		variables.params = [];
		return this;
	}

	public any function addPart() {
		arrayAppend( variables.parts, duplicate( arguments ) );
		return this;
	}

	public array function getParts() {
		return variables.parts;
	}

	public any function clearParts() {
		variables.parts = [];
		return this;
	}

	public any function clear() {
		clearAttributes();
		clearParams();
		clearParts();
		return this;
	}

	public string function getTagName() {
		return "mail";
	}

	/**
	 * Lucee resolves setX/getX against the mail tag's declared attributes, plus
	 * `body` as an extra. An unknown name throws there, so it throws here too —
	 * silently accepting a misspelt setter would let a message go out missing
	 * the thing the caller asked for.
	 */
	public any function onMissingMethod( required string missingMethodName, required any missingMethodArguments ) {
		var prefix = lCase( left( arguments.missingMethodName, 3 ) );
		var attr   = lCase( mid( arguments.missingMethodName, 4, len( arguments.missingMethodName ) ) );

		if ( ( prefix EQ "set" OR prefix EQ "get" ) AND listFindNoCase( _supportedAttributes(), attr ) ) {
			if ( prefix EQ "get" ) {
				return structKeyExists( variables.attributes, attr ) ? variables.attributes[ attr ] : "";
			}
			variables.attributes[ attr ] = arguments.missingMethodArguments[ 1 ];
			return this;
		}

		throw(
			  message = "There is no method with the name #arguments.missingMethodName#"
			, type    = "expression"
		);
	}

	/**
	 * Send the accumulated message. Any arguments are folded into the
	 * attributes first, matching Lucee. Returns `this`, as Lucee's mail case
	 * does (the Result object is for the query/http/ftp cases).
	 */
	public any function send() {
		setAttributes( argumentCollection = arguments );

		var opts = duplicate( variables.attributes );
		if ( arrayLen( variables.params ) ) {
			opts.params = variables.params;
		}
		if ( arrayLen( variables.parts ) ) {
			opts.parts = variables.parts;
		}
		__cfmail( opts );
		return this;
	}

	/**
	 * The <cfmail> attribute set, as Lucee declares it. `body` is the extra
	 * Lucee allows past the tag's own attributes.
	 */
	private string function _supportedAttributes() {
		return "to,from,subject,cc,bcc,replyto,failto,body,type,charset,server,port,"
		     & "username,password,usessl,usetls,timeout,spoolenable,async,priority,"
		     & "wraptext,mailerid,remove,mimeattach,group,groupcasesensitive,"
		     & "startrow,maxrows,query,debug,sign,keystore,keystorepassword,"
		     & "keyalias,keypassword,encrypt,recipientcert";
	}
}
