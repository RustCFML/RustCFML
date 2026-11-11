<cfscript>
suiteBegin("java shim: javax.crypto.* and java.security.SecureRandom");

// The surface Preside's GoogleAuthenticator.cfc (TOTP two-factor auth) uses.
// Backed by the hmac(), generatePBKDFKey() and randomBytes() builtins.

// ---- javax.crypto.Mac against the RFC 4226 HOTP vectors -------------------
// Key is ASCII "12345678901234567890"; the published tokens for counters 0..9.
key = [];
for ( i = 1; i <= 20; i++ ) { arrayAppend( key, asc( mid( "12345678901234567890", i, 1 ) ) ); }

function hotp( required array key, required numeric counter ) {
	var spec   = CreateObject( "java", "javax.crypto.spec.SecretKeySpec" ).init( arguments.key, "HmacSHA1" );
	var mac    = CreateObject( "java", "javax.crypto.Mac" ).getInstance( spec.getAlgorithm() );
	var buffer = CreateObject( "java", "java.nio.ByteBuffer" ).allocate( 8 );

	mac.init( spec );
	buffer.putLong( arguments.counter );

	var h = mac.doFinal( buffer.array() );
	var t = h[ 20 ]; if ( t < 0 ) t += 256;
	var o = bitAnd( t, 15 ) + 1;

	var num = 0;
	for ( var k = 3; k >= 0; k-- ) {
		var b = h[ o + k ]; if ( b < 0 ) b += 256;
		num = bitOr( num, bitSHLN( b, ( 3 - k ) * 8 ) );
	}
	return numberFormat( bitAnd( num, 2147483647 ) % 1000000, "000000" );
}

expected = [ "755224","287082","359152","969429","338314","254676","287922","162583","399871","520489" ];
allMatch = true;
for ( i = 0; i <= 9; i++ ) {
	if ( hotp( key, i ) != expected[ i + 1 ] ) { allMatch = false; }
}
assertTrue( "all ten RFC 4226 HOTP vectors reproduce", allMatch );

// Mac.init() takes a Key OBJECT, not a byte array — reading it as text still
// produces a plausible-looking token, so only an external vector catches it.
assert( "getAlgorithm round-trips the requested spelling"
      , CreateObject( "java", "javax.crypto.Mac" ).getInstance( "HmacSHA256" ).getAlgorithm()
      , "HmacSHA256" );
assert( "getMacLength reports the digest size"
      , CreateObject( "java", "javax.crypto.Mac" ).getInstance( "HmacSHA256" ).getMacLength()
      , 32 );

noSuchAlg = "";
try { CreateObject( "java", "javax.crypto.Mac" ).getInstance( "HmacNoSuchThing" ); }
catch ( any e ) { noSuchAlg = e.type; }
assert( "an unknown Mac algorithm raises NoSuchAlgorithmException", noSuchAlg, "java.security.NoSuchAlgorithmException" );

uninit = "";
try { CreateObject( "java", "javax.crypto.Mac" ).getInstance( "HmacSHA1" ).doFinal( [ 1, 2 ] ); }
catch ( any e ) { uninit = e.type; }
assert( "doFinal before init raises IllegalStateException", uninit, "java.lang.IllegalStateException" );

// ---- java.security.SecureRandom ------------------------------------------
// nextBytes fills the CALLER'S array in place. If it did not, every derived key
// below would come from an all-zero salt.
salt = [];
for ( i = 1; i <= 16; i++ ) { arrayAppend( salt, 0 ); }
CreateObject( "java", "java.security.SecureRandom" ).init().nextBytes( salt );
assert( "nextBytes leaves the array the same length", arrayLen( salt ), 16 );
allZero = true;
for ( b in salt ) { if ( b != 0 ) { allZero = false; } }
assertFalse( "nextBytes actually wrote into the caller's array", allZero );

