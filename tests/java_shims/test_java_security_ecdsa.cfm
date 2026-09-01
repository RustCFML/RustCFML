<cfscript>
// ECDSA over the java.security shims — the ES256/384/512 half of the JWT
// surface (java.security.KeyFactory.getInstance("EC"), Signature
// SHA*withECDSA, KeyPairGenerator + ECGenParameterSpec).
//
// EVERY key and signature below was produced by OPENSSL, not by this engine.
// A self-round-trip (sign here, verify here) passes even when the scheme is
// wrong end-to-end, so the anchor has to come from outside: these are real
// P-256/P-384/P-521 keys and real DER signatures over a fixed payload, and the
// test asserts we agree with them. Reproduce with:
//
//   openssl ecparam -genkey -name prime256v1 -noout -out raw.pem
//   openssl pkcs8 -topk8 -nocrypt -in raw.pem -out priv.pem
//   openssl ec -in raw.pem -pubout -out pub.pem
//   printf '<payload>' > msg.txt
//   openssl dgst -sha256 -sign priv.pem -out p256.sig msg.txt
//
// Runs on Lucee too, against the real JDK — which is the point.

suiteBegin( "java.security ECDSA (EC keys, SHA*withECDSA)" );

ecPayload = "eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJydXN0Y2ZtbCJ9";

ecVectors = [
	  {
		  curve  = "secp256r1"
		, algo   = "SHA256withECDSA"
		, priv   = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgujhgkB5DOksURRvDeWP6fatcRumWMzvEl5rmrELYjxuhRANCAAS0+x4JOX5Dirv/xf4tWFojRXbMkrZF4eKtLX/hbx3ZWtILWnlXgb4n6KEWtmxg93zba4N7ozPKD1FhGtiVUpEj"
		, pub    = "MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEtPseCTl+Q4q7/8X+LVhaI0V2zJK2ReHirS1/4W8d2VrSC1p5V4G+J+ihFrZsYPd822uDe6Mzyg9RYRrYlVKRIw=="
		, sig    = "MEUCIHIFfsIwN3ZzsWcrmzsIq5f735Ob3nasCNxU6syR2KQsAiEAnRUnxLUi7ldH++Aqw1pqh4LxUm1DPIji00K6SssnFpc="
	  }
	, {
		  curve  = "secp384r1"
		, algo   = "SHA384withECDSA"
		, priv   = "MIG2AgEAMBAGByqGSM49AgEGBSuBBAAiBIGeMIGbAgEBBDBjMvobN6F+ULaZujvfm6KKv7qKALh44lI7YlcY92A8g0Mdapx8QRaWLyUOx987A5GhZANiAARPtys9/aKLQzUuTz7bT0kGqxc9THc0WtldmyRpj8YgOZ6HTAluPVUrNAgH3m5OqqynEdFRmyQUPvLlga9gKxtB4R1qvQ3xk5sbGsZ2ape4o6Z8lixquuiv0UHB+uy77hk="
		, pub    = "MHYwEAYHKoZIzj0CAQYFK4EEACIDYgAET7crPf2ii0M1Lk8+209JBqsXPUx3NFrZXZskaY/GIDmeh0wJbj1VKzQIB95uTqqspxHRUZskFD7y5YGvYCsbQeEdar0N8ZObGxrGdmqXuKOmfJYsarror9FBwfrsu+4Z"
		, sig    = "MGUCMCqS1Zl4MAcQJ3Gy3lyyXsCfphnvtDpbKRao4V0F+lRxOY9Zy/o7AWBsmQT48pLxoQIxAPrsFfRNkNivZfKTh+ndTiRja3wS5GF+6whw9txwWZM0T15n5x36nfyR0wJTMEdFSg=="
	  }
	, {
		  curve  = "secp521r1"
		, algo   = "SHA512withECDSA"
		, priv   = "MIHtAgEAMBAGByqGSM49AgEGBSuBBAAjBIHVMIHSAgEBBEE5h06S8DRVVC/8dEjYzffOL0ibRIIggUCLCTCMqMyK4qLLMF8FztqDpzFSlP+LDbiq3zjBCZfkDzX4dJx5fmSHAqGBiQOBhgAEAR3HrBJCPxXmhAc1L6I7ZZk2Yz7L+iH7aX2gDv4Y4+IFY0UkLIxh3NsDP8MMD98WrkkgZJj6OLble1gGGMUI6WNcAD6S1U6Q6jH0XevAhhQBhhe9z2+XK9wGKPRarJnD2ZPnJ+JlaUPv5O4qO8VlO1fnH8Uj4T6W8iD32wjdM9dpQYw1"
		, pub    = "MIGbMBAGByqGSM49AgEGBSuBBAAjA4GGAAQBHcesEkI/FeaEBzUvojtlmTZjPsv6IftpfaAO/hjj4gVjRSQsjGHc2wM/wwwP3xauSSBkmPo4tuV7WAYYxQjpY1wAPpLVTpDqMfRd68CGFAGGF73Pb5cr3AYo9FqsmcPZk+cn4mVpQ+/k7io7xWU7V+cfxSPhPpbyIPfbCN0z12lBjDU="
		, sig    = "MIGHAkFjSfzCnR6BNlsZ2U5Q7pTqcksrvTerB91+aKy+KhuvS7DFxaMNkw+rHDOvhFzsTRbtkAp6jZMkG2VUhIth9ns+EQJCAYWU6Fi2C32L2bW6Itxn1ewtuHBRv2qriitOcM1A3sa1xwInrtu6zpdGxQU0JLDB4TA8kyChnann4Kk9Yfg8U9UC"
	  }
];

