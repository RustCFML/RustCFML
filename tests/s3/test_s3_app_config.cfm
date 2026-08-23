<cfscript>
include "../harness.cfm";

// ============================================================================
// Application.cfc `this.s3` reaches the S3 builtins (GH #334)
// ============================================================================
// The s3*() builtins take only a positional value list, so there was nowhere
// for an application context to arrive and `this.s3` was reachable only by
// duplicating every value into process environment variables — which is
// per-process where `this.s3` is per-application, so a container could hold
// exactly one credential set.
//
// This suite is pure: presigning is offline computation, so the settings in
// tests/Application.cfc (a non-resolving host plus the AWS documentation
// example keypair) can be observed in the generated URL with no bucket, no
// network and no env var. Real-world repro: titan (Moopa) sets exactly these
// keys from env in Application.cfc and works on Lucee; here the first
// s3readBinary() fell through to default AWS us-east-1 with no credentials.

suiteBegin("Application.cfc this.s3 configures the S3 builtins (GH ##334)");

// NOTE: call with explicit named arguments, never argumentCollection. Lucee
// uppercases struct keys, and its own BIF signature lookup is not case-blind,
// so an argumentCollection call fails there with "missing required argument
// [bucketNameOrPath]" before any of this is exercised.

// No inline credentials at all — everything must come from this.s3.
try {
    fromApp = s3GeneratePresignedURL( bucketNameOrPath = "my-bucket", objectName = "a/b.txt" );
} catch ( any e ) {
    fromApp = "THREW: " & e.message;
}

// The cross-engine invariant: the host from this.s3 reached the S3 layer.
// Lucee proves it by failing to CONNECT to that host (it resolves the bucket
// location over the network before signing, where this engine presigns purely
// offline), so accept either the signed URL or an error naming the host —
// both show the setting was picked up, which is the whole point of #334.
assertTrue( "this.s3.host reaches the S3 layer (saw: " & left( fromApp, 70 ) & ")",
    find( "my-bucket.app-config-probe.example.com", fromApp ) GT 0 );

if ( left( fromApp, 8 ) == "https://" ) {
    assertTrue( "this.s3.accessKeyId is used to sign",
        find( "X-Amz-Credential=AKIAIOSFODNN7EXAMPLE", fromApp ) GT 0 );
    assertTrue( "the signature is present, so the secret resolved too",
        find( "X-Amz-Signature=", fromApp ) GT 0 );
} else {
    assertTrue( "signing legs skipped (this engine contacts the host before signing)", true );
    assertTrue( "signature leg skipped (this engine contacts the host before signing)", true );
}

// An inline host must still win over the application setting.
try {
    inlineWins = s3GeneratePresignedURL( bucketNameOrPath = "my-bucket", objectName = "a/b.txt",
                      accessKeyId = "AKIAIOSFODNN7EXAMPLE",
                      secretAccessKey = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
                      host = "syd1.digitaloceanspaces.com" );
} catch ( any e ) {
    inlineWins = "THREW: " & e.message;
}
assertTrue( "an inline host overrides this.s3.host (saw: " & left( inlineWins, 60 ) & ")",
    left( inlineWins, 45 ) == "https://my-bucket.syd1.digitaloceanspaces.com" );

suiteEnd();
</cfscript>
