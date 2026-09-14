component {
	variables.someState = 1;

	// level 0 = plain construction
	//       1 = + getMetaData(this)
	//       2 = + the whole-variables-scope copy into `core`
	//       3 = + the three mixin StructAppends   (the full Wheels body)
	public any function init( numeric level = 0 ) {
		if ( arguments.level GT 0 ) { $initializeMixins( variables, arguments.level ); }
		return this;
	}

	public any function $initializeMixins( required struct variablesScope, required numeric level ) {
		var $wheels = {};
		$wheels.metaData = getMetaData( variablesScope.this );
		$wheels.className = "model";
		if ( arguments.level LT 2 ) { return variablesScope; }
		if ( structKeyExists( application.mixins, $wheels.className ) ) {
			if ( !structKeyExists( variablesScope, "core" ) ) {
				variablesScope.core = {};
				structAppend( variablesScope.core, variablesScope );
				structDelete( variablesScope.core, "$wheels" );
			}
			if ( arguments.level LT 3 ) { return variablesScope; }
			structAppend( variablesScope, application.mixins[ $wheels.className ], true );
			if ( structKeyExists( variablesScope, "this" ) ) {
				structAppend( variablesScope.this, application.mixins[ $wheels.className ], true );
			}
			if ( structKeyExists( variablesScope.core, "this" ) ) {
				structAppend( variablesScope.core.this, application.mixins[ $wheels.className ], true );
			}
		}
		return variablesScope;
	}

	public string function m001( a, b ) { return "m001" & a & b; }
	public string function m002( a, b ) { return "m002" & a & b; }
	public string function m003( a, b ) { return "m003" & a & b; }
	public string function m004( a, b ) { return "m004" & a & b; }
	public string function m005( a, b ) { return "m005" & a & b; }
	public string function m006( a, b ) { return "m006" & a & b; }
	public string function m007( a, b ) { return "m007" & a & b; }
	public string function m008( a, b ) { return "m008" & a & b; }
	public string function m009( a, b ) { return "m009" & a & b; }
	public string function m010( a, b ) { return "m010" & a & b; }
	public string function m011( a, b ) { return "m011" & a & b; }
	public string function m012( a, b ) { return "m012" & a & b; }
	public string function m013( a, b ) { return "m013" & a & b; }
	public string function m014( a, b ) { return "m014" & a & b; }
	public string function m015( a, b ) { return "m015" & a & b; }
	public string function m016( a, b ) { return "m016" & a & b; }
	public string function m017( a, b ) { return "m017" & a & b; }
	public string function m018( a, b ) { return "m018" & a & b; }
	public string function m019( a, b ) { return "m019" & a & b; }
	public string function m020( a, b ) { return "m020" & a & b; }
	public string function m021( a, b ) { return "m021" & a & b; }
	public string function m022( a, b ) { return "m022" & a & b; }
	public string function m023( a, b ) { return "m023" & a & b; }
	public string function m024( a, b ) { return "m024" & a & b; }
	public string function m025( a, b ) { return "m025" & a & b; }
	public string function m026( a, b ) { return "m026" & a & b; }
	public string function m027( a, b ) { return "m027" & a & b; }
	public string function m028( a, b ) { return "m028" & a & b; }
	public string function m029( a, b ) { return "m029" & a & b; }
	public string function m030( a, b ) { return "m030" & a & b; }
	public string function m031( a, b ) { return "m031" & a & b; }
	public string function m032( a, b ) { return "m032" & a & b; }
	public string function m033( a, b ) { return "m033" & a & b; }
	public string function m034( a, b ) { return "m034" & a & b; }
	public string function m035( a, b ) { return "m035" & a & b; }
	public string function m036( a, b ) { return "m036" & a & b; }
	public string function m037( a, b ) { return "m037" & a & b; }
	public string function m038( a, b ) { return "m038" & a & b; }
	public string function m039( a, b ) { return "m039" & a & b; }
	public string function m040( a, b ) { return "m040" & a & b; }
	public string function m041( a, b ) { return "m041" & a & b; }
	public string function m042( a, b ) { return "m042" & a & b; }
	public string function m043( a, b ) { return "m043" & a & b; }
	public string function m044( a, b ) { return "m044" & a & b; }
	public string function m045( a, b ) { return "m045" & a & b; }
	public string function m046( a, b ) { return "m046" & a & b; }
	public string function m047( a, b ) { return "m047" & a & b; }
	public string function m048( a, b ) { return "m048" & a & b; }
	public string function m049( a, b ) { return "m049" & a & b; }
	public string function m050( a, b ) { return "m050" & a & b; }
	public string function m051( a, b ) { return "m051" & a & b; }
	public string function m052( a, b ) { return "m052" & a & b; }
	public string function m053( a, b ) { return "m053" & a & b; }
	public string function m054( a, b ) { return "m054" & a & b; }
	public string function m055( a, b ) { return "m055" & a & b; }
	public string function m056( a, b ) { return "m056" & a & b; }
	public string function m057( a, b ) { return "m057" & a & b; }
	public string function m058( a, b ) { return "m058" & a & b; }
	public string function m059( a, b ) { return "m059" & a & b; }
	public string function m060( a, b ) { return "m060" & a & b; }
	public string function m061( a, b ) { return "m061" & a & b; }
	public string function m062( a, b ) { return "m062" & a & b; }
	public string function m063( a, b ) { return "m063" & a & b; }
	public string function m064( a, b ) { return "m064" & a & b; }
	public string function m065( a, b ) { return "m065" & a & b; }
	public string function m066( a, b ) { return "m066" & a & b; }
	public string function m067( a, b ) { return "m067" & a & b; }
	public string function m068( a, b ) { return "m068" & a & b; }
	public string function m069( a, b ) { return "m069" & a & b; }
	public string function m070( a, b ) { return "m070" & a & b; }
	public string function m071( a, b ) { return "m071" & a & b; }
	public string function m072( a, b ) { return "m072" & a & b; }
	public string function m073( a, b ) { return "m073" & a & b; }
	public string function m074( a, b ) { return "m074" & a & b; }
	public string function m075( a, b ) { return "m075" & a & b; }
	public string function m076( a, b ) { return "m076" & a & b; }
	public string function m077( a, b ) { return "m077" & a & b; }
	public string function m078( a, b ) { return "m078" & a & b; }
	public string function m079( a, b ) { return "m079" & a & b; }
	public string function m080( a, b ) { return "m080" & a & b; }
	public string function m081( a, b ) { return "m081" & a & b; }
	public string function m082( a, b ) { return "m082" & a & b; }
	public string function m083( a, b ) { return "m083" & a & b; }
	public string function m084( a, b ) { return "m084" & a & b; }
	public string function m085( a, b ) { return "m085" & a & b; }
	public string function m086( a, b ) { return "m086" & a & b; }
	public string function m087( a, b ) { return "m087" & a & b; }
	public string function m088( a, b ) { return "m088" & a & b; }
	public string function m089( a, b ) { return "m089" & a & b; }
	public string function m090( a, b ) { return "m090" & a & b; }
	public string function m091( a, b ) { return "m091" & a & b; }
	public string function m092( a, b ) { return "m092" & a & b; }
	public string function m093( a, b ) { return "m093" & a & b; }
	public string function m094( a, b ) { return "m094" & a & b; }
	public string function m095( a, b ) { return "m095" & a & b; }
	public string function m096( a, b ) { return "m096" & a & b; }
	public string function m097( a, b ) { return "m097" & a & b; }
	public string function m098( a, b ) { return "m098" & a & b; }
	public string function m099( a, b ) { return "m099" & a & b; }
	public string function m100( a, b ) { return "m100" & a & b; }
	public string function m101( a, b ) { return "m101" & a & b; }
	public string function m102( a, b ) { return "m102" & a & b; }
	public string function m103( a, b ) { return "m103" & a & b; }
	public string function m104( a, b ) { return "m104" & a & b; }
	public string function m105( a, b ) { return "m105" & a & b; }
	public string function m106( a, b ) { return "m106" & a & b; }
	public string function m107( a, b ) { return "m107" & a & b; }
	public string function m108( a, b ) { return "m108" & a & b; }
	public string function m109( a, b ) { return "m109" & a & b; }
	public string function m110( a, b ) { return "m110" & a & b; }
	public string function m111( a, b ) { return "m111" & a & b; }
	public string function m112( a, b ) { return "m112" & a & b; }
	public string function m113( a, b ) { return "m113" & a & b; }
	public string function m114( a, b ) { return "m114" & a & b; }
	public string function m115( a, b ) { return "m115" & a & b; }
	public string function m116( a, b ) { return "m116" & a & b; }
	public string function m117( a, b ) { return "m117" & a & b; }
	public string function m118( a, b ) { return "m118" & a & b; }
	public string function m119( a, b ) { return "m119" & a & b; }
	public string function m120( a, b ) { return "m120" & a & b; }
	public string function m121( a, b ) { return "m121" & a & b; }
	public string function m122( a, b ) { return "m122" & a & b; }
	public string function m123( a, b ) { return "m123" & a & b; }
	public string function m124( a, b ) { return "m124" & a & b; }
	public string function m125( a, b ) { return "m125" & a & b; }
	public string function m126( a, b ) { return "m126" & a & b; }
	public string function m127( a, b ) { return "m127" & a & b; }
	public string function m128( a, b ) { return "m128" & a & b; }
	public string function m129( a, b ) { return "m129" & a & b; }
	public string function m130( a, b ) { return "m130" & a & b; }
	public string function m131( a, b ) { return "m131" & a & b; }
	public string function m132( a, b ) { return "m132" & a & b; }
	public string function m133( a, b ) { return "m133" & a & b; }
	public string function m134( a, b ) { return "m134" & a & b; }
	public string function m135( a, b ) { return "m135" & a & b; }
	public string function m136( a, b ) { return "m136" & a & b; }
	public string function m137( a, b ) { return "m137" & a & b; }
	public string function m138( a, b ) { return "m138" & a & b; }
	public string function m139( a, b ) { return "m139" & a & b; }
	public string function m140( a, b ) { return "m140" & a & b; }
	public string function m141( a, b ) { return "m141" & a & b; }
	public string function m142( a, b ) { return "m142" & a & b; }
	public string function m143( a, b ) { return "m143" & a & b; }
	public string function m144( a, b ) { return "m144" & a & b; }
	public string function m145( a, b ) { return "m145" & a & b; }
	public string function m146( a, b ) { return "m146" & a & b; }
	public string function m147( a, b ) { return "m147" & a & b; }
	public string function m148( a, b ) { return "m148" & a & b; }
	public string function m149( a, b ) { return "m149" & a & b; }
	public string function m150( a, b ) { return "m150" & a & b; }
	public string function m151( a, b ) { return "m151" & a & b; }
	public string function m152( a, b ) { return "m152" & a & b; }
	public string function m153( a, b ) { return "m153" & a & b; }
	public string function m154( a, b ) { return "m154" & a & b; }
	public string function m155( a, b ) { return "m155" & a & b; }
	public string function m156( a, b ) { return "m156" & a & b; }
	public string function m157( a, b ) { return "m157" & a & b; }
	public string function m158( a, b ) { return "m158" & a & b; }
	public string function m159( a, b ) { return "m159" & a & b; }
	public string function m160( a, b ) { return "m160" & a & b; }
	public string function m161( a, b ) { return "m161" & a & b; }
	public string function m162( a, b ) { return "m162" & a & b; }
	public string function m163( a, b ) { return "m163" & a & b; }
	public string function m164( a, b ) { return "m164" & a & b; }
	public string function m165( a, b ) { return "m165" & a & b; }
	public string function m166( a, b ) { return "m166" & a & b; }
	public string function m167( a, b ) { return "m167" & a & b; }
	public string function m168( a, b ) { return "m168" & a & b; }
	public string function m169( a, b ) { return "m169" & a & b; }
	public string function m170( a, b ) { return "m170" & a & b; }
	public string function m171( a, b ) { return "m171" & a & b; }
	public string function m172( a, b ) { return "m172" & a & b; }
	public string function m173( a, b ) { return "m173" & a & b; }
	public string function m174( a, b ) { return "m174" & a & b; }
	public string function m175( a, b ) { return "m175" & a & b; }
	public string function m176( a, b ) { return "m176" & a & b; }
	public string function m177( a, b ) { return "m177" & a & b; }
	public string function m178( a, b ) { return "m178" & a & b; }
	public string function m179( a, b ) { return "m179" & a & b; }
	public string function m180( a, b ) { return "m180" & a & b; }
	public string function m181( a, b ) { return "m181" & a & b; }
	public string function m182( a, b ) { return "m182" & a & b; }
	public string function m183( a, b ) { return "m183" & a & b; }
	public string function m184( a, b ) { return "m184" & a & b; }
	public string function m185( a, b ) { return "m185" & a & b; }
	public string function m186( a, b ) { return "m186" & a & b; }
	public string function m187( a, b ) { return "m187" & a & b; }
	public string function m188( a, b ) { return "m188" & a & b; }
	public string function m189( a, b ) { return "m189" & a & b; }
	public string function m190( a, b ) { return "m190" & a & b; }
	public string function m191( a, b ) { return "m191" & a & b; }
	public string function m192( a, b ) { return "m192" & a & b; }
	public string function m193( a, b ) { return "m193" & a & b; }
	public string function m194( a, b ) { return "m194" & a & b; }
	public string function m195( a, b ) { return "m195" & a & b; }
	public string function m196( a, b ) { return "m196" & a & b; }
	public string function m197( a, b ) { return "m197" & a & b; }
	public string function m198( a, b ) { return "m198" & a & b; }
	public string function m199( a, b ) { return "m199" & a & b; }
	public string function m200( a, b ) { return "m200" & a & b; }
	public string function m201( a, b ) { return "m201" & a & b; }
	public string function m202( a, b ) { return "m202" & a & b; }
	public string function m203( a, b ) { return "m203" & a & b; }
	public string function m204( a, b ) { return "m204" & a & b; }
	public string function m205( a, b ) { return "m205" & a & b; }
	public string function m206( a, b ) { return "m206" & a & b; }
	public string function m207( a, b ) { return "m207" & a & b; }
	public string function m208( a, b ) { return "m208" & a & b; }
	public string function m209( a, b ) { return "m209" & a & b; }
	public string function m210( a, b ) { return "m210" & a & b; }
	public string function m211( a, b ) { return "m211" & a & b; }
	public string function m212( a, b ) { return "m212" & a & b; }
	public string function m213( a, b ) { return "m213" & a & b; }
	public string function m214( a, b ) { return "m214" & a & b; }
	public string function m215( a, b ) { return "m215" & a & b; }
	public string function m216( a, b ) { return "m216" & a & b; }
	public string function m217( a, b ) { return "m217" & a & b; }
	public string function m218( a, b ) { return "m218" & a & b; }
	public string function m219( a, b ) { return "m219" & a & b; }
	public string function m220( a, b ) { return "m220" & a & b; }
	public string function m221( a, b ) { return "m221" & a & b; }
	public string function m222( a, b ) { return "m222" & a & b; }
	public string function m223( a, b ) { return "m223" & a & b; }
	public string function m224( a, b ) { return "m224" & a & b; }
	public string function m225( a, b ) { return "m225" & a & b; }
	public string function m226( a, b ) { return "m226" & a & b; }
	public string function m227( a, b ) { return "m227" & a & b; }
	public string function m228( a, b ) { return "m228" & a & b; }
	public string function m229( a, b ) { return "m229" & a & b; }
	public string function m230( a, b ) { return "m230" & a & b; }
	public string function m231( a, b ) { return "m231" & a & b; }
	public string function m232( a, b ) { return "m232" & a & b; }
	public string function m233( a, b ) { return "m233" & a & b; }
	public string function m234( a, b ) { return "m234" & a & b; }
	public string function m235( a, b ) { return "m235" & a & b; }
	public string function m236( a, b ) { return "m236" & a & b; }
	public string function m237( a, b ) { return "m237" & a & b; }
	public string function m238( a, b ) { return "m238" & a & b; }
	public string function m239( a, b ) { return "m239" & a & b; }
	public string function m240( a, b ) { return "m240" & a & b; }
	public string function m241( a, b ) { return "m241" & a & b; }
	public string function m242( a, b ) { return "m242" & a & b; }
	public string function m243( a, b ) { return "m243" & a & b; }
	public string function m244( a, b ) { return "m244" & a & b; }
	public string function m245( a, b ) { return "m245" & a & b; }
	public string function m246( a, b ) { return "m246" & a & b; }
	public string function m247( a, b ) { return "m247" & a & b; }
	public string function m248( a, b ) { return "m248" & a & b; }
	public string function m249( a, b ) { return "m249" & a & b; }
	public string function m250( a, b ) { return "m250" & a & b; }
	public string function m251( a, b ) { return "m251" & a & b; }
	public string function m252( a, b ) { return "m252" & a & b; }
	public string function m253( a, b ) { return "m253" & a & b; }
	public string function m254( a, b ) { return "m254" & a & b; }
	public string function m255( a, b ) { return "m255" & a & b; }
	public string function m256( a, b ) { return "m256" & a & b; }
	public string function m257( a, b ) { return "m257" & a & b; }
	public string function m258( a, b ) { return "m258" & a & b; }
	public string function m259( a, b ) { return "m259" & a & b; }
	public string function m260( a, b ) { return "m260" & a & b; }
	public string function m261( a, b ) { return "m261" & a & b; }
	public string function m262( a, b ) { return "m262" & a & b; }
	public string function m263( a, b ) { return "m263" & a & b; }
	public string function m264( a, b ) { return "m264" & a & b; }
	public string function m265( a, b ) { return "m265" & a & b; }
	public string function m266( a, b ) { return "m266" & a & b; }
	public string function m267( a, b ) { return "m267" & a & b; }
	public string function m268( a, b ) { return "m268" & a & b; }
	public string function m269( a, b ) { return "m269" & a & b; }
	public string function m270( a, b ) { return "m270" & a & b; }
	public string function m271( a, b ) { return "m271" & a & b; }
	public string function m272( a, b ) { return "m272" & a & b; }
	public string function m273( a, b ) { return "m273" & a & b; }
	public string function m274( a, b ) { return "m274" & a & b; }
	public string function m275( a, b ) { return "m275" & a & b; }
	public string function m276( a, b ) { return "m276" & a & b; }
	public string function m277( a, b ) { return "m277" & a & b; }
	public string function m278( a, b ) { return "m278" & a & b; }
	public string function m279( a, b ) { return "m279" & a & b; }
	public string function m280( a, b ) { return "m280" & a & b; }
	public string function m281( a, b ) { return "m281" & a & b; }
	public string function m282( a, b ) { return "m282" & a & b; }
	public string function m283( a, b ) { return "m283" & a & b; }
	public string function m284( a, b ) { return "m284" & a & b; }
	public string function m285( a, b ) { return "m285" & a & b; }
	public string function m286( a, b ) { return "m286" & a & b; }
	public string function m287( a, b ) { return "m287" & a & b; }
	public string function m288( a, b ) { return "m288" & a & b; }
	public string function m289( a, b ) { return "m289" & a & b; }
	public string function m290( a, b ) { return "m290" & a & b; }
	public string function m291( a, b ) { return "m291" & a & b; }
	public string function m292( a, b ) { return "m292" & a & b; }
	public string function m293( a, b ) { return "m293" & a & b; }
	public string function m294( a, b ) { return "m294" & a & b; }
	public string function m295( a, b ) { return "m295" & a & b; }
	public string function m296( a, b ) { return "m296" & a & b; }
	public string function m297( a, b ) { return "m297" & a & b; }
	public string function m298( a, b ) { return "m298" & a & b; }
	public string function m299( a, b ) { return "m299" & a & b; }
	public string function m300( a, b ) { return "m300" & a & b; }
}