ecKeyFactory = CreateObject( "java", "java.security.KeyFactory" ).getInstance( "EC" );
assert( "KeyFactory.getInstance('EC').getAlgorithm()", ecKeyFactory.getAlgorithm(), "EC" );

for ( ecVector in ecVectors ) {
	ecLabel = ecVector.curve;

	// ── the keys load, and are what Java calls them ──────────────────────
	ecPublic = ecKeyFactory.generatePublic(
		CreateObject( "java", "java.security.spec.X509EncodedKeySpec" ).init( ToBinary( ecVector.pub ) )
	);
	ecPrivate = ecKeyFactory.generatePrivate(
		CreateObject( "java", "java.security.spec.PKCS8EncodedKeySpec" ).init( ToBinary( ecVector.priv ) )
	);
	assertTrue( "#ecLabel#: generatePublic gives a java.security.PublicKey" , IsInstanceOf( ecPublic , "java.security.PublicKey"  ) );
	assertTrue( "#ecLabel#: generatePrivate gives a java.security.PrivateKey", IsInstanceOf( ecPrivate, "java.security.PrivateKey" ) );
	assert( "#ecLabel#: public key algorithm" , ecPublic.getAlgorithm() , "EC" );
	assert( "#ecLabel#: private key algorithm", ecPrivate.getAlgorithm(), "EC" );

	// getEncoded() must give back the SAME encoding it was built from —
	// what the vendored libraries re-PEM for storage.
	assert( "#ecLabel#: public getEncoded round-trips" , ToBase64( ecPublic.getEncoded()  ), ecVector.pub  );
	assert( "#ecLabel#: private getEncoded round-trips", ToBase64( ecPrivate.getEncoded() ), ecVector.priv );

	// ── an OPENSSL signature verifies here ──────────────────────────────
	ecVerifier = CreateObject( "java", "java.security.Signature" ).getInstance( ecVector.algo );
	ecVerifier.initVerify( ecPublic );
	ecVerifier.update( ecPayload.getBytes() );
	assertTrue( "#ecLabel#: openssl's signature verifies", ecVerifier.verify( ToBinary( ecVector.sig ) ) );

	// ── a tampered payload does NOT ─────────────────────────────────────
	ecVerifier = CreateObject( "java", "java.security.Signature" ).getInstance( ecVector.algo );
	ecVerifier.initVerify( ecPublic );
	ecVerifier.update( "#ecPayload#tampered".getBytes() );
	assertFalse( "#ecLabel#: a tampered payload does not verify", ecVerifier.verify( ToBinary( ecVector.sig ) ) );

	// ── our own signature verifies against openssl's PUBLIC key ─────────
	// Signing with the private half openssl generated, verifying with the
	// public half openssl generated: agreement on both encodings at once.
	ecSigner = CreateObject( "java", "java.security.Signature" ).getInstance( ecVector.algo );
	ecSigner.initSign( ecPrivate );
	ecSigner.update( ecPayload.getBytes() );
	ecOurSig = ecSigner.sign();

	// Java hands back a DER SEQUENCE (0x30), not raw r||s — libraries that
	// want the JOSE form re-encode it themselves, and would mis-read the other.
	assert( "#ecLabel#: sign() returns a DER SEQUENCE", Left( ToBase64( ecOurSig ), 1 ), "M" );

	ecVerifier = CreateObject( "java", "java.security.Signature" ).getInstance( ecVector.algo );
	ecVerifier.initVerify( ecPublic );
	ecVerifier.update( ecPayload.getBytes() );
	assertTrue( "#ecLabel#: our signature verifies under openssl's public key", ecVerifier.verify( ecOurSig ) );
}

