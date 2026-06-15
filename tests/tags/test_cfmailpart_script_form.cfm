<cfscript>
suiteBegin("Tags: cfmailpart script-statement form parses inside a script cfmail block");

// Background: inside a script cfmail(){} block, cfmailpart is callable as a
// script statement to declare a multipart (text + html) body — the same as the
// <cfmailpart> tag inside <cfmail>. Lucee/Adobe CF/BoxLang PARSE it (the
// function below compiles and is defined). RustCFML 0.161.0 fails to PARSE the
// script-form cfmailpart:
//   "Parse error: Expected RBrace, found Semicolon"
//
//   function buildMultipart() {
//       cfmail(to="a@x.com", from="b@x.com", subject="S") {
//           cfmailpart(type="text") { writeOutput("text part"); }
//           cfmailpart(type="html") { writeOutput("<b>html part</b>"); }
//       }
//   }
//
// The test exercises PARSE-ABILITY only (the function is defined, never called)
// so it needs no SMTP server. Because the gap is a PARSE error (not runtime),
// it aborts the whole file at compile time and cannot be caught — so this test
// is filed UNREGISTERED in tests/runner.cfm (registering it would abort the
// entire suite at the include). Register it once the parse support lands.
//
// Why it matters for Wheels: vendor/wheels/Global.cfc $mail() emits
// cfmailpart(attributeCollection=local.i){...} and cfmailparam(attributeCollection=local.i)
// in script form, so every multipart Wheels email (the default text+html
// mailer output) fails to COMPILE on RustCFML. (Plain single-part cfmail works
// — RustCFML implements cfmail end-to-end with a spool; only the script-form
// cfmailpart/cfmailparam sub-statements are unparsed.)

function cmpsfBuildMultipart() {
    cfmail(to = "a@example.com", from = "b@example.com", subject = "parts test") {
        cfmailpart(type = "text") { writeOutput("text-part-body"); }
        cfmailpart(type = "html") { writeOutput("<b>html-part-body</b>"); }
    }
    return "defined";
}

// Reaching here means the file PARSED (on RustCFML it parse-aborts before this).
assertTrue("script-form cfmailpart inside cfmail compiles (function is defined)",
    isCustomFunction(cmpsfBuildMultipart));

suiteEnd();
</cfscript>
