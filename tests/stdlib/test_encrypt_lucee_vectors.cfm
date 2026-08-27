<cfscript>
suiteBegin("encrypt/decrypt: Lucee cipher vectors");

// Every ciphertext below was captured from Lucee 7 and is asserted verbatim.
// Round-trip tests alone cannot see the divergences these cover: RustCFML used
// to read a bare "AES" as CBC-with-a-zero-IV (which agrees with Lucee's ECB for
// the FIRST BLOCK ONLY), and its CFMX_COMPAT was a different cipher entirely --
// so an application's data written under Lucee read back as garbage.
aesKey   = "XnP4WM9I8LhRm06Z39RzKA==";       // 16 bytes
tripleKey= "u9qoCflq2P+UwqfQUDXzZeGO53A5SU0v"; // 24 bytes
desKey   = "19u6i34HuqA=";                   // 8 bytes
long     = "This plaintext is definitely longer than one sixteen byte block!!";

// --- CFMX_COMPAT: the DEFAULT algorithm when the caller names none ---
assert( "a two-argument Encrypt is CFMX_COMPAT, not AES"
      , Encrypt( long, "notverysecurekey", "CFMX_COMPAT", "hex" )
      , Encrypt( long, "notverysecurekey", "", "hex" ) );
assert( "CFMX_COMPAT keystream matches Lucee"
      , Encrypt( long, "mykey", "CFMX_COMPAT", "hex" )
      , "47B6B481ED96D71A95DC8F0267B887305CEF971C5586828487899B02CE8BA05DFA1247454D46B39D169A488AD287DAB3909B9B80DCAE8A4BACD97B5E93FA5847EA" );
assert( "CFMX_COMPAT seeds an empty key from NUL chars, not from a default seed"
      , Encrypt( long, "", "CFMX_COMPAT", "hex" )
      , "B4039E45BC834B079A888753C6B8EF7E3E4CD2BAA895B9008B529B958E98F8517B9809C69357479579B2C1AA77AEF64346BB9C093B149D8D76D4B54099ADF4CFD6" );
assert( "CFMX_COMPAT repeats a short key over the 12-char seed"
      , Encrypt( long, "ab", "CFMX_COMPAT", "hex" )
      , "75B195BDDEBC90FF8DF1CFAC8ED3CF0FAC1E810393779CB06A8CB38A069186511D8BABD39372969D0F76A156F480A0974D823B9DFF7EB0BABB92A91F2446BFD833" );
assert( "CFMX_COMPAT seeds a key longer than 12 chars from chars 4-7"
      , Encrypt( long, "a-much-longer-key-than-twelve", "CFMX_COMPAT", "hex" )
      , "4EB3D5EEDDC75F97A5A5073AE342C78F946F829C95A0A4A64318B2C64E9E93A55C4A148E92857CF0ECB031BC0D4DA3C183589295CF9B25C2ACD31D4B088CB6DE4F" );
assert( "CFMX_COMPAT seeds a non-ASCII key per UTF-16 char, not per byte"
      , Encrypt( long, "kéy-ünicode", "CFMX_COMPAT", "hex" )
      , "4381A601DD57329C9A5C9D5EE6C8D2A5AA3F2B003FF0778F8D5B974ADEFE91A2BA78A6ED51919AF1074BD199D695B09F5B8A9C72EE99638DA8DE8B455CB8561618" );
assert( "CFMX_COMPAT enciphers the body's UTF-8 bytes"
      , Encrypt( "héllo wörld", "mykey", "CFMX_COMPAT", "hex" )
      , "7B1D749EA1899B0C3F04890B7B" );
assert( "CFMX_COMPAT base64 encoding"
      , Encrypt( long, "mykey", "CFMX_COMPAT", "base64" )
      , "R7a0ge2W1xqV3I8CZ7iHMFzvlxxVhoKEh4mbAs6LoF36EkdFTUaznRaaSIrSh9qzkJubgNyuikus2Xtek/pYR+o=" );
assert( "CFMX_COMPAT round-trips through the default (UU) encoding"
      , Decrypt( Encrypt( long, "mykey" ), "mykey" ), long );

// --- block ciphers: a bare algorithm name is ECB, as on the JVM ---
assert( "a bare AES is ECB"
      , Encrypt( long, aesKey, "AES", "hex" )
      , "A19C42C0943F507A6C917ED27E88D34CE6EDF688A0AA88FD1DD63C40F3E085377F8C16138502CB52D38ED2F4C12FC596F3670305127FE6400F627C72B7AC47844BF3EAFD95D7B4295DF10732A4CD446D" );
assert( "an explicit AES/ECB/PKCS5Padding is the same cipher"
      , Encrypt( long, aesKey, "AES/ECB/PKCS5Padding", "hex" )
      , Encrypt( long, aesKey, "AES", "hex" ) );