// ── an EC public key must not be usable with an RSA algorithm ────────────
// The cipher half of the algorithm name picks the scheme; crossing them over
// is the failure mode that silently "works" if only the digest is honoured.
// The JDK refuses at initVerify(), not at verify() — probed — so the caller
// sees an InvalidKeyException rather than a quiet `false` a page later.
ecCrossedKey = ecKeyFactory.generatePublic(
	CreateObject( "java", "java.security.spec.X509EncodedKeySpec" ).init( ToBinary( ecVectors[ 1 ].pub ) )
);
assertThrows( "an EC key is refused by a SHA256withRSA Signature, at initVerify", function() {
	CreateObject( "java", "java.security.Signature" ).getInstance( "SHA256withRSA" ).initVerify( ecCrossedKey );
} );

// An EC KeyFactory refuses an RSA key spec, too.
assertThrows( "an EC KeyFactory refuses an RSAPublicKeySpec", function() {
	ecKeyFactory.generatePublic( CreateObject( "java", "java.security.spec.RSAPublicKeySpec" ).init(
		  CreateObject( "java", "java.math.BigInteger" ).init( "17" )
		, CreateObject( "java", "java.math.BigInteger" ).init( "3" )
	) );
} );

// ── KeyPairGenerator + ECGenParameterSpec ───────────────────────────────
ecGen = CreateObject( "java", "java.security.KeyPairGenerator" ).getInstance( "EC" );
ecGen.initialize( CreateObject( "java", "java.security.spec.ECGenParameterSpec" ).init( "secp256r1" ) );
ecPair = ecGen.generateKeyPair();

assertTrue( "generated private key is a java.security.PrivateKey", IsInstanceOf( ecPair.getPrivate(), "java.security.PrivateKey" ) );
assertTrue( "generated public key is a java.security.PublicKey" , IsInstanceOf( ecPair.getPublic() , "java.security.PublicKey"  ) );
assert( "generated key algorithm", ecPair.getPrivate().getAlgorithm(), "EC" );

