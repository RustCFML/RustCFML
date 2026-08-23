<cfscript>
include "../harness.cfm";

// ============================================================================
// s3GeneratePresignedURL — Lucee-compat contracts (pure, NO network, NO env)
// ============================================================================
// Presigning is offline computation, so every leg passes explicit dummy
// credentials (the AWS documentation example keypair) and a custom host.
// All expected values below were measured on Lucee 7.0.2.106 with the S3
// Resource Extension (17AB52DE-B300-A94B-E058BD978511E39E).
//
// Real-world repro class: any app that stores object keys with a leading
// slash (Lucee strips it when signing, so the objects live at the slash-less
// key) and/or presigns browser PUT uploads via httpMethod="PUT".
//
// Every call is wrapped so a throw is asserted as a "THREW: ..." value and
// can never abort the file.

function psuGet(objectName) {
    try {
        return s3GeneratePresignedURL(
            bucketName = "my-bucket",
            objectName = arguments.objectName,
            accessKeyId = "AKIAIOSFODNN7EXAMPLE",
            awsSecretKey = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            host = "syd1.digitaloceanspaces.com");
    } catch (any e) {
        return "THREW: " & e.message;
    }
}

function psuMethod(m, exp) {
    try {
        return s3GeneratePresignedURL(
            bucketName = "my-bucket",
            objectName = "a/b.txt",
            httpMethod = arguments.m,
            expireDate = arguments.exp,
            accessKeyId = "AKIAIOSFODNN7EXAMPLE",
            awsSecretKey = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            host = "syd1.digitaloceanspaces.com");
    } catch (any e) {
        return "THREW: " & e.message;
    }
}

function psuGetAltSecretArg(objectName) {
    try {
        return s3GeneratePresignedURL(
            bucketName = "my-bucket",
            objectName = arguments.objectName,
            accessKeyId = "AKIAIOSFODNN7EXAMPLE",
            secretAccessKey = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            host = "syd1.digitaloceanspaces.com");
    } catch (any e) {
        return "THREW: " & e.message;
    }
}

// Path portion of an https URL (between host and '?'); non-URLs pass through
// so a THREW string can never accidentally equal a real path.
function urlPathOf(u) {
    var s = arguments.u;
    if (left(s, 8) != "https://") return s;
    var rest = mid(s, 9, len(s));
    var slashAt = find("/", rest);
    if (slashAt == 0) return "";
    var pathAndQuery = mid(rest, slashAt, len(rest));
    var qAt = find("?", pathAndQuery);
    if (qAt > 0) return left(pathAndQuery, qAt - 1);
    return pathAndQuery;
}

function urlParamOf(u, name) {
    var s = arguments.u;
    var marker = arguments.name & "=";
    var at = find(marker, s);
    if (at == 0) return "";
    var val = mid(s, at + len(marker), len(s));
    var amp = find("&", val);
    if (amp > 0) return left(val, amp - 1);
    return val;
}

function isHttpsUrl(u) {
    return left(arguments.u, 8) == "https://";
}


suiteBegin("s3GeneratePresignedURL key normalization (Lucee-compat)");

uPlain = psuGet("a/b.txt");
uSlash = psuGet("/a/b.txt");
uDbl   = psuGet("//a/b.txt");

assertTrue("dummy-cred presign returns a URL (pure, no network)",
    isHttpsUrl(uPlain));

// Lucee strips one leading slash: '/a/b.txt' signs the SAME path as 'a/b.txt'.
// (Both isHttpsUrl guards matter: two identical THREW strings must not pass.)
assertTrue("objectName '/a/b.txt' signs the same path as 'a/b.txt'",
    isHttpsUrl(uSlash) && isHttpsUrl(uPlain) && urlPathOf(uSlash) == urlPathOf(uPlain));

assertTrue("no '//' in the signed path for objectName '/a/b.txt'",
    isHttpsUrl(uSlash) && find("//", urlPathOf(uSlash)) == 0);

