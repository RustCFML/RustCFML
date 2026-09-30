<cfscript>
suiteBegin("org.pixl8.s3storageprovider.Service (preside-ext-s3-storage-provider)");

// The Java class preside-ext-s3-storage-provider constructs. RustCFML shims it
// natively; on Lucee this file drives the extension's real jar, so the two
// engines must agree on every value below. Needs a live S3 endpoint, so it is
// configured from the environment and skips when that is absent (CI):
//
//   S3SHIM_TEST_BUCKET, S3SHIM_TEST_REGION,
//   S3SHIM_TEST_ACCESS_KEY, S3SHIM_TEST_SECRET_KEY   — an existing, empty-ish bucket
//   S3SHIM_TEST_JAR                                   — Lucee only: path to
//                                                       lib/s3storageprovider-1.0.0.jar
//   AWS_ENDPOINT_URL (+ RUSTCFML_S3_PATH_STYLE=true)  — RustCFML only, for MinIO
//
// The jar ignores AWS_ENDPOINT_URL (AWS SDK 2.20.96), so on Lucee the endpoint
// is redirected in the JVM instead — see s3shim.md, "Reference first".

env = createObject("java", "java.lang.System").getenv();
function envOr(name, dflt = "") { var v = env.get(arguments.name); return isNull(v) ? arguments.dflt : v; }
cfg = {
	  bucket    = envOr("S3SHIM_TEST_BUCKET")
	, region    = envOr("S3SHIM_TEST_REGION", "eu-west-2")
	, accessKey = envOr("S3SHIM_TEST_ACCESS_KEY")
	, secretKey = envOr("S3SHIM_TEST_SECRET_KEY")
	, jar       = envOr("S3SHIM_TEST_JAR")
};
// The extension passes its OSGi bundle name as the third argument; Lucee
// without the bundle installed needs the jar path there instead.
loadArg = isRustCFML() ? "org.pixl8.s3storageprovider" : cfg.jar;
enabled = len(cfg.bucket) && len(cfg.accessKey) && ( isRustCFML() || len(cfg.jar) );

function newService(region = cfg.region, bucket = cfg.bucket, accessKey = cfg.accessKey, secretKey = cfg.secretKey) {
	return createObject("java", "org.pixl8.s3storageprovider.Service", loadArg)
		.init(arguments.region, arguments.bucket, arguments.accessKey, arguments.secretKey);
}
function errorOf(required any f) {
	var caught = { type = "", message = "" };
	try {
		arguments.f();
	} catch (any e) {
		caught = { type = e.type, message = e.message };
	}
	return caught;
}
function rowsOf(required query q) {
	var out = [];
	for (var r in q) { arrayAppend(out, r.name & " @ " & r.path & " (" & r.size & ")"); }
	return out;
}
function fetch(required string href) {
	var r = {};
	cfhttp(url = arguments.href, method = "GET", result = "r", timeout = 30);
	return r;
}

