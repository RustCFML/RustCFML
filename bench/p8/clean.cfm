<cfscript>
lib = "/Users/alexskinner/Projects/Websites/readyintelligencewebsite/website/preside/system/services/security/antisamylib/";
jars = [ lib & "antisamy-1.5.3.jar" ];
policy = createObject( "java", "org.owasp.validator.html.Policy", jars ).getInstance( lib & "antisamy-preside-1.4.4.xml" );
as = createObject( "java", "org.owasp.validator.html.AntiSamy", jars );
para = '<p class="lead">Hello <strong>world</strong>, <a href="https://example.com/x?y=1">link</a> and <em>more</em> text here to pad things out a little bit.</p><ul><li>one</li><li>two <span style="color:red">red</span></li></ul><script>alert(1)</script><img src="javascript:bad()" onerror="x()" alt="a">';
function frag(kb) { var s = ""; while (len(s) < kb*1024) { s &= para; } return s; }
function tm(label, html, n, sam, policy) { var t0 = getTickCount("nano"); var out = ""; for (var i=1;i<=n;i++) { out = sam.scan(html, policy).getCleanHTML(); } writeOutput(label & ": " & numberFormat((getTickCount("nano")-t0)/1000/n,"0") & " us/clean (in " & len(html) & " B, out " & len(out) & " B)" & chr(10)); }
tm("1KB ", frag(1), 200, sam, policy); tm("10KB", frag(10), 50, sam, policy); tm("50KB", frag(50), 10, sam, policy);
</cfscript>