sr = CreateObject( "java", "java.security.SecureRandom" ).init();
assertTrue( "nextInt( bound ) stays in range", sr.nextInt( 10 ) >= 0 && sr.nextInt( 10 ) < 10 );

badBound = "";
try { sr.nextInt( 0 ); } catch ( any e ) { badBound = e.type; }
assert( "nextInt( 0 ) raises IllegalArgumentException", badBound, "java.lang.IllegalArgumentException" );

// The builtin behind it.
assert( "randomBytes returns the requested length", len( binaryEncode( randomBytes( 8 ), "hex" ) ), 16 );
assertTrue( "randomBytes returns binary", isBinary( randomBytes( 4 ) ) );
assertThrows( "randomBytes( 0 ) is rejected", function(){ randomBytes( 0 ); } );

// ---- javax.crypto.SecretKeyFactory / PBEKeySpec ---------------------------
fixedSalt = [];
for ( i = 1; i <= 16; i++ ) { arrayAppend( fixedSalt, i ); }

function derive( required string password, required array salt ) {
	var kf   = CreateObject( "java", "javax.crypto.SecretKeyFactory" ).getInstance( "PBKDF2WithHmacSHA1" );
	var spec = CreateObject( "java", "javax.crypto.spec.PBEKeySpec" ).init( password.toCharArray(), arguments.salt, 128, 80 );
	return kf.generateSecret( spec ).getEncoded();
}

k1 = derive( "correct horse", fixedSalt );
k2 = derive( "correct horse", fixedSalt );
assert( "a PBKDF2 key is 80 bits = 10 bytes", arrayLen( k1 ), 10 );
assert( "same password + same salt derives the same key", arrayToList( k1 ), arrayToList( k2 ) );
assertTrue( "a different password derives a different key"
          , arrayToList( derive( "wrong horse", fixedSalt ) ) != arrayToList( k1 ) );

// Byte-for-byte against the builtin the shim delegates to. generatePBKDFKey()
// takes the salt as raw bytes too, so the same array goes into both.
assert( "the shim and generatePBKDFKey() agree"
      , CreateObject( "java", "java.util.Base64" ).getEncoder().encodeToString( k1 )
      , generatePBKDFKey( "PBKDF2WithHmacSHA1", "correct horse", fixedSalt, 128, 80 ) );

badSpec = "";
try { CreateObject( "java", "javax.crypto.SecretKeyFactory" ).getInstance( "PBKDF2WithHmacSHA1" ).generateSecret( "not a spec" ); }
catch ( any e ) { badSpec = e.type; }
assert( "generateSecret without a PBEKeySpec raises InvalidKeySpecException", badSpec, "java.security.spec.InvalidKeySpecException" );

// ---- javax.crypto.KeyGenerator -------------------------------------------
// How CFML mints an HMAC key when it wants the key object (the cfsignatures
// module's Hmac.generateKeys(), which Preside uses to sign admin object pickers).
function generatedKey( required string algorithm, numeric bits=0 ) {
	var keyGen = CreateObject( "java", "javax.crypto.KeyGenerator" ).getInstance( arguments.algorithm );
	if ( arguments.bits > 0 ) { keyGen.init( arguments.bits ); }
	return keyGen.generateKey();
}

assert( "an HmacSHA256 key is 32 bytes", arrayLen( generatedKey( "HmacSHA256", 256 ).getEncoded() ), 32 );
assert( "an HmacSHA384 key is 48 bytes", arrayLen( generatedKey( "HmacSHA384", 384 ).getEncoded() ), 48 );
assert( "an HmacSHA512 key is 64 bytes", arrayLen( generatedKey( "HmacSHA512", 512 ).getEncoded() ), 64 );
assert( "without init() an HmacSHA256 key takes the digest size", arrayLen( generatedKey( "HmacSHA256" ).getEncoded() ), 32 );
assert( "an AES key honours init( 128 )", arrayLen( generatedKey( "AES", 128 ).getEncoded() ), 16 );
assert( "the generated key reports its algorithm", generatedKey( "HmacSHA256" ).getAlgorithm(), "HmacSHA256" );
assert( "getAlgorithm round-trips the requested spelling"
      , CreateObject( "java", "javax.crypto.KeyGenerator" ).getInstance( "HmacSHA256" ).getAlgorithm()
      , "HmacSHA256" );
