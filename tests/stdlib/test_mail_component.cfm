<!--- GH #356 — Lucee's built-in script component `Mail` (`new Mail()`,
      org.lucee.cfml.Mail extending HelperBase). It did not exist here, and it
      was the last blocker for Preside email sending: its SMTP service provider
      builds every message through this API.

      Engine-bundled as a compat CFC, like `new Query()`. A user's own component
      named "mail" on disk still shadows it.

      This exercises the BUILDER surface only — `send()` needs a live SMTP
      server, so delivery is not asserted here. Measured against Lucee 7.1.0.204,
      whose Mail is built out of onMissingMethod over the <cfmail> attribute
      set. --->
<cfscript>
suiteBegin("stdlib: new Mail() built-in component (GH ##356)");

_m = new Mail();
assertTrue("new Mail() constructs", isObject(_m));

// setX/getX map onto the <cfmail> attribute of the same name and return `this`,
// so they chain.
_m.setTo( "a@x.com;b@y.com" );
_m.setFrom( "sender@z.com" );
_m.setSubject( "hello" );
assert("setTo/getTo round-trip", _m.getTo(), "a@x.com;b@y.com");
assert("setFrom/getFrom round-trip", _m.getFrom(), "sender@z.com");
assert("setSubject/getSubject round-trip", _m.getSubject(), "hello");
assert("an unset attribute reads back empty", _m.getUsername(), "");

_chained = _m.setCc( "c@x.com" );
assertTrue("a setter returns the instance, so calls chain", isObject(_chained));
assert("...and the chained value took", _m.getCc(), "c@x.com");

// The rest of the surface Preside's Smtp.cfc uses.
_m.setBCc( "d@x.com" );
_m.setReplyTo( "reply@x.com" );
_m.setFailTo( "bounce@x.com" );
_m.setServer( "127.0.0.1" );
_m.setPort( 2525 );
_m.setUsername( "u" );
_m.setPassword( "p" );
_m.setUseTls( false );
assert("bcc", _m.getBCc(), "d@x.com");
assert("replyTo", _m.getReplyTo(), "reply@x.com");
assert("failTo", _m.getFailTo(), "bounce@x.com");
assert("server", _m.getServer(), "127.0.0.1");
assert("port", _m.getPort(), 2525);

// addPart / addParam accumulate.
_m.addPart( type="text", body="plain" );
_m.addPart( type="html", body="<b>rich</b>" );
assert("addPart accumulates", arrayLen(_m.getParts()), 2);
_m.addParam( name="X-Mailer", value="Preside" );
assert("addParam accumulates", arrayLen(_m.getParams()), 1);

// clearParts / clearParams / clear.
_m.clearParts();
assert("clearParts empties the parts", arrayLen(_m.getParts()), 0);
_m.clearParams();
assert("clearParams empties the params", arrayLen(_m.getParams()), 0);
_m.clear();
assert("clear() also drops the attributes", _m.getTo(), "");

// An unknown setter throws rather than silently swallowing the value — a
// misspelt setter must not let a message go out missing what was asked for.
assertThrows("an unknown setter throws", function() {
	var m2 = new Mail();
	m2.setDefinitelyNotAMailAttribute( "x" );
});

suiteEnd();
</cfscript>
