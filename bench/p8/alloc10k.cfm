<cfscript>
lib = "/Users/alexskinner/Projects/Websites/readyintelligencewebsite/website/preside/system/services/security/antisamylib/";
pol = createObject( "java", "org.owasp.validator.html.Policy", [ lib & "antisamy-1.5.3.jar" ] ).getInstance( lib & "antisamy-preside-1.4.4.xml" );
san = createObject( "java", "org.owasp.validator.html.AntiSamy", [ lib & "antisamy-1.5.3.jar" ] );
para = '<p class="lead">Hello <strong>world</strong>, <a href="https://example.com/x?y=1">link</a> and <em>more</em> text here to pad things out a little bit.</p><ul><li>one</li><li>two <span style="color:red">red</span></li></ul><script>alert(1)</script><img src="javascript:bad()" onerror="x()" alt="a">';
h = ""; while (len(h) < 10240) { h &= para; }
function doClean(san, pol, h) { return san.scan(h, pol).getCleanHTML(); }
for (i=1;i<=50;i++) { out = doClean(san, pol, h); }
writeOutput("ok " & len(out));
</cfscript>