assertTrue( "two generated keys differ"
          , arrayToList( generatedKey( "HmacSHA256" ).getEncoded() ) != arrayToList( generatedKey( "HmacSHA256" ).getEncoded() ) );

// init( keysize, SecureRandom ) is the other common spelling.
kgWithRandom = CreateObject( "java", "javax.crypto.KeyGenerator" ).getInstance( "HmacSHA512" );
kgWithRandom.init( 512, CreateObject( "java", "java.security.SecureRandom" ).init() );
assert( "init( bits, SecureRandom ) is accepted", arrayLen( kgWithRandom.generateKey().getEncoded() ), 64 );

noSuchKeyAlg = "";
try { CreateObject( "java", "javax.crypto.KeyGenerator" ).getInstance( "NoSuchThing" ); }
catch ( any e ) { noSuchKeyAlg = e.type; }
assert( "an unknown KeyGenerator algorithm raises NoSuchAlgorithmException", noSuchKeyAlg, "java.security.NoSuchAlgorithmException" );

badKeySize = "";
try { generatedKey( "AES", 100 ); }
catch ( any e ) { badKeySize = e.type; }
assert( "an invalid AES key size raises InvalidParameterException", badKeySize, "java.security.InvalidParameterException" );

// A generated key goes straight into Mac.init(), and the result must be the
// real HMAC: compare with the hmac() builtin over the same key bytes.
genKey    = generatedKey( "HmacSHA256", 256 );
genKeyB64 = ToBase64( genKey.getEncoded() );
assert( "ToBase64 of a byte[] encodes the bytes, not the array's text", arrayLen( ToBinary( genKeyB64 ) ), 32 );

genMac = CreateObject( "java", "javax.crypto.Mac" ).getInstance( "HmacSHA256" );
genMac.init( CreateObject( "java", "javax.crypto.spec.SecretKeySpec" ).init( ToBinary( genKeyB64 ), "HmacSHA256" ) );
assert( "ToBase64( mac.doFinal() ) is the base64 of the HMAC"
      , ToBase64( genMac.doFinal( "payload".getBytes() ) )
      , ToBase64( BinaryDecode( hmac( "payload", ToBinary( genKeyB64 ), "HmacSHA256" ), "hex" ) ) );
assert( "BinaryEncode of a byte[] encodes the bytes"
      , lCase( BinaryEncode( genKey.getEncoded(), "hex" ) )
      , lCase( BinaryEncode( ToBinary( genKeyB64 ), "hex" ) ) );

// RFC 4231 test case 2, through the path a signing library takes end to end.
rfcMac = CreateObject( "java", "javax.crypto.Mac" ).getInstance( "HmacSHA256" );
rfcMac.init( CreateObject( "java", "javax.crypto.spec.SecretKeySpec" ).init( ToBinary( ToBase64( "Jefe" ) ), "HmacSHA256" ) );
assert( "RFC 4231 case 2 reproduces through ToBase64( doFinal() )"
      , lCase( BinaryEncode( ToBinary( ToBase64( rfcMac.doFinal( "what do ya want for nothing?".getBytes() ) ) ), "hex" ) )
      , "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843" );

// generateSecretKey() mints HMAC keys too (Lucee does, via the same KeyGenerator).
assert( "generateSecretKey( HmacSHA256 ) is 32 bytes", arrayLen( ToBinary( generateSecretKey( "HmacSHA256" ) ) ), 32 );
assert( "generateSecretKey( HmacSHA512, 512 ) is 64 bytes", arrayLen( ToBinary( generateSecretKey( "HmacSHA512", 512 ) ) ), 64 );

suiteEnd();
</cfscript>
