<cfscript>
suiteBegin("Core: getPageContext() provides non-null request/response objects");

// ============================================================
// Background
// ============================================================
// The page context's servlet-bridge accessors are a load-bearing CFML
// surface: getPageContext().getRequest() / .getResponse() return live
// request/response objects on Lucee and Adobe CF in EVERY execution
// context — including CLI/task contexts, where Lucee synthesizes them
// (getRequestURL() returns "http://localhost/index.cfm" under a
// CommandBox task with no real HTTP request in sight).
//
// RustCFML 0.124.0 returns a non-null page-context STUB whose
// getRequest() and getResponse() are NULL, in both CLI and serve mode.
//
// Until v0.119.0 this was masked: a method call on a null receiver
// silently returned null, so chains like
//     GetPageContext().getRequest().getRequestURL()
// limped through as null. v0.119.0 (PR #94) correctly made null-receiver
// calls THROW — which turned the missing bridge into a request-killing
// error: Wheels calls exactly that chain while building request URLs
// (vendor/wheels/Global.cfc:2403), so every request on a Wheels app now
// 500s with "cannot call method [getRequestURL] on a null value".
// The fix that exposed it is right; the bridge underneath it is the gap.
// ============================================================

pgctxPc = getPageContext();
pgctxReqNull = true;
pgctxRespNull = true;
pgctxReqUrl = "(unavailable)";
if (!isNull(pgctxPc)) {
    pgctxReq = pgctxPc.getRequest();
    pgctxReqNull = isNull(pgctxReq);
    if (!pgctxReqNull) {
        try {
            pgctxReqUrl = toString(pgctxReq.getRequestURL());
        } catch (any pgctxE) {
            pgctxReqUrl = "(threw: " & pgctxE.message & ")";
        }
    }
    pgctxResp = pgctxPc.getResponse();
    pgctxRespNull = isNull(pgctxResp);
}

// CONTROL (passes on both engines): the page context itself exists.
assertFalse("CONTROL: getPageContext() returns an object", isNull(pgctxPc));

// --- the gap: the bridge objects must be non-null in every context ---
assertFalse("getPageContext().getRequest() returns a non-null request object",
    pgctxReqNull);
assertFalse("getPageContext().getResponse() returns a non-null response object",
    pgctxRespNull);
assertTrue("request.getRequestURL() yields a non-empty simple value",
    isSimpleValue(pgctxReqUrl) && len(pgctxReqUrl) && left(pgctxReqUrl, 1) != "(");

suiteEnd();
</cfscript>
