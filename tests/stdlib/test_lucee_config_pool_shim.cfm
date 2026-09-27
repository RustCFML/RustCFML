<cfscript>
suiteBegin( "getPageContext().getConfig() connection-pool gauges (RustCFML)" );
// preside-ext-k8s-essentials' Prometheus collector reads these. The maximum is
// real; the active/idle counts are not observable and come back EMPTY, so an
// IsNumeric() formatter emits NaN ("no data") rather than a misleading 0
// (docs/known-issues.md §116).
_pcCfg = getPageContext().getConfig();
_pcDs = _pcCfg.getDatasource( "testds" );
assert( "getDatasource().getName()", _pcDs.getName(), "testds" );
_pcPool = _pcCfg.getDatasourceConnectionPool( _pcDs, nullValue(), nullValue() );
assert( "getMaxTotal() is the real pool maximum (sqlite pool)", _pcPool.getMaxTotal(), 10 );
assertFalse( "getNumActive() is not a number", isNumeric( _pcPool.getNumActive() ) );
assertFalse( "getNumIdle() is not a number", isNumeric( _pcPool.getNumIdle() ) );
function _pcInt( _number ) { if ( IsNumeric( arguments._number ) ) return Int( _number ); return "Nan"; }
assert( "formatted the collector's way it is NaN", _pcInt( _pcPool.getNumActive() ), "Nan" );
assertThrows( "an unknown datasource throws", function(){ getPageContext().getConfig().getDatasource( "no_such_ds" ); } );
suiteEnd();
</cfscript>
