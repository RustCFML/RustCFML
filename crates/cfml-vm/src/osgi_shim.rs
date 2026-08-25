//! `lucee.runtime.osgi.OSGiUtil` and `lucee.loader.engine.CFMLEngineFactory` —
//! Lucee's OSGi bundle plumbing, made **inert**.
//!
//! CFML libraries that ship their own jars load them by installing an OSGi bundle
//! into the running Lucee and then naming the bundle when they construct a class:
//!
//! ```cfml
//! osgiUtil = CreateObject( "java", "lucee.runtime.osgi.OSGiUtil" );
//! engine   = CreateObject( "java", "lucee.loader.engine.CFMLEngineFactory" ).getInstance();
//! if ( !bundleIsLoaded( name, version ) ) {
//!     resource = engine.getResourceUtil().toResourceExisting( GetPageContext(), jarPath );
//!     osgiUtil.installBundle( engine.getBundleContext(), resource, true );
//! }
//! return CreateObject( "java", className, name, version );   // <- the real request
//! ```
//!
//! There is no JVM, no OSGi container and no jar to install, so none of that can
//! happen. What matters is the last line: the bundle dance is *ceremony* around a
//! `createObject` the engine may well be able to answer natively. This shim lets
//! the ceremony complete so the real request is reached — and the real request is
//! then answered, or refused, on its own merits by the class shims.
//!
//! `getBundleLoaded()` deliberately returns a **stub bundle rather than null**, so
//! callers take their "already loaded, nothing to do" path and never build a
//! `Resource` from a jar that will not be read. That is honest here in the only
//! sense that matters to the caller: whether the classes from that bundle can be
//! constructed afterwards. If they cannot, `createObject` says so by name.
//!
//! **This is a no-op by design and it is recorded as one** — see
//! `docs/known-issues.md`. Nothing that was working stops working: without it the
//! `init()` of any such library is a hard error, so the only reachable states are
//! "throws at construction" and "reaches the class shims".

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};

pub const OSGI_UTIL_CLASS: &str = "lucee.runtime.osgi.osgiutil";
pub const ENGINE_FACTORY_CLASS: &str = "lucee.loader.engine.cfmlenginefactory";
pub const ENGINE_CLASS: &str = "lucee.runtime.cfmlengine";
pub const BUNDLE_CLASS: &str = "org.osgi.framework.bundle";
pub const BUNDLE_CONTEXT_CLASS: &str = "org.osgi.framework.bundlecontext";
pub const RESOURCE_UTIL_CLASS: &str = "lucee.commons.io.res.util.resourceutil";
pub const RESOURCE_CLASS: &str = "lucee.commons.io.res.resource";
pub const VERSION_CLASS: &str = "org.osgi.framework.version";

pub fn is_osgi_class(class_lower: &str) -> bool {
    matches!(
        class_lower,
        OSGI_UTIL_CLASS
            | ENGINE_FACTORY_CLASS
            | ENGINE_CLASS
            | BUNDLE_CLASS
            | BUNDLE_CONTEXT_CLASS
            | RESOURCE_UTIL_CLASS
            | RESOURCE_CLASS
            | VERSION_CLASS
    )
}

fn shim(class: &str) -> CfmlValue {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    CfmlValue::strukt(m)
}

pub fn construct(class_lower: &str) -> CfmlResult {
    Ok(shim(class_lower))
}

fn unsupported(class: &str, method: &str) -> CfmlError {
    CfmlError::new(
        format!(
            "{}.{}() is not supported by RustCFML's inert OSGi shim. There is no OSGi \
             container; the shim exists only so a library's bundle-loading ceremony can \
             complete and reach the class it actually wants.",
            class, method
        ),
        CfmlErrorType::Custom("java.lang.UnsupportedOperationException".to_string()),
    )
}

pub fn dispatch(class_lower: &str, method: &str, args: Vec<CfmlValue>) -> CfmlResult {
    match (class_lower, method) {
        // ---- OSGiUtil ----
        // Report every bundle as already present, so callers skip installBundle
        // entirely rather than constructing a Resource for an unreadable jar.
        (OSGI_UTIL_CLASS, "getbundleloaded") => Ok(shim(BUNDLE_CLASS)),
        (OSGI_UTIL_CLASS, "getbundle") => Ok(shim(BUNDLE_CLASS)),
        // If a caller installs anyway (it checked nothing, or asked for a version
        // we did not claim), accept and do nothing — the class shims are what
        // decide whether anything actually works.
        (OSGI_UTIL_CLASS, "installbundle") => Ok(shim(BUNDLE_CLASS)),
        (OSGI_UTIL_CLASS, "toversion") => Ok(CfmlValue::string(
            args.first().map(|v| v.as_string()).unwrap_or_default(),
        )),

        // ---- CFMLEngineFactory / CFMLEngine ----
        (ENGINE_FACTORY_CLASS, "getinstance") => Ok(shim(ENGINE_CLASS)),
        (ENGINE_CLASS, "getresourceutil") => Ok(shim(RESOURCE_UTIL_CLASS)),
        (ENGINE_CLASS, "getbundlecontext") => Ok(shim(BUNDLE_CONTEXT_CLASS)),
        // Libraries version-gate their behaviour on this. Report what the rest of
        // the engine reports, so a library sees one consistent story.
        (ENGINE_CLASS, "getversion" | "getinfo") => Ok(CfmlValue::string("7.0.0.0".to_string())),

        // ---- ResourceUtil / Resource ----
        // A "resource" is only ever handed straight back to installBundle, so it
        // need carry nothing but the path it was asked about.
        (RESOURCE_UTIL_CLASS, "toresourceexisting" | "toresource") => {
            let mut m = ValueMap::default();
            m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            m.insert(
                "__java_class".to_string(),
                CfmlValue::string(RESOURCE_CLASS.to_string()),
            );
            // toResourceExisting( pageContext, path ) — the path is the 2nd arg.
            m.insert(
                "__path".to_string(),
                CfmlValue::string(args.get(1).map(|v| v.as_string()).unwrap_or_default()),
            );
            Ok(CfmlValue::strukt(m))
        }

        // ---- Bundle ----
        (BUNDLE_CLASS, "uninstall" | "start" | "stop" | "update") => Ok(CfmlValue::Null),
        (BUNDLE_CLASS, "getsymbolicname") => Ok(CfmlValue::string(String::new())),
        (BUNDLE_CLASS, "getversion") => Ok(CfmlValue::string("0.0.0".to_string())),
        (BUNDLE_CLASS, "getstate") => Ok(CfmlValue::Int(32)), // Bundle.ACTIVE

        (class, other) => Err(unsupported(class, other)),
    }
}