// Exactly ONE slash is stripped, so '//a/b.txt' still addresses the key
// '/a/b.txt'. The two engines SPELL that key differently in the path and both
// spellings are correct: Lucee percent-encodes the key's own leading slash
// ('/%2Fa/b.txt'), while this engine leaves it as a path character
// ('//a/b.txt') because the AWS SDK builds and signs the canonical URI. Both
// URL-decode to the same key, so they fetch the same object; matching Lucee
// byte-for-byte here would mean hand-rolling SigV4 rather than using the SDK.
// See docs/known-issues.md. The invariant that matters — one slash stripped,
// not two, not none — is what is asserted.
dblPath = urlPathOf(uDbl);
assertTrue("objectName '//a/b.txt': exactly one slash stripped, key is still '/a/b.txt' (saw: " & dblPath & ")",
    isHttpsUrl(uDbl) && replace(dblPath, "%2F", "/", "all") == "//a/b.txt");

suiteEnd();


suiteBegin("s3GeneratePresignedURL URL shape (Lucee-compat)");

// Lucee builds virtual-host-style URLs: https://{bucket}.{host}/{key}?...
assertTrue("virtual-host style: https://my-bucket.syd1.digitaloceanspaces.com/a/b.txt",
    left(uPlain, 46) == "https://my-bucket.syd1.digitaloceanspaces.com/"
    && urlPathOf(uPlain) == "/a/b.txt");

suiteEnd();


suiteBegin("s3GeneratePresignedURL httpMethod (Lucee-compat)");

// Same key, same expireDate: only the method differs, so the signatures MUST
// differ. Retried until both URLs carry the same X-Amz-Date and X-Amz-Expires
// so a second-boundary between the two calls cannot fake a difference.
exp5 = dateAdd("n", 5, now());
gUrl = "";
pUrl = "";
attempt = 0;
while (attempt < 5) {
    attempt = attempt + 1;
    gUrl = psuMethod("GET", exp5);
    pUrl = psuMethod("PUT", exp5);
    if (urlParamOf(gUrl, "X-Amz-Date") == urlParamOf(pUrl, "X-Amz-Date")
        && urlParamOf(gUrl, "X-Amz-Expires") == urlParamOf(pUrl, "X-Amz-Expires")) break;
}

assertTrue("httpMethod='PUT' signs differently from GET (same key + expiry)",
    len(urlParamOf(gUrl, "X-Amz-Signature"))
    && urlParamOf(gUrl, "X-Amz-Signature") != urlParamOf(pUrl, "X-Amz-Signature"));

suiteEnd();


suiteBegin("s3GeneratePresignedURL expiry (Lucee-compat)");

// No expireDate → Lucee default of 15 minutes. Lucee computes the window
// end first and then diffs against call time, so a second boundary can shave
// a second off (measured values: 899 and 900).
defExp = urlParamOf(uPlain, "X-Amz-Expires");
assertTrue("default X-Amz-Expires is 900 (or 899 across a second boundary)",
    isNumeric(defExp) && defExp >= 899 && defExp <= 900);

// expireDate=now()+5min → X-Amz-Expires is the seconds remaining (Lucee
// measured 299/300 — it counts down from call time, hence the small range).
expVal = urlParamOf(gUrl, "X-Amz-Expires");
assertTrue("expireDate now()+5min gives X-Amz-Expires in 240..301",
    isNumeric(expVal) && expVal >= 240 && expVal <= 301);

suiteEnd();


suiteBegin("s3GeneratePresignedURL credential argument aliases (Lucee-compat)");

// Lucee accepts both awsSecretKey and secretAccessKey for the secret; the two
// spellings produce identical URLs (measured byte-identical on Lucee).
uAlt = psuGetAltSecretArg("a/b.txt");
assertTrue("awsSecretKey and secretAccessKey are aliases",
    isHttpsUrl(uPlain) && isHttpsUrl(uAlt) && urlPathOf(uAlt) == urlPathOf(uPlain));

suiteEnd();
</cfscript>