assert( "AES/CBC/PKCS5Padding with a caller-supplied IV"
      , Encrypt( long, aesKey, "AES/CBC/PKCS5Padding", "hex", "0123456789abcdef" )
      , "96D21AC3926065109CE50DCC107D819F833D68D7FA72E30E30B1E92BC6758447ABCDD16E7776E918CF3E8AFB27DDE6605FF7E5033CA24E1B08D3DE44EC17B81FE558AA7003F82EA4A9660FA3DEC8C7FB" );
assert( "AES-192 from a 24-byte key"
      , Encrypt( long, tripleKey, "AES", "hex" )
      , "1D663909620F8256647012836F45B54C1763E3A1F52FD299FB7566CBCB55A4445566A1AC1FF9380B9EBB336286BD74A3139FA10B3F29BADB550A812A60F5D44B5740966201583AA047CE59D8C539CDA8" );
assert( "DES"
      , Encrypt( long, desKey, "DES", "hex" )
      , "8C02BA3C98CC0AF6C755D8AA1A995585E309C4BBBF2AF0D81291F8CAC9CAF24CE1102C23F11F0451901FAD68C8710A9D797B36B81928EAC6BDD17BC5B0F02E786BC8C948C9716901" );
assert( "DESEDE"
      , Encrypt( long, tripleKey, "DESEDE", "hex" )
      , "9FB6EF0B2917BC82AA7F9E655DA38F8480508B7E52B70EFCDD07E7777C282F70AAF2BB2289FE6360B7972B44BA204F5AA4482CA46780673359938C3EF2144F726830268B7DAACC85" );
assert( "BLOWFISH"
      , Encrypt( long, aesKey, "BLOWFISH", "hex" )
      , "277C6BBD35AF67226FFF008B62595EC8078415DC00CA6C6390D1AB569EB613A64AFE74805A73DCEB17BE967B186BE2EE6E94F2777928E9E52E617B9400CD70BF542CCD6FCBBD569F" );

// A feedback-mode encrypt with no IV generates one and PREPENDS it, so the
// ciphertext is one block longer and decrypt finds the IV where it was left.
cbc = Encrypt( long, aesKey, "AES/CBC/PKCS5Padding", "hex" );
assert( "a generated IV is prepended to the ciphertext", Len( cbc ), 192 );
assert( "...and decrypt reads it back", Decrypt( cbc, aesKey, "AES/CBC/PKCS5Padding", "hex" ), long );
assert( "AES round-trips", Decrypt( Encrypt( long, aesKey, "AES", "hex" ), aesKey, "AES", "hex" ), long );

// --- key handling ---
// Under 8 characters a key is raw UTF-8, NUL-padded to the algorithm's length.
assert( "a key under 8 chars is raw UTF-8, padded to the key length"
      , Encrypt( long, "abcdefg", "AES", "hex" )
      , "144FB7C252CB3F6E91258DB32588822C4ABDCDE6BA046AB1074B22254341F7B073692508AACF93A6520D5BCB98B704250CE06757BC4BAAFC63146632B3B8F0CFCC78B90375677D1A3BC60AB994089086" );
assert( "...and for DES, padded to 8"
      , Encrypt( long, "abc", "DES", "hex" )
      , "75201474A3FBFE3AE76D05EC3F756D8B6080D994052DED3B4671975B60F8E5D97F7DF616A42AE61709FBDEC711D3311907839E949A4949DCA2224686B94916EE8543F4AEEC06D6C3" );

// 8 characters or more is base64 -- and must decode to a length the algorithm
// takes. The failures are typed, because CFML code catches them by type.
function errType( required string key ) {
	try { Encrypt( "a", arguments.key, "AES", "hex" ); }
	catch( any e ) { return e.type & "|" & e.message; }
	return "no error";
}
assert( "a base64 key of the wrong length is a RuntimeException naming the length"
      , errType( "abcdefghijklmnop" )
      , "java.lang.RuntimeException|Invalid key length for AES: 12 bytes" );
assert( "a key that is not base64 at all is a CoderException"
      , errType( "not-base64!!key" )
      , "lucee.runtime.coder.CoderException|cannot convert the input to a binary, invalid length (15) of the string" );
assert( "...and an invalid character reports the LAST one, scanning backwards"
      , errType( "aaaa!!!!aaaa!!!!" )
      , "lucee.runtime.coder.CoderException|invalid character [!] in base64 string at position [16]" );

// precise=false trades those errors for the raw-UTF-8 fallback.
assert( "precise=false falls back to the raw key rather than throwing"
      , Encrypt( "a", "abcdefghijklmnop", "AES", "hex", "", 0, false )
      , "426D49FF405BBDFF58294E9E871B8205" );
assert( "...for an undecodable key too"
      , Encrypt( "a", "not-base64!!key", "AES", "hex", "", 0, false )
      , "043F00712409DF97D0A0462E4FC77192" );

suiteEnd();
</cfscript>