// The body is a function: Lucee 7.1 emits unverifiable bytecode ("Bad local
// variable type") for a page-level try/finally this size.
function runShimTests() {
	var e = {}; var got = ""; var dest = ""; var info = {}; var q = ""; var prefixPath = "";
	var i = 0; var r = {}; var signed = "";
	// ── construction ────────────────────────────────────────────────────────
	assertTrue("init does no network I/O: bogus credentials construct fine",
		isObject(newService(accessKey = "bogus", secretKey = "bogus")) || isStruct(newService(accessKey = "bogus", secretKey = "bogus")));
	e = errorOf(function() { newService(region = ""); });
	assert("a blank region throws IllegalArgumentException", e.type, "java.lang.IllegalArgumentException");
	assert("with the SDK's message", e.message, "region must not be blank or empty.");

	// ── access checks: booleans, never throw ────────────────────────────────
	assert("checkS3Access with good credentials", svc.checkS3Access(), true);
	assert("checkS3Access with bad credentials is false, not an error",
		newService(accessKey = "bogus", secretKey = "bogus").checkS3Access(), false);
	assert("checkBucketAccess on the bucket", svc.checkBucketAccess(), true);
	assert("checkBucketAccess on a missing bucket is false, not an error",
		newService(bucket = "nobucket-xyz").checkBucketAccess(), false);
	assert("checkBucketRegion when the region matches", svc.checkBucketRegion(), true);
	e = errorOf(function() { newService(bucket = "nobucket-xyz").checkBucketRegion(); });
	assert("checkBucketRegion on a missing bucket throws NoSuchBucketException",
		e.type, "software.amazon.awssdk.services.s3.model.NoSuchBucketException");
	assertTrue("carrying the S3 message", e.message contains "The specified bucket does not exist");

	// ── put / get ───────────────────────────────────────────────────────────
	svc.putObject(run & "a/b.txt", bytes, "text/plain", "inline", false, false);
	svc.putObject(run & "f.txt", localFile, "text/csv", 'attachment; filename="f.txt"', true, false);
	svc.putObject(run & "dir/", charsetDecode("", "utf-8"), "application/x-directory", "", false, false);
	svc.putObject(run & "sp ace+plus%pct/ünï.txt", bytes, "text/plain", "", false, false);

	got = svc.getObject(run & "a/b.txt");
	assertTrue("getObject(key) returns binary", isBinary(got));
	assert("getObject(key) round-trips the bytes", charsetEncode(got, "utf-8"), "hello world");
	assert("a file upload round-trips", charsetEncode(svc.getObject(run & "f.txt"), "utf-8"), "from file");
	assert("a key with spaces, +, % and non-ASCII round-trips",
		charsetEncode(svc.getObject(run & "sp ace+plus%pct/ünï.txt"), "utf-8"), "hello world");

	dest = getTempDirectory() & "s3shim_dl_" & createUUID() & ".txt";
	svc.getObject(run & "f.txt", dest);
	assert("getObject(key, path) writes the object to the file", fileRead(dest), "from file");
	e = errorOf(function() { svc.getObject(run & "f.txt", dest); });
	assert("getObject(key, path) refuses to overwrite an existing file",
		e.type, "software.amazon.awssdk.core.exception.SdkClientException");
	assertTrue("naming the file", e.message contains "Failed to read response into file");
	e = errorOf(function() { svc.getObject(run & "missing.txt"); });
	assert("getObject of a missing key throws NoSuchKeyException",
		e.type, "software.amazon.awssdk.services.s3.model.NoSuchKeyException");
	assertTrue("with the S3 message", e.message contains "The specified key does not exist");
	e = errorOf(function() { svc.getObject(run & "missing.txt", getTempDirectory() & createUUID()); });
	assert("getObject(key, path) of a missing key throws NoSuchKeyException too",
		e.type, "software.amazon.awssdk.services.s3.model.NoSuchKeyException");

	// ── getObjectInfo ───────────────────────────────────────────────────────
	info = svc.getObjectInfo(run & "a/b.txt");
	assert("getObjectInfo has size and lastmodified", listSort(structKeyList(info), "textnocase"), "lastmodified,size");
	assert("size is the content length", info.size, 11);
	assertTrue("lastmodified is a date", isDate(info.lastmodified));
	e = errorOf(function() { svc.getObjectInfo(run & "missing.txt"); });
	assert("getObjectInfo of a missing key throws NoSuchKeyException",
		e.type, "software.amazon.awssdk.services.s3.model.NoSuchKeyException");

	// ── listObjects ─────────────────────────────────────────────────────────
	q = svc.listObjects(run);
	assert("listObjects returns a query", isQuery(q), true);
	assert("with name, path, size and lastmodified", listSort(q.columnList, "textnocase"), "LASTMODIFIED,NAME,PATH,SIZE");
	prefixPath = "/" & left(run, len(run) - 1);
	assert("one row per object, folder marker included, in key order", arrayToList(rowsOf(q), " ; "),
		  "b.txt @ #prefixPath#/a (11) ; dir @ #prefixPath# (0) ; f.txt @ #prefixPath# (9) ; ünï.txt @ #prefixPath#/sp ace+plus%pct (11)");
	assertTrue("lastmodified cells are dates", isDate(q.lastmodified[1]));
	assert("an empty prefix match is an empty query", svc.listObjects(run & "nothing-here/").recordCount, 0);

	// More than one page (S3 returns at most 1,000 keys per ListObjectsV2 page).
	for (i = 1; i <= 1005; i++) { svc.putObject(run & "many/k" & numberFormat(i, "0000"), bytes, "text/plain", "", false, false); }
	assert("listObjects walks every page", svc.listObjects(run & "many/").recordCount, 1005);

	// ── moveObject replaces headers, ACL and storage class ──────────────────
	svc.moveObject(run & "a/b.txt", run & "moved b.txt", "text/html", "attachment", true, true);
	assert("moveObject removes the source", svc.listObjects(run & "a/").recordCount, 0);
	assert("and writes the target", charsetEncode(svc.getObject(run & "moved b.txt"), "utf-8"), "hello world");
	r = fetch(svc.getPresignedUrl(run & "moved b.txt", javaCast("long", 5)));
	assert("the presigned URL fetches the private object", r.status_code, 200);
	assertTrue("move REPLACED the content type", (r.responseHeader["Content-Type"] ?: "") contains "text/html");
	assert("move REPLACED the disposition", r.responseHeader["Content-Disposition"] ?: "", "attachment");
	assert("a trashed object is reduced-redundancy", r.responseHeader["x-amz-storage-class"] ?: "", "REDUCED_REDUNDANCY");
	r = fetch(svc.getPresignedUrl(run & "f.txt", 5));
	assert("put set the disposition", r.responseHeader["Content-Disposition"] ?: "", 'attachment; filename="f.txt"');
	assert("an untrashed object is STANDARD (no storage-class header)", r.responseHeader["x-amz-storage-class"] ?: "", "");

	e = errorOf(function() { svc.moveObject(run & "missing.txt", run & "x.txt", "text/plain", "", false, false); });
	assert("moving a missing source throws NoSuchKeyException",
		e.type, "software.amazon.awssdk.services.s3.model.NoSuchKeyException");
	svc.moveObject(run & "sp ace+plus%pct/ünï.txt", run & "moved/ünï 2.txt", "text/plain", "", false, false);
	assert("a key with special characters moves", charsetEncode(svc.getObject(run & "moved/ünï 2.txt"), "utf-8"), "hello world");

	// ── delete ──────────────────────────────────────────────────────────────
	svc.deleteObject(run & "f.txt");
	e = errorOf(function() { svc.getObjectInfo(run & "f.txt"); });
	assertTrue("deleteObject removes the object", len(e.type) > 0);
	assert("deleting a missing key succeeds", errorOf(function() { svc.deleteObject(run & "missing.txt"); }).type, "");

	// ── presigned URLs ──────────────────────────────────────────────────────
	signed = svc.getPresignedUrl(run & "moved b.txt", javaCast("long", 5));
	assertTrue("the URL carries a 5-minute expiry", signed contains "X-Amz-Expires=300");
	assertTrue("a 0-minute URL is allowed", len(svc.getPresignedUrl(run & "moved b.txt", javaCast("long", 0))) > 0);
	assertTrue("7 days is the limit", len(svc.getPresignedUrl(run & "moved b.txt", javaCast("long", 10080))) > 0);
	e = errorOf(function() { svc.getPresignedUrl(run & "moved b.txt", javaCast("long", 10081)); });
	assert("beyond 7 days throws", e.type, "software.amazon.awssdk.core.exception.SdkClientException");
	assertTrue("with the SigV4 message", e.message contains "valid for at most 7 days");

	// ── the object itself ───────────────────────────────────────────────────
	e = errorOf(function() { svc.frobnicate(); });
	assert("an unknown method throws NoSuchMethodException", e.type, "java.lang.NoSuchMethodException");
	var dumped = "";
	savecontent variable="dumped" { writeDump(svc); }
	assertFalse("writeDump does not show the secret key", findNoCase(cfg.secretKey, dumped) > 0);
	request._s3shimDest = dest;
}