// The ENCODED LENGTHS are part of the contract, not an implementation detail:
// the JDK omits both optional fields of the inner ECPrivateKey (the curve
// parameters and the public point) that OpenSSL fills in, so its PKCS#8 is
// 67/80/98 bytes rather than 138/185/241. Real CFML libraries gate on exactly
// these numbers — the cfsignatures module Preside's ReadyIntelligence
// extension bundles rejects any EC signing key of another length — so emitting
// the OpenSSL shape makes a key we just generated fail its own validation.
ecLengths = [
	  { curve = "secp256r1", priv = 67, pub =  91 }
	, { curve = "secp384r1", priv = 80, pub = 120 }
	, { curve = "secp521r1", priv = 98, pub = 158 }
];
for ( ecLength in ecLengths ) {
	ecGen = CreateObject( "java", "java.security.KeyPairGenerator" ).getInstance( "EC" );
	ecGen.initialize( CreateObject( "java", "java.security.spec.ECGenParameterSpec" ).init( ecLength.curve ) );
	ecSized = ecGen.generateKeyPair();
	assert( "#ecLength.curve#: PKCS##8 private key is the JDK's length", ArrayLen( ecSized.getPrivate().getEncoded() ), ecLength.priv );
	assert( "#ecLength.curve#: X.509 public key is the JDK's length"   , ArrayLen( ecSized.getPublic().getEncoded()  ), ecLength.pub  );
}

// A generated pair must round-trip through the PEM/base64 path the callers
// use — i.e. the encodings really are PKCS#8 and X.509, not our own shapes.
ecReloadedPrivate = ecKeyFactory.generatePrivate(
	CreateObject( "java", "java.security.spec.PKCS8EncodedKeySpec" ).init( ecPair.getPrivate().getEncoded() )
);
ecReloadedPublic = ecKeyFactory.generatePublic(
	CreateObject( "java", "java.security.spec.X509EncodedKeySpec" ).init( ecPair.getPublic().getEncoded() )
);

ecSigner = CreateObject( "java", "java.security.Signature" ).getInstance( "SHA256withECDSA" );
ecSigner.initSign( ecReloadedPrivate );
ecSigner.update( ecPayload.getBytes() );
ecGenSig = ecSigner.sign();

ecVerifier = CreateObject( "java", "java.security.Signature" ).getInstance( "SHA256withECDSA" );
ecVerifier.initVerify( ecReloadedPublic );
ecVerifier.update( ecPayload.getBytes() );
assertTrue( "a generated key pair signs and verifies after a PKCS8/X509 round trip", ecVerifier.verify( ecGenSig ) );

// An EC generator left uninitialised defaults to P-384, NOT P-256 — probed
// against a real JDK, where the default EC key size is 384 bits.
ecDefaultPair = CreateObject( "java", "java.security.KeyPairGenerator" ).getInstance( "EC" ).generateKeyPair();
assert( "an uninitialised EC generator defaults to P-384", ArrayLen( ecDefaultPair.getPrivate().getEncoded() ), 80 );

// ── unknown curves and algorithms fail LOUDLY ───────────────────────────
// secp256k1 is a real curve we do not implement; answering it with P-256
// would be the worst possible outcome. The spec object itself is an inert
// name holder — it constructs fine, and initialize() is where the JDK
// refuses the curve.
ecBadSpec = CreateObject( "java", "java.security.spec.ECGenParameterSpec" ).init( "secp256k1" );
assert( "ECGenParameterSpec keeps the name it was given", ecBadSpec.getName(), "secp256k1" );
assertThrows( "an unsupported curve is refused at initialize()", function() {
	CreateObject( "java", "java.security.KeyPairGenerator" ).getInstance( "EC" ).initialize( ecBadSpec );
} );
assertThrows( "an unknown signature algorithm throws", function() {
	CreateObject( "java", "java.security.Signature" ).getInstance( "SHA256withNOPE" );
} );
// "ECDSA" names the CIPHER, not a key algorithm: the JDK's KeyFactory and
// KeyPairGenerator both reject it, and accepting it here would let code pass
// on RustCFML and fail everywhere else.
assertThrows( "KeyFactory.getInstance('ECDSA') throws", function() {
	CreateObject( "java", "java.security.KeyFactory" ).getInstance( "ECDSA" );
} );
assertThrows( "KeyPairGenerator.getInstance('ECDSA') throws", function() {
	CreateObject( "java", "java.security.KeyPairGenerator" ).getInstance( "ECDSA" );
} );

suiteEnd();
</cfscript>