function cleanUp() {
	try {
		for (var r in svc.listObjects(run)) {
			svc.deleteObject(mid(r.path, 2, 9999) & "/" & r.name);
		}
		svc.deleteObject(run & "dir/");
	} catch (any ignore) {}
	try { fileDelete(localFile); } catch (any ignore) {}
	try { if (fileExists(request._s3shimDest ?: "")) fileDelete(request._s3shimDest); } catch (any ignore) {}
}

if (enabled) {
	svc = newService();
	run = "shimtest-" & lCase(left(createUUID(), 8)) & "/";
	bytes = charsetDecode("hello world", "utf-8");
	localFile = getTempDirectory() & "s3shim_" & createUUID() & ".txt";
	fileWrite(localFile, "from file");

	// Clean up whether or not the tests threw. (Not try/finally: Lucee 7.1 emits
	// unverifiable bytecode for a try/catch nested in a page-level finally.)
	shimFailure = "";
	try {
		runShimTests();
	} catch (any e) {
		shimFailure = e;
	}
	cleanUp();
	if (!isSimpleValue(shimFailure)) {
		throw(object = shimFailure);
	}
} else {
	assertTrue("S3SHIM_TEST_* not configured — S3 storage-provider shim tests skipped", true);
}

suiteEnd();
</cfscript>
