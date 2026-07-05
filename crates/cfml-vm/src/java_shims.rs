// Java shim handlers - to be inserted into lib.rs

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};
use chrono::{Datelike, NaiveDateTime, Timelike};

/// Build a bare java-shim marker map for `class`.
fn java_shim_map(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m
}

// ─────────────────────────────────────────────
// java.util.concurrent — executor family
//
// RustCFML has no JVM thread pool. These shims present the ExecutorService /
// ScheduledExecutorService / Executors / TimeUnit surface that ColdBox
// (`coldbox/system/async/*`) and Preside (`system/externals/cfconcurrent/*`)
// build on, and route the *executing* methods (submit/execute/schedule) through
// the engine's native async kernel (`runAsync`/`_schedule`) — those live VM-side
// (they need `&mut VM`); the pure lifecycle/stat methods live here.
// ─────────────────────────────────────────────

pub const EXECUTORS_CLASS: &str = "java.util.concurrent.executors";
pub const EXECUTOR_SERVICE_CLASS: &str = "java.util.concurrent.threadpoolexecutor";
pub const SCHEDULED_EXECUTOR_CLASS: &str = "java.util.concurrent.scheduledthreadpoolexecutor";
pub const TIMEUNIT_CLASS: &str = "java.util.concurrent.timeunit";
pub const THREADFACTORY_CLASS: &str = "java.util.concurrent.threadfactory";
pub const COMPLETION_SERVICE_CLASS: &str = "java.util.concurrent.executorcompletionservice";

/// `java.util.concurrent.Executors` — a static factory holder. Method calls
/// (newFixedThreadPool/newScheduledThreadPool/…) are handled VM-side.
pub fn make_executors_static() -> CfmlValue {
    CfmlValue::strukt(java_shim_map(EXECUTORS_CLASS))
}

/// An ExecutorService / ThreadPoolExecutor shim. `kind` is informational
/// ("fixed"/"cached"/"single"). Carries a `__shutdown` flag toggled by
/// shutdown()/shutdownNow() (via method writeback).
pub fn make_executor_service(kind: &str) -> CfmlValue {
    let mut m = java_shim_map(EXECUTOR_SERVICE_CLASS);
    m.insert("__executor_kind".to_string(), CfmlValue::string(kind.to_string()));
    m.insert("__shutdown".to_string(), CfmlValue::Bool(false));
    CfmlValue::strukt(m)
}

/// A ScheduledExecutorService / ScheduledThreadPoolExecutor shim.
pub fn make_scheduled_executor() -> CfmlValue {
    let mut m = java_shim_map(SCHEDULED_EXECUTOR_CLASS);
    m.insert("__executor_kind".to_string(), CfmlValue::string("scheduled".to_string()));
    m.insert("__shutdown".to_string(), CfmlValue::Bool(false));
    CfmlValue::strukt(m)
}

/// `java.util.concurrent.TimeUnit` — an enum holder. Reads of `.SECONDS` etc.
/// (property access) return the token string; `.toString()` on a token is the
/// token itself. RustCFML never feeds these to a live executor, so opaque
/// string tokens are sufficient (matches Preside's own RustCFML branch).
pub fn make_timeunit() -> CfmlValue {
    let mut m = java_shim_map(TIMEUNIT_CLASS);
    for u in [
        "NANOSECONDS",
        "MICROSECONDS",
        "MILLISECONDS",
        "SECONDS",
        "MINUTES",
        "HOURS",
        "DAYS",
    ] {
        m.insert(u.to_string(), CfmlValue::string(u.to_string()));
    }
    CfmlValue::strukt(m)
}

/// `java.util.concurrent.ThreadFactory` (or Executors.defaultThreadFactory()).
pub fn make_threadfactory() -> CfmlValue {
    CfmlValue::strukt(java_shim_map(THREADFACTORY_CLASS))
}

/// `java.util.concurrent.ExecutorCompletionService(executor[, queue])`. Wraps a
/// backing executor; submit() runs the task and stashes the Future so poll()/
/// take() can return it in completion order. The completed-future queue lives on
/// the shim struct and is mutated via method writeback.
pub fn make_completion_service(executor: CfmlValue) -> CfmlValue {
    let mut m = java_shim_map(COMPLETION_SERVICE_CLASS);
    m.insert("__cs_executor".to_string(), executor);
    m.insert("__cs_completed".to_string(), CfmlValue::array(vec![]));
    CfmlValue::strukt(m)
}

/// `System.out` / `System.err` PrintStream methods. `println`/`print` write to
/// the process stdout/stderr (Java semantics — console, NOT the HTTP output
/// buffer); `flush`/`close`/`write`/`append` are accepted. Used by ColdBox's
/// ConsoleAppender.
pub fn handle_java_printstream(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    let is_err = matches!(object, CfmlValue::Struct(s)
        if s.get("__java_class")
            .map(|v| v.as_string())
            .unwrap_or_default()
            .ends_with(".err"));
    let text: String = args.iter().map(|a| a.as_string()).collect::<Vec<_>>().join("");
    match method {
        "println" => {
            if is_err {
                eprintln!("{}", text);
            } else {
                println!("{}", text);
            }
        }
        "print" | "write" | "append" | "printf" | "format" => {
            if is_err {
                eprint!("{}", text);
            } else {
                print!("{}", text);
            }
        }
        _ => {} // flush/close/checkError/etc — no-op
    }
    // Return the stream object so a `.append(x).append(y)` chain still resolves,
    // and so the caller treats this as authoritatively handled (not a miss).
    Ok(object.clone())
}

/// TimeUnit method dispatch — only `toString()`/`name()` are ever called; both
/// return the token. Property reads (`.SECONDS`) are handled by generic struct
/// member access, not here.
pub fn handle_java_timeunit(method: &str, _args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    let m = method.to_ascii_lowercase();
    if m == "tostring" || m == "name" {
        // The token is stored per-constant, not on the class object; a bare
        // TimeUnit class .toString() has no single value, so return "TimeUnit".
        if let CfmlValue::Struct(s) = object {
            if let Some(t) = s.get("__timeunit_token") {
                return Ok(CfmlValue::string(t.as_string()));
            }
        }
        return Ok(CfmlValue::string("TimeUnit".to_string()));
    }
    Ok(CfmlValue::Null)
}

/// ThreadFactory.newThread(runnable) → a lightweight Thread shim carrying the
/// runnable. RustCFML never starts it via the factory (the executor shim runs
/// tasks directly), so this only needs getName/setName to satisfy Preside's
/// `ThreadFactory.cfc` naming logic.
pub fn handle_java_threadfactory(
    method: &str,
    args: Vec<CfmlValue>,
    _object: &CfmlValue,
) -> CfmlResult {
    if method.eq_ignore_ascii_case("newThread") {
        let mut m = java_shim_map("java.lang.thread");
        if let Some(r) = args.into_iter().next() {
            m.insert("__runnable".to_string(), r);
        }
        m.insert("__thread_name".to_string(), CfmlValue::string("rustcfml-worker".to_string()));
        return Ok(CfmlValue::strukt(m));
    }
    Ok(CfmlValue::Null)
}

/// Pure ExecutorService methods: lifecycle flags + pool statistics. The
/// executing methods (submit/execute/schedule/invokeAll) are VM-side. Returns
/// `Ok(Null)` for anything unrecognised so the caller can fall through.
/// `shutdown`/`shutdownNow` mutate the `__shutdown` flag; the VM caller writes
/// the returned struct back (via `method_this_writeback`) — signalled by the
/// second tuple element being `Some(new_self)`.
pub fn handle_java_executor_service_pure(
    method: &str,
    object: &CfmlValue,
) -> (CfmlResult, Option<CfmlValue>) {
    let m = method.to_ascii_lowercase();
    let is_shut = matches!(object, CfmlValue::Struct(s)
        if s.get("__shutdown").map(|v| v.is_true()).unwrap_or(false));
    match m.as_str() {
        "shutdown" => {
            let mut ns = match object {
                CfmlValue::Struct(s) => s.snapshot(),
                _ => ValueMap::default(),
            };
            ns.insert("__shutdown".to_string(), CfmlValue::Bool(true));
            (Ok(CfmlValue::Null), Some(CfmlValue::strukt(ns)))
        }
        "shutdownnow" => {
            let mut ns = match object {
                CfmlValue::Struct(s) => s.snapshot(),
                _ => ValueMap::default(),
            };
            ns.insert("__shutdown".to_string(), CfmlValue::Bool(true));
            // Returns the list of never-started tasks — always empty for us.
            (Ok(CfmlValue::array(vec![])), Some(CfmlValue::strukt(ns)))
        }
        "isshutdown" | "isterminated" => (Ok(CfmlValue::Bool(is_shut)), None),
        "isterminating" => (Ok(CfmlValue::Bool(false)), None),
        // awaitTermination(timeout, unit) → true (nothing is still running that
        // we track; tasks are detached real threads).
        "awaittermination" => (Ok(CfmlValue::Bool(true)), None),
        "purge" => (Ok(CfmlValue::Null), None),
        // Pool statistics — no real pool, so report benign zeros.
        "getactivecount" | "gettaskcount" | "getcompletedtaskcount" | "getpoolsize"
        | "getlargestpoolsize" => (Ok(CfmlValue::Int(0)), None),
        "getcorepoolsize" | "getmaximumpoolsize" => (Ok(CfmlValue::Int(0)), None),
        "getqueue" => (Ok(CfmlValue::array(vec![])), None),
        _ => (Ok(CfmlValue::Null), None),
    }
}

/// Map a Java single-abstract-method (SAM) interface name to the CFML method
/// name a `createDynamicProxy` target is expected to expose. Used by the
/// java.util.concurrent shim to invoke a proxied CFC. Matches the functional
/// interfaces ColdBox/Preside proxy: Callable→call, Runnable→run,
/// ThreadFactory→newThread, Function/BiFunction→apply, Consumer/BiConsumer→
/// accept, Supplier→get. Unknown interfaces default to `run`.
pub fn sam_method_for_interface(iface: &str) -> String {
    let last = iface
        .rsplit(['.', '$'])
        .next()
        .unwrap_or(iface)
        .to_ascii_lowercase();
    match last.as_str() {
        "callable" => "call",
        "runnable" => "run",
        "threadfactory" => "newThread",
        "supplier" => "get",
        "function" | "bifunction" | "futurefunction" => "apply",
        "consumer" | "biconsumer" => "accept",
        _ => "run",
    }
    .to_string()
}

/// `org.mindrot.jbcrypt.BCrypt` construction. Returns a marker shim; the actual
/// gensalt/hashpw/checkpw method calls are routed (VM-side) to the pure-Rust
/// bcrypt builtins (the crypto crate lives in cfml-stdlib, not cfml-vm). This
/// lets legacy Preside's BCryptService work without a JVM while the canonical
/// BCryptHash()/BCryptVerify() BIFs give native bcrypt to all RustCFML code.
pub fn handle_java_bcrypt(
    _method: &str,
    _args: Vec<CfmlValue>,
    _object: &CfmlValue,
) -> CfmlResult {
    let mut shim = ValueMap::default();
    shim.insert(
        "__java_class".to_string(),
        CfmlValue::string("org.mindrot.jbcrypt.bcrypt".to_string()),
    );
    shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    Ok(CfmlValue::strukt(shim))
}

/// `org.yaml.snakeyaml.Yaml` construction. Returns a marker shim; the `load`
/// method is routed (VM-side) to the native `yamlDeserialize` builtin. The jar
/// path arg is ignored. Lets legacy Preside's cfflow YamlParser work without a
/// JVM while the canonical yamlDeserialize()/yamlSerialize() BIFs give YAML to
/// all RustCFML code.
pub fn handle_java_yaml(
    _method: &str,
    _args: Vec<CfmlValue>,
    _object: &CfmlValue,
) -> CfmlResult {
    let mut shim = ValueMap::default();
    shim.insert(
        "__java_class".to_string(),
        CfmlValue::string("org.yaml.snakeyaml.yaml".to_string()),
    );
    shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    Ok(CfmlValue::strukt(shim))
}

/// `ca.vanmulligen.json.schema.Validator` construction. Returns an unconfigured
/// marker shim; `.init(schema, baseUri)` (VM-routed) returns a configured shim,
/// and `.isValid(json)` validates via the native validateJSON builtin. Lets
/// legacy Preside's cfflow JsonSchemaValidator work without a JVM.
pub fn handle_java_jsonvalidator(
    _method: &str,
    _args: Vec<CfmlValue>,
    _object: &CfmlValue,
) -> CfmlResult {
    let mut shim = ValueMap::default();
    shim.insert(
        "__java_class".to_string(),
        CfmlValue::string("ca.vanmulligen.json.schema.validator".to_string()),
    );
    shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    Ok(CfmlValue::strukt(shim))
}

pub fn handle_java_messagedigest(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" | "getinstance" => {
            let algorithm = args
                .first()
                .map(|a| a.as_string().to_lowercase())
                .unwrap_or_else(|| "sha-256".to_string());
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.security.messagedigest".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert("__algorithm".to_string(), CfmlValue::string(algorithm));
            shim.insert("__data".to_string(), CfmlValue::string(String::new()));
            Ok(CfmlValue::strukt(shim))
        }
        "update" => {
            // Real Java MessageDigest.update takes a byte[]. We accept both
            // Binary (from "...".getBytes()) and String (lenient) so Lucee and
            // RustCFML run the same interop code without rewrites.
            if let CfmlValue::Struct(ref shim) = object {
                let current = shim
                    .get("__data")
                    .map(|d| d.as_string())
                    .unwrap_or_default();
                let input = match args.first() {
                    Some(CfmlValue::Binary(b)) => String::from_utf8_lossy(b).to_string(),
                    Some(v) => v.as_string(),
                    None => String::new(),
                };
                let mut new_shim = shim.snapshot();
                new_shim.insert(
                    "__data".to_string(),
                    CfmlValue::string(format!("{}{}", current, input)),
                );
                Ok(CfmlValue::strukt(new_shim))
            } else {
                Ok(CfmlValue::Null)
            }
        }
        "digest" => {
            if let CfmlValue::Struct(ref shim) = object {
                let data = shim
                    .get("__data")
                    .map(|d| d.as_string())
                    .unwrap_or_default();
                Ok(CfmlValue::Binary(data.into_bytes()))
            } else {
                Ok(CfmlValue::Null)
            }
        }
        "isequal" => {
            // Real Java MessageDigest.isEqual compares two byte[] for content
            // equality. Args here are almost always Binary (from `.getBytes()`),
            // and Binary stringifies to the constant "<Binary>" — so comparing
            // via as_string() made ANY two byte arrays compare equal, defeating
            // JWT signature / password verification. Compare the raw bytes.
            if args.len() >= 2 {
                fn to_bytes(v: &CfmlValue) -> Vec<u8> {
                    match v {
                        CfmlValue::Binary(b) => b.clone(),
                        other => other.as_string().into_bytes(),
                    }
                }
                Ok(CfmlValue::Bool(to_bytes(&args[0]) == to_bytes(&args[1])))
            } else {
                Ok(CfmlValue::Null)
            }
        }
        "reset" => {
            if let CfmlValue::Struct(ref shim) = object {
                let mut new_shim = shim.snapshot();
                new_shim.insert("__data".to_string(), CfmlValue::string(String::new()));
                Ok(CfmlValue::strukt(new_shim))
            } else {
                Ok(CfmlValue::Null)
            }
        }
        _ => Ok(CfmlValue::Null),
    }
}

pub fn handle_java_uuid(method: &str, _args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    match method {
        "init" | "randomuuid" => {
            let uuid = format!("{:032x}", rand_u128());
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.util.uuid".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert("__uuid".to_string(), CfmlValue::string(uuid));
            Ok(CfmlValue::strukt(shim))
        }
        "tostring" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::String(uuid)) = shim.get("__uuid") {
                    if uuid.len() >= 32 {
                        let formatted = format!(
                            "{}-{}-{}-{}-{}",
                            &uuid[0..8],
                            &uuid[8..12],
                            &uuid[12..16],
                            &uuid[16..20],
                            &uuid[20..32]
                        );
                        return Ok(CfmlValue::string(formatted));
                    }
                }
            }
            Ok(CfmlValue::Null)
        }
        "getversion" => Ok(CfmlValue::Int(4)),
        "getvariant" => Ok(CfmlValue::Int(2)),
        _ => Ok(CfmlValue::Null),
    }
}

pub fn handle_java_date(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    // java.util.Date shim. Lucee/Adobe commonly construct it from epoch millis
    // (`new Date(0)` as a UTC base date for date math) and read `getTime()`.
    // State: `__millis` (epoch milliseconds, as a Java `long`). (PR #163.)
    let to_millis = |v: &CfmlValue| -> i64 {
        match v {
            CfmlValue::Int(n) => *n,
            CfmlValue::Double(d) => *d as i64,
            other => other.as_string().trim().parse::<i64>().unwrap_or(0),
        }
    };
    match method {
        "init" => {
            // `Date()` (no arg) = now; `Date(long)` = the given epoch millis.
            let millis = match args.first() {
                Some(v) => to_millis(v),
                None => std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
            };
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.util.date".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert("__millis".to_string(), CfmlValue::Int(millis));
            Ok(CfmlValue::strukt(shim))
        }
        "gettime" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(v) = shim.get("__millis") {
                    return Ok(CfmlValue::Int(to_millis(&v)));
                }
            }
            Ok(CfmlValue::Int(0))
        }
        "settime" => {
            if let CfmlValue::Struct(ref shim) = object {
                let millis = args.first().map(&to_millis).unwrap_or(0);
                let mut ns = shim.snapshot();
                ns.insert("__millis".to_string(), CfmlValue::Int(millis));
                return Ok(CfmlValue::strukt(ns));
            }
            Ok(CfmlValue::Null)
        }
        _ => Ok(CfmlValue::Null),
    }
}

pub fn handle_java_thread(method: &str, _args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    // "threadgroup" is a nested shim for java.lang.ThreadGroup accessed via
    // Thread.getThreadGroup(). We route its own methods here too.
    if let CfmlValue::Struct(ref shim) = object {
        if shim
            .get("__java_class")
            .map(|v| v.as_string())
            .unwrap_or_default()
            == "java.lang.threadgroup"
        {
            return match method {
                "getname" => Ok(shim
                    .get("__name")
                    .unwrap_or(CfmlValue::string("main".to_string()))),
                _ => Ok(CfmlValue::Null),
            };
        }
    }
    match method {
        "init" | "currentthread" => {
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.lang.thread".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert("__name".to_string(), CfmlValue::string("main".to_string()));
            Ok(CfmlValue::strukt(shim))
        }
        "getname" => {
            if let CfmlValue::Struct(ref shim) = object {
                Ok(shim
                    .get("__name")
                    .unwrap_or(CfmlValue::string("main".to_string())))
            } else {
                Ok(CfmlValue::string("main".to_string()))
            }
        }
        "getthreadgroup" => {
            let mut tg = ValueMap::default();
            tg.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.lang.threadgroup".to_string()),
            );
            tg.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            tg.insert("__name".to_string(), CfmlValue::string("main".to_string()));
            Ok(CfmlValue::strukt(tg))
        }
        "getpriority" => Ok(CfmlValue::Int(5)),
        "isdaemon" => Ok(CfmlValue::Bool(false)),
        "sleep" => Ok(CfmlValue::Null),
        _ => Ok(CfmlValue::Null),
    }
}

/// Java `InetAddress.isLoopbackAddress()` semantics: true for the entire IPv4
/// 127.0.0.0/8 block and the IPv6 loopback `::1` (in any zero-padded / compressed
/// form), plus the literal host "localhost". Rust's address parsers canonicalise
/// "::1", "0:0:0:0:0:0:0:1" and "0000:…:0001" to the same value.
fn is_loopback_addr(addr: &str) -> bool {
    let a = addr.trim().to_lowercase();
    if a == "localhost" {
        return true;
    }
    if let Ok(v4) = a.parse::<std::net::Ipv4Addr>() {
        return v4.is_loopback();
    }
    if let Ok(v6) = a.parse::<std::net::Ipv6Addr>() {
        return v6.is_loopback();
    }
    false
}

pub fn handle_java_inetaddress(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    match method {
        // `createObject("java", "java.net.InetAddress")` lands here via the
        // "init" path. Java's InetAddress has no public constructor, but we must
        // still return a non-null class-reference shim so the static factory
        // methods can be dispatched on it (e.g.
        // `createObject(...).getLocalHost()`); otherwise the receiver is null
        // and the chained call throws since v0.119.0.
        "init" => {
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.net.inetaddress".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            Ok(CfmlValue::strukt(shim))
        }
        "getlocalhost" => {
            let hostname = std::env::var("HOSTNAME")
                .or_else(|_| std::env::var("HOST"))
                .unwrap_or_else(|_| "localhost".to_string());
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.net.inetaddress".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert(
                "__hostname".to_string(),
                CfmlValue::string(hostname.clone()),
            );
            shim.insert(
                "__address".to_string(),
                CfmlValue::string("127.0.0.1".to_string()),
            );
            Ok(CfmlValue::strukt(shim))
        }
        "getbyname" => {
            let hostname = args
                .first()
                .map(|a| a.as_string())
                .unwrap_or_else(|| "localhost".to_string());
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.net.inetaddress".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert(
                "__hostname".to_string(),
                CfmlValue::string(hostname.clone()),
            );
            // Resolve the address. We do no DNS, so: an IP literal is stored as-is
            // (the input was already an address); "localhost" resolves to the IPv4
            // loopback like real Java; any other hostname round-trips the name as a
            // best effort. isLoopbackAddress()/getHostAddress() read this back.
            let address = if hostname.eq_ignore_ascii_case("localhost") {
                "127.0.0.1".to_string()
            } else {
                hostname
            };
            shim.insert(
                "__address".to_string(),
                CfmlValue::string(address),
            );
            Ok(CfmlValue::strukt(shim))
        }
        "isloopbackaddress" => {
            if let CfmlValue::Struct(ref shim) = object {
                let addr = shim
                    .get("__address")
                    .map(|v| v.as_string())
                    .unwrap_or_default();
                return Ok(CfmlValue::Bool(is_loopback_addr(&addr)));
            }
            Ok(CfmlValue::Bool(false))
        }
        "gethostname" | "gethostaddress" | "getcanonicalhostname" | "tostring" => {
            if let CfmlValue::Struct(ref shim) = object {
                let key = match method {
                    "gethostname" | "tostring" => "__hostname",
                    "gethostaddress" => "__address",
                    _ => "__hostname",
                };
                Ok(shim
                    .get(key)
                    .unwrap_or(CfmlValue::string("localhost".to_string())))
            } else {
                Ok(CfmlValue::string("localhost".to_string()))
            }
        }
        _ => Ok(CfmlValue::Null),
    }
}

/// Default TCP port for a scheme, matching java.net.URL.getDefaultPort().
/// Unknown schemes return -1 (Java's sentinel for "no default").
fn url_default_port(protocol: &str) -> i64 {
    match protocol.to_ascii_lowercase().as_str() {
        "http" => 80,
        "https" => 443,
        "ftp" => 21,
        "file" => -1,
        _ => -1,
    }
}

/// Parse a spec string into the java.net.URL component parts. No JVM, no I/O —
/// a best-effort structural parse covering the accessor surface real CFML code
/// uses (protocol/host/port/path/query/ref/authority/file). Follows java.net.URL
/// conventions: getPort() is -1 when the URL omits the port (default port is a
/// separate getDefaultPort()); getFile() is path+"?"+query; getPath() is bare.
fn parse_url_parts(spec: &str) -> ValueMap {
    let mut m = ValueMap::default();
    let mut rest = spec.trim();

    // protocol: leading "<scheme>:" (scheme is [A-Za-z][A-Za-z0-9+.-]*)
    let mut protocol = String::new();
    if let Some(colon) = rest.find(':') {
        let candidate = &rest[..colon];
        if !candidate.is_empty()
            && candidate.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false)
            && candidate
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '.' || c == '-')
        {
            protocol = candidate.to_string();
            rest = &rest[colon + 1..];
        }
    }

    // fragment (ref): split off trailing "#..."
    let mut reference = String::new();
    if let Some(hash) = rest.find('#') {
        reference = rest[hash + 1..].to_string();
        rest = &rest[..hash];
    }

    // authority: present when the remainder begins with "//"
    let mut authority = String::new();
    let mut userinfo = String::new();
    let mut host = String::new();
    let mut port: i64 = -1;
    if let Some(after) = rest.strip_prefix("//") {
        let auth_end = after.find(['/', '?']).unwrap_or(after.len());
        authority = after[..auth_end].to_string();
        rest = &after[auth_end..];

        let mut auth_body = authority.as_str();
        if let Some(at) = auth_body.rfind('@') {
            userinfo = auth_body[..at].to_string();
            auth_body = &auth_body[at + 1..];
        }
        // host:port — an IPv6 literal is bracketed [::1]:8080
        if let Some(rb) = auth_body.rfind(']') {
            host = auth_body[..=rb].to_string();
            if let Some(colon) = auth_body[rb + 1..].find(':') {
                if let Ok(p) = auth_body[rb + 1 + colon + 1..].parse::<i64>() {
                    port = p;
                }
            }
        } else if let Some(colon) = auth_body.rfind(':') {
            host = auth_body[..colon].to_string();
            if let Ok(p) = auth_body[colon + 1..].parse::<i64>() {
                port = p;
            }
        } else {
            host = auth_body.to_string();
        }
    }

    // query: split off trailing "?..."
    let mut query = String::new();
    if let Some(q) = rest.find('?') {
        query = rest[q + 1..].to_string();
        rest = &rest[..q];
    }
    let path = rest.to_string();

    let file = if query.is_empty() {
        path.clone()
    } else {
        format!("{}?{}", path, query)
    };

    m.insert("__java_class".to_string(), CfmlValue::string("java.net.url".to_string()));
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__spec".to_string(), CfmlValue::string(spec.to_string()));
    m.insert("__protocol".to_string(), CfmlValue::string(protocol));
    m.insert("__authority".to_string(), CfmlValue::string(authority));
    m.insert("__userinfo".to_string(), CfmlValue::string(userinfo));
    m.insert("__host".to_string(), CfmlValue::string(host));
    m.insert("__port".to_string(), CfmlValue::Int(port));
    m.insert("__path".to_string(), CfmlValue::string(path));
    m.insert("__query".to_string(), CfmlValue::string(query));
    m.insert("__ref".to_string(), CfmlValue::string(reference));
    m.insert("__file".to_string(), CfmlValue::string(file));
    m
}

/// Value-equality for two `java.net.URL` shims, matching `java.net.URL.equals()`
/// semantics: same protocol (case-insensitive), same host (case-insensitive),
/// same effective port (a `-1` resolves to the scheme default), same file
/// (path+query) and same ref (anchor). Used by both the `equals()` method and
/// the CFML `eq` operator so two URLs built from the same spec compare equal and
/// two different URLs do not (GH #238). Non-URL operands compare unequal.
pub fn url_shim_equals(a: &CfmlValue, b: &CfmlValue) -> bool {
    let is_url = |v: &CfmlValue| -> bool {
        matches!(v, CfmlValue::Struct(s)
            if s.get("__java_class")
                .map(|c| c.as_string().eq_ignore_ascii_case("java.net.url"))
                .unwrap_or(false))
    };
    if !is_url(a) || !is_url(b) {
        return false;
    }
    let get = |v: &CfmlValue, k: &str| -> String {
        if let CfmlValue::Struct(s) = v {
            s.get(k).map(|x| x.as_string()).unwrap_or_default()
        } else {
            String::new()
        }
    };
    let port = |v: &CfmlValue| -> i64 {
        let raw = if let CfmlValue::Struct(s) = v {
            match s.get("__port") {
                Some(CfmlValue::Int(i)) => i,
                Some(o) => o.as_string().trim().parse().unwrap_or(-1),
                None => -1,
            }
        } else {
            -1
        };
        if raw == -1 {
            url_default_port(&get(v, "__protocol"))
        } else {
            raw
        }
    };
    get(a, "__protocol").eq_ignore_ascii_case(&get(b, "__protocol"))
        && get(a, "__host").eq_ignore_ascii_case(&get(b, "__host"))
        && port(a) == port(b)
        && get(a, "__file") == get(b, "__file")
        && get(a, "__ref") == get(b, "__ref")
}

/// `java.net.URL` shim (no JVM). Covers the parse/accessor surface; network I/O
/// (openConnection/openStream/getContent) throws — there is no JVM behind it.
/// See GH #231.
pub fn handle_java_url(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    let get = |key: &str| -> CfmlValue {
        if let CfmlValue::Struct(ref s) = object {
            s.get(key).unwrap_or(CfmlValue::Null)
        } else {
            CfmlValue::Null
        }
    };
    match method {
        // `createObject("java","java.net.URL")` lands here with empty args: return
        // the bare class-reference shim so a chained `.init(spec)` can build it.
        "init" => {
            match args.len() {
                0 => {
                    let mut shim = ValueMap::default();
                    shim.insert(
                        "__java_class".to_string(),
                        CfmlValue::string("java.net.url".to_string()),
                    );
                    shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
                    Ok(CfmlValue::strukt(shim))
                }
                1 => Ok(CfmlValue::strukt(parse_url_parts(&args[0].as_string()))),
                // URL(protocol, host, port, file) / URL(protocol, host, file):
                // reassemble into a spec then parse for uniform accessors.
                _ => {
                    let protocol = args[0].as_string();
                    let host = args[1].as_string();
                    let (port, file) = if args.len() >= 4 {
                        (args[2].as_string(), args[3].as_string())
                    } else {
                        (String::new(), args[2].as_string())
                    };
                    let hostport = if port.is_empty() || port == "-1" {
                        host
                    } else {
                        format!("{}:{}", host, port)
                    };
                    let file = if file.is_empty() || file.starts_with('/') || file.starts_with('?') {
                        file
                    } else {
                        format!("/{}", file)
                    };
                    let spec = format!("{}://{}{}", protocol, hostport, file);
                    Ok(CfmlValue::strukt(parse_url_parts(&spec)))
                }
            }
        }
        "getprotocol" => Ok(get("__protocol")),
        "gethost" => Ok(get("__host")),
        "getport" => Ok(match get("__port") {
            CfmlValue::Null => CfmlValue::Int(-1),
            v => v,
        }),
        "getdefaultport" => {
            let protocol = get("__protocol").as_string();
            Ok(CfmlValue::Int(url_default_port(&protocol)))
        }
        "getpath" => Ok(get("__path")),
        "getquery" => {
            // Java returns null (not "") when there is no query component.
            match get("__query") {
                CfmlValue::String(s) if s.is_empty() => Ok(CfmlValue::Null),
                v => Ok(v),
            }
        }
        "getref" => match get("__ref") {
            CfmlValue::String(s) if s.is_empty() => Ok(CfmlValue::Null),
            v => Ok(v),
        },
        "getfile" => Ok(get("__file")),
        "getauthority" => match get("__authority") {
            CfmlValue::String(s) if s.is_empty() => Ok(CfmlValue::Null),
            v => Ok(v),
        },
        "getuserinfo" => match get("__userinfo") {
            CfmlValue::String(s) if s.is_empty() => Ok(CfmlValue::Null),
            v => Ok(v),
        },
        "tostring" | "toexternalform" => Ok(get("__spec")),
        // java.net.URL.equals() — value equality, not identity. TestBox's
        // equalize() falls here once isStruct() reports false for the shim
        // (GH #238).
        "equals" => {
            let other = args.into_iter().next().unwrap_or(CfmlValue::Null);
            Ok(CfmlValue::Bool(url_shim_equals(object, &other)))
        }
        "openconnection" | "openstream" | "getcontent" | "getinputstream" => {
            Err(CfmlError::runtime(format!(
                "java.net.URL.{}() requires network I/O, which RustCFML has no JVM \
                 to provide. Use <cfhttp> for HTTP requests.",
                method
            )))
        }
        _ => Ok(CfmlValue::Null),
    }
}

pub fn handle_java_file(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    match method {
        "init" => {
            let path = args.first().map(|a| a.as_string()).unwrap_or_default();
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.io.file".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert("__path".to_string(), CfmlValue::string(path));
            Ok(CfmlValue::strukt(shim))
        }
        "tostring" => {
            // java.io.File.toString() returns the original path as given.
            if let CfmlValue::Struct(ref shim) = object {
                return Ok(shim
                    .get("__path")
                    .unwrap_or(CfmlValue::string(String::new())));
            }
            Ok(CfmlValue::string(String::new()))
        }
        "getabsolute_path" | "getabsolutepath" => {
            // getAbsolutePath() makes the path absolute but does NOT collapse
            // `.`/`..` segments (Java leaves them for getCanonicalPath).
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::String(path)) = shim.get("__path") {
                    let p = std::path::Path::new(path.as_str());
                    if p.is_absolute() {
                        return Ok(CfmlValue::string(path.to_string()));
                    }
                    if let Ok(cwd) = std::env::current_dir() {
                        return Ok(CfmlValue::string(
                            cwd.join(path.as_str()).to_string_lossy().to_string(),
                        ));
                    }
                }
            }
            Ok(CfmlValue::string(String::new()))
        }
        "getcanonicalpath" => {
            // getCanonicalPath() makes the path absolute AND lexically resolves
            // `.` and `..` segments and strips a trailing separator (Wheels'
            // path-traversal guard relies on this to detect escapes from the
            // assets directory). We resolve lexically rather than via
            // std::fs::canonicalize so it works for paths that don't exist on
            // disk (the traversal targets in the security spec).
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::String(path)) = shim.get("__path") {
                    let abs = {
                        let p = std::path::Path::new(path.as_str());
                        if p.is_absolute() {
                            std::path::PathBuf::from(path.as_str())
                        } else if let Ok(cwd) = std::env::current_dir() {
                            cwd.join(path.as_str())
                        } else {
                            std::path::PathBuf::from(path.as_str())
                        }
                    };
                    return Ok(CfmlValue::string(lexically_normalize(&abs)));
                }
            }
            Ok(CfmlValue::string(String::new()))
        }
        "isabsolute" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::String(path)) = shim.get("__path") {
                    return Ok(CfmlValue::Bool(std::path::Path::new(path.as_str()).is_absolute()));
                }
            }
            Ok(CfmlValue::Bool(false))
        }
        "exists" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::String(path)) = shim.get("__path") {
                    return Ok(CfmlValue::Bool(std::path::Path::new(path.as_str()).exists()));
                }
            }
            Ok(CfmlValue::Bool(false))
        }
        "isfile" | "is_directory" | "isdirectory" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::String(path)) = shim.get("__path") {
                    let p = std::path::Path::new(path.as_str());
                    return Ok(CfmlValue::Bool(if method == "isfile" {
                        p.is_file()
                    } else {
                        p.is_dir()
                    }));
                }
            }
            Ok(CfmlValue::Bool(false))
        }
        "getname" | "lastmodified" | "length" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::String(path)) = shim.get("__path") {
                    if let Ok(meta) = std::fs::metadata(path.as_str()) {
                        if method == "getname" {
                            if let Some(n) = std::path::Path::new(path.as_str()).file_name() {
                                return Ok(CfmlValue::string(n.to_string_lossy().to_string()));
                            }
                        } else if method == "lastmodified" {
                            if let Ok(t) = meta.modified() {
                                let d = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                                return Ok(CfmlValue::Double(d.as_millis() as f64));
                            }
                        } else {
                            return Ok(CfmlValue::Int(meta.len() as i64));
                        }
                    }
                }
            }
            Ok(CfmlValue::Int(0))
        }
        "tourl" | "touri" => {
            // File.toURL()/toURI() returns a java.net.URL/URI. cbjavaloader only
            // stuffs the result into a URL[] it hands to a (deferred) class
            // loader and never dereferences it, so a `file:` string suffices.
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::String(path)) = shim.get("__path") {
                    return Ok(CfmlValue::string(format!("file:{}", path)));
                }
            }
            Ok(CfmlValue::string(String::new()))
        }
        "topath" => {
            // File.toPath() returns a java.nio.file.Path. This is the portable
            // alternative to Paths.get(…), which Lucee can't dispatch to
            // cleanly due to its String/varargs signature.
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(path) = shim.get("__path") {
                    let mut ps = ValueMap::default();
                    ps.insert(
                        "__java_class".to_string(),
                        CfmlValue::string("java.nio.file.paths".to_string()),
                    );
                    ps.insert("__java_shim".to_string(), CfmlValue::Bool(true));
                    ps.insert("__path".to_string(), path.clone());
                    return Ok(CfmlValue::strukt(ps));
                }
            }
            Ok(CfmlValue::Null)
        }
        _ => Ok(CfmlValue::Null),
    }
}

/// Lexically resolve `.` and `..` components of an absolute path (no disk
/// access), strip any trailing separator, and return the result as a string.
/// Mirrors the lexical part of java.io.File.getCanonicalPath for the common
/// case the path-traversal guard needs. A leading `..` (escaping the root) is
/// dropped, matching Java's resolution against the filesystem root.
fn lexically_normalize(path: &std::path::Path) -> String {
    use std::path::Component;
    let mut out: Vec<std::ffi::OsString> = Vec::new();
    let mut prefix = String::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => prefix = p.as_os_str().to_string_lossy().to_string(),
            Component::RootDir => {} // re-added when joining below
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(c) => out.push(c.to_os_string()),
        }
    }
    let joined = out
        .iter()
        .map(|s| s.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    if prefix.is_empty() {
        format!("/{}", joined)
    } else {
        // Windows-style prefix (e.g. C:) — keep it, separate with `\`.
        format!("{}\\{}", prefix, joined.replace('/', "\\"))
    }
}

/// Extract a filesystem path string from an argument that may be either a
/// java.nio.file.Path / java.io.File shim struct (carrying `__path`) or a
/// plain string.
fn java_path_arg(arg: &CfmlValue) -> Option<String> {
    match arg {
        CfmlValue::String(s) => Some(s.to_string()),
        CfmlValue::Struct(shim) => match shim.get("__path") {
            Some(CfmlValue::String(p)) => Some(p.to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// java.nio.file.Files — static helper class. The CreateObject shim carries no
/// state; the path is always passed as the first argument (a Path shim or a
/// string). Only the members Wheels' plugin loader uses are implemented:
/// isSymbolicLink (does NOT follow links) and delete (removes the link/file
/// itself without following symlinks).
pub fn handle_java_files(method: &str, args: Vec<CfmlValue>, _object: &CfmlValue) -> CfmlResult {
    match method {
        "init" => {
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.nio.file.files".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            Ok(CfmlValue::strukt(shim))
        }
        "issymboliclink" => {
            if let Some(path) = args.first().and_then(java_path_arg) {
                let is_link = std::fs::symlink_metadata(&path)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                return Ok(CfmlValue::Bool(is_link));
            }
            Ok(CfmlValue::Bool(false))
        }
        "delete" => {
            // Files.delete removes the symlink/file itself and does NOT follow
            // links. On Unix std::fs::remove_file removes a symlink without
            // touching its target; fall back to remove_dir for an empty dir.
            if let Some(path) = args.first().and_then(java_path_arg) {
                let p = std::path::Path::new(&path);
                let meta = std::fs::symlink_metadata(p);
                let res = match meta {
                    Ok(m) if m.file_type().is_dir() => std::fs::remove_dir(p),
                    _ => std::fs::remove_file(p),
                };
                if let Err(e) = res {
                    return Err(CfmlError::runtime(format!(
                        "java.nio.file.Files.delete failed for '{}': {}",
                        path, e
                    )));
                }
            }
            Ok(CfmlValue::Null)
        }
        "exists" => {
            // Files.exists(path) — follows symlinks like the real API default.
            if let Some(path) = args.first().and_then(java_path_arg) {
                return Ok(CfmlValue::Bool(std::path::Path::new(&path).exists()));
            }
            Ok(CfmlValue::Bool(false))
        }
        "isdirectory" => {
            if let Some(path) = args.first().and_then(java_path_arg) {
                return Ok(CfmlValue::Bool(std::path::Path::new(&path).is_dir()));
            }
            Ok(CfmlValue::Bool(false))
        }
        _ => Ok(CfmlValue::Null),
    }
}

/// java.lang.ProcessBuilder — supports the `init(List<String> command)` →
/// `start()` → Process pattern Wheels uses to shell out to `ln -s`. ACF/Lucee
/// resolve ProcessBuilder's List ctor cleanly (unlike Files' varargs), so the
/// tests deliberately go through it. We collapse start()+waitFor() by running
/// the command to completion synchronously inside start() — the caller always
/// calls waitFor() immediately, so this is behaviourally equivalent.
pub fn handle_java_processbuilder(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => {
            // command is supplied as an array (List) or as varargs strings.
            let cmd: Vec<CfmlValue> = match args.first() {
                Some(CfmlValue::Array(a)) => a.iter().collect(),
                _ => args.clone(),
            };
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.lang.processbuilder".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert("__command".to_string(), CfmlValue::array(cmd));
            Ok(CfmlValue::strukt(shim))
        }
        "command" => {
            // ProcessBuilder.command(list) sets and returns the builder.
            if let CfmlValue::Struct(ref shim) = object {
                let cmd: Vec<CfmlValue> = match args.first() {
                    Some(CfmlValue::Array(a)) => a.iter().collect(),
                    _ => args.clone(),
                };
                shim.insert("__command".to_string(), CfmlValue::array(cmd));
                return Ok(CfmlValue::Struct(shim.clone()));
            }
            Ok(object.clone())
        }
        "start" => {
            if let CfmlValue::Struct(ref shim) = object {
                let parts: Vec<String> = match shim.get("__command") {
                    Some(CfmlValue::Array(a)) => a.iter().map(|v| v.as_string()).collect(),
                    _ => vec![],
                };
                if parts.is_empty() {
                    return Err(CfmlError::runtime(
                        "ProcessBuilder.start() called with empty command".to_string(),
                    ));
                }
                let mut command = std::process::Command::new(&parts[0]);
                command.args(&parts[1..]);
                let exit_code = match command.status() {
                    Ok(status) => status.code().unwrap_or(-1),
                    Err(e) => {
                        return Err(CfmlError::runtime(format!(
                            "ProcessBuilder.start() failed to launch '{}': {}",
                            parts[0], e
                        )));
                    }
                };
                let mut proc = ValueMap::default();
                proc.insert(
                    "__java_class".to_string(),
                    CfmlValue::string("java.lang.process".to_string()),
                );
                proc.insert("__java_shim".to_string(), CfmlValue::Bool(true));
                proc.insert("__exitcode".to_string(), CfmlValue::Int(exit_code as i64));
                return Ok(CfmlValue::strukt(proc));
            }
            Ok(CfmlValue::Null)
        }
        _ => Ok(CfmlValue::Null),
    }
}

/// java.lang.Process — the handle returned by ProcessBuilder.start(). Since we
/// run the command synchronously, waitFor()/exitValue() just report the
/// captured exit code.
pub fn handle_java_process(method: &str, _args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    match method {
        "waitfor" | "exitvalue" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(code) = shim.get("__exitcode") {
                    return Ok(code);
                }
            }
            Ok(CfmlValue::Int(0))
        }
        "isalive" => Ok(CfmlValue::Bool(false)),
        "destroy" | "destroyforcibly" => Ok(CfmlValue::Null),
        _ => Ok(CfmlValue::Null),
    }
}

pub fn handle_java_system(method: &str, args: Vec<CfmlValue>, _object: &CfmlValue) -> CfmlResult {
    match method {
        "init" => {
            // java.lang.System is a static-only class in real Java, but we
            // return a shim struct so both init() and static-style access work.
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.lang.system".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            // Expose `out` as a nested shim so `system.out.println(...)` works.
            let mut out = ValueMap::default();
            out.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.lang.system.out".to_string()),
            );
            out.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert("out".to_string(), CfmlValue::strukt(out));
            // Expose `err` too, so `System.err.println(...)` works (ColdBox's
            // ConsoleAppender writes error-level output to stderr).
            let mut err = ValueMap::default();
            err.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.lang.system.err".to_string()),
            );
            err.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert("err".to_string(), CfmlValue::strukt(err));
            Ok(CfmlValue::strukt(shim))
        }
        "currenttimemillis" => {
            Ok(CfmlValue::Double(cfml_common::clock::now_unix_millis() as f64))
        }
        "nanotime" => {
            Ok(CfmlValue::Double(cfml_common::clock::now_unix_nanos() as f64))
        }
        "identityhashcode" => {
            // java.lang.System.identityHashCode(obj) — a stable per-object
            // identity hash: same reference -> same value, distinct
            // (reference-typed) objects -> distinct values. Used by CacheBox's
            // CacheFactory (factoryId) and TestBox's assertSame/assertNotSame.
            // Reference types (Struct — incl. live CFC instances — Array, Query,
            // NativeObject, Function) carry an Arc backing pointer that is
            // exactly this identity. Value types have no Java-object identity,
            // so we fold a content hash; either way we never return null.
            let obj = args.first().cloned().unwrap_or(CfmlValue::Null);
            let raw: u64 = match &obj {
                CfmlValue::Struct(s) => s.backing_ptr() as u64,
                CfmlValue::Array(a) => a.backing_ptr() as u64,
                CfmlValue::Query(q) => q.backing_ptr() as u64,
                CfmlValue::NativeObject(n) => std::sync::Arc::as_ptr(n) as *const () as u64,
                CfmlValue::Function(f) => {
                    std::sync::Arc::as_ptr(f) as *const () as u64
                }
                // Value types (and the cloned-by-value Component) have no
                // shared backing store, so hash their content for a stable,
                // non-null result.
                other => {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    other.as_string().hash(&mut h);
                    h.finish()
                }
            };
            // Fold to a positive 31-bit int (Java identity hashes are ints).
            let folded = (raw ^ (raw >> 32)) & 0x7fff_ffff;
            Ok(CfmlValue::Int(folded as i64))
        }
        "getproperty" => {
            // Some callers pass the key as the first "real" arg, but member
            // dispatch prepends the object — skip leading shim structs.
            let key = args
                .iter()
                .find_map(|a| match a {
                    CfmlValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let val = match key.to_lowercase().as_str() {
                "os.name" => std::env::consts::OS.to_string(),
                "file.separator" => std::path::MAIN_SEPARATOR.to_string(),
                "path.separator" => {
                    if cfg!(unix) {
                        ":".to_string()
                    } else {
                        ";".to_string()
                    }
                }
                "user.dir" => std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default(),
                "user.home" => std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_default(),
                "java.version" => "rustcfml".to_string(),
                _ => String::new(),
            };
            Ok(CfmlValue::string(val))
        }
        "getenv" => {
            // Single-arg form returns the value for that key. No-arg form returns
            // a Map (real Java: `Map<String,String>`), so it MUST be a java.util
            // map shim — not a plain struct — or member calls on the result like
            // `System.getenv().get("X")` (Preside's `_getEnvironmentVariable`) and
            // `.containsKey`/`.keySet` would not dispatch and silently return null.
            let key = args.iter().find_map(|a| match a {
                CfmlValue::String(s) => Some(s.clone()),
                _ => None,
            });
            match key {
                Some(k) => Ok(CfmlValue::string(std::env::var(k.as_str()).unwrap_or_default())),
                None => {
                    let mut env = ValueMap::default();
                    env.insert(
                        "__java_class".to_string(),
                        CfmlValue::string("java.util.linkedhashmap".to_string()),
                    );
                    env.insert("__java_shim".to_string(), CfmlValue::Bool(true));
                    for (k, v) in std::env::vars() {
                        env.insert(k, CfmlValue::string(v));
                    }
                    Ok(CfmlValue::strukt(env))
                }
            }
        }
        _ => Ok(CfmlValue::Null),
    }
}

pub fn handle_java_stringbuilder(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => {
            let init = args.first().map(|a| a.as_string()).unwrap_or_default();
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.lang.stringbuilder".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert("__buffer".to_string(), CfmlValue::string(init));
            Ok(CfmlValue::strukt(shim))
        }
        "append" => {
            if let CfmlValue::Struct(ref shim) = object {
                let cur = shim
                    .get("__buffer")
                    .map(|b| b.as_string())
                    .unwrap_or_default();
                let app = args.first().map(|a| a.as_string()).unwrap_or_default();
                // Mutate the buffer IN PLACE through the shared handle and return
                // the same builder (Java's `append` returns `this`). A snapshot +
                // new-struct return only survived reassignment writeback
                // (`sb = sb.append(x)`); when the builder was passed into a
                // function (`fn(sb){ sb.append(x); }`, e.g. MockBox's
                // generateMethodsFromMD) the caller's instance never saw the
                // append, so generated stubs came out empty.
                shim.insert(
                    "__buffer".to_string(),
                    CfmlValue::string(format!("{}{}", cur, app)),
                );
                Ok(object.clone())
            } else {
                Ok(CfmlValue::Null)
            }
        }
        "tostring" => {
            if let CfmlValue::Struct(ref shim) = object {
                Ok(shim
                    .get("__buffer")
                    .unwrap_or(CfmlValue::string(String::new())))
            } else {
                Ok(CfmlValue::string(String::new()))
            }
        }
        "length" => {
            if let CfmlValue::Struct(ref shim) = object {
                let b = shim
                    .get("__buffer")
                    .map(|x| x.as_string())
                    .unwrap_or_default();
                Ok(CfmlValue::Int(b.len() as i64))
            } else {
                Ok(CfmlValue::Int(0))
            }
        }
        "clear" => {
            if let CfmlValue::Struct(ref shim) = object {
                // Mutate in place + return the builder (see `append`).
                shim.insert("__buffer".to_string(), CfmlValue::string(String::new()));
                Ok(object.clone())
            } else {
                Ok(CfmlValue::Null)
            }
        }
        _ => Ok(CfmlValue::Null),
    }
}

// ---- TreeMap ----
pub fn handle_java_treemap(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    match method {
        "init" => {
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.util.treemap".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            if let Some(CfmlValue::Struct(init)) = args.first() {
                for (k, v) in init.iter() {
                    shim.insert(k.clone(), v.clone());
                }
            }
            Ok(CfmlValue::strukt(shim))
        }
        "put" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some((k, v)) = args.get(0).zip(args.get(1)) {
                    let mut ns = shim.snapshot();
                    ns.insert(k.as_string(), v.clone());
                    Ok(CfmlValue::strukt(ns))
                } else {
                    Ok(object.clone())
                }
            } else {
                Ok(CfmlValue::Null)
            }
        }
        "keyset" | "keys" => {
            if let CfmlValue::Struct(ref shim) = object {
                let mut ks: Vec<String> = shim
                    .iter()
                    .filter(|(k, _)| !k.starts_with("__"))
                    .map(|(k, _)| k.clone())
                    .collect();
                ks.sort(); // TreeMap = sorted key order
                Ok(CfmlValue::array(
                    ks.into_iter().map(CfmlValue::string).collect(),
                ))
            } else {
                Ok(CfmlValue::array(Vec::new()))
            }
        }
        "get" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(key) = args.first() {
                    let k = key.as_string();
                    return Ok(shim.get(&k).unwrap_or(CfmlValue::Null));
                }
            }
            Ok(CfmlValue::Null)
        }
        "size" | "len" => {
            if let CfmlValue::Struct(ref shim) = object {
                Ok(CfmlValue::Int(
                    shim.iter().filter(|(k, _)| !k.starts_with("__")).count() as i64,
                ))
            } else {
                Ok(CfmlValue::Int(0))
            }
        }
        "containskey" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(key) = args.first() {
                    let k = key.as_string();
                    return Ok(CfmlValue::Bool(shim.contains_key(&k)));
                }
            }
            Ok(CfmlValue::Bool(false))
        }
        "isempty" => {
            if let CfmlValue::Struct(ref shim) = object {
                Ok(CfmlValue::Bool(
                    shim.iter().all(|(k, _)| k.starts_with("__")),
                ))
            } else {
                Ok(CfmlValue::Bool(true))
            }
        }
        _ => Ok(CfmlValue::Null),
    }
}

pub fn handle_java_linkedhashmap(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => {
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.util.linkedhashmap".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            Ok(CfmlValue::strukt(shim))
        }
        "keyset" | "keys" => {
            if let CfmlValue::Struct(ref shim) = object {
                let ks: Vec<CfmlValue> = shim
                    .iter()
                    .filter(|(k, _)| !k.starts_with("__"))
                    .map(|(k, _)| CfmlValue::string(k.clone()))
                    .collect();
                Ok(CfmlValue::array(ks))
            } else {
                Ok(CfmlValue::array(Vec::new()))
            }
        }
        "get" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::String(k)) = args.first() {
                    Ok(shim.get(k).unwrap_or(CfmlValue::Null))
                } else {
                    Ok(CfmlValue::Null)
                }
            } else {
                Ok(CfmlValue::Null)
            }
        }
        "size" | "len" => {
            if let CfmlValue::Struct(ref shim) = object {
                Ok(CfmlValue::Int(
                    shim.iter().filter(|(k, _)| !k.starts_with("__")).count() as i64,
                ))
            } else {
                Ok(CfmlValue::Int(0))
            }
        }
        "containskey" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::String(k)) = args.first() {
                    Ok(CfmlValue::Bool(shim.contains_key(k)))
                } else {
                    Ok(CfmlValue::Bool(false))
                }
            } else {
                Ok(CfmlValue::Bool(false))
            }
        }
        "put" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some((k, v)) = args.get(0).zip(args.get(1)) {
                    let mut ns = shim.snapshot();
                    ns.insert(k.as_string(), v.clone());
                    Ok(CfmlValue::strukt(ns))
                } else {
                    Ok(object.clone())
                }
            } else {
                Ok(CfmlValue::Null)
            }
        }
        "isempty" => {
            if let CfmlValue::Struct(ref shim) = object {
                Ok(CfmlValue::Bool(
                    shim.iter().all(|(k, _)| k.starts_with("__")),
                ))
            } else {
                Ok(CfmlValue::Bool(true))
            }
        }
        _ => Ok(CfmlValue::Null),
    }
}

pub fn handle_java_concurrentlinkedqueue(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => {
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.util.concurrent.concurrentlinkedqueue".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert("__queue".to_string(), CfmlValue::array(Vec::new()));
            Ok(CfmlValue::strukt(shim))
        }
        "add" | "offer" => {
            // Append IN PLACE through the shared shim (real Java queues are
            // reference types: `variables.q.add(x)` and a `q` passed into a
            // function must both see it). Java's add/offer return boolean true.
            //
            // The push MUST mutate the existing backing array under its own write
            // lock rather than snapshot-copy-then-replace: a ConcurrentLinkedQueue
            // is expected to be thread-safe, and the old read-copy-replace lost
            // concurrent adds (10 threads × 5 items surfaced ~48/50 — GH #234).
            // A single with_write per add serialises appends on the shared Arc, so
            // no update is lost.
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(item) = args.first() {
                    match shim.get("__queue") {
                        Some(CfmlValue::Array(a)) => {
                            a.with_write(|v| v.push(item.clone()));
                        }
                        _ => {
                            shim.insert(
                                "__queue".to_string(),
                                CfmlValue::array(vec![item.clone()]),
                            );
                        }
                    }
                }
                Ok(CfmlValue::Bool(true))
            } else {
                Ok(CfmlValue::Null)
            }
        }
        "poll" | "remove" => {
            // Remove and RETURN the head element, mutating the queue in place.
            // (The old impl returned the queue struct and discarded the head.)
            if let CfmlValue::Struct(ref shim) = object {
                // Pop the head in place on the shared backing array (same
                // reasoning as add/offer — keep the Arc stable and the mutation
                // atomic under one write lock).
                if let Some(CfmlValue::Array(a)) = shim.get("__queue") {
                    return Ok(a.with_write(|v| {
                        if v.is_empty() {
                            CfmlValue::Null
                        } else {
                            v.remove(0)
                        }
                    }));
                }
                Ok(CfmlValue::Null)
            } else {
                Ok(CfmlValue::Null)
            }
        }
        "iterator" => {
            // A weakly-consistent snapshot iterator (good enough for the queue's
            // documented use). Returns a java.util.iterator shim with hasNext/next.
            if let CfmlValue::Struct(ref shim) = object {
                let items = match shim.get("__queue") {
                    Some(CfmlValue::Array(a)) => a.snapshot(),
                    _ => Vec::new(),
                };
                let mut it = ValueMap::default();
                it.insert(
                    "__java_class".to_string(),
                    CfmlValue::string("java.util.iterator".to_string()),
                );
                it.insert("__java_shim".to_string(), CfmlValue::Bool(true));
                // Use the engine's existing iterator convention (__iter_items /
                // __iter_pos) so hasNext()/next() — handled in cfml-vm — work.
                it.insert("__iter_items".to_string(), CfmlValue::array(items));
                it.insert("__iter_pos".to_string(), CfmlValue::Int(0));
                Ok(CfmlValue::strukt(it))
            } else {
                Ok(CfmlValue::Null)
            }
        }
        "peek" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::Array(q)) = shim.get("__queue") {
                    if let Some(first) = q.first() {
                        return Ok(first);
                    }
                }
                Ok(CfmlValue::Null)
            } else {
                Ok(CfmlValue::Null)
            }
        }
        "size" | "len" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::Array(q)) = shim.get("__queue") {
                    Ok(CfmlValue::Int(q.len() as i64))
                } else {
                    Ok(CfmlValue::Int(0))
                }
            } else {
                Ok(CfmlValue::Int(0))
            }
        }
        "isempty" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::Array(q)) = shim.get("__queue") {
                    return Ok(CfmlValue::Bool(q.is_empty()));
                }
                Ok(CfmlValue::Bool(true))
            } else {
                Ok(CfmlValue::Bool(true))
            }
        }
        "clear" => {
            if let CfmlValue::Struct(ref shim) = object {
                shim.insert("__queue".to_string(), CfmlValue::array(Vec::new()));
            }
            Ok(CfmlValue::Null)
        }
        _ => Ok(CfmlValue::Null),
    }
}

// ---- SoftReference / ReferenceQueue ----
// These are JVM garbage-collection primitives: a SoftReference holds its
// referent until the GC decides to clear it under memory pressure, enqueuing
// the cleared reference onto a ReferenceQueue. RustCFML has no JVM and no such
// GC pass, so we shim them as STRONG references that are never cleared:
// SoftReference.get() always returns the held referent and the ReferenceQueue
// stays empty (poll() == null). The practical effect is that CacheBox's default
// `ConcurrentSoftReferenceStore` constructs and runs unmodified, but loses only
// memory-pressure eviction — entries still expire via the reap policy /
// maxObjects, exactly like the non-soft `ConcurrentStore`. See issue #218.
pub fn handle_java_softreference(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    handle_java_reference("java.lang.ref.softreference", method, args, object)
}

// A WeakReference is the same kind of reference holder as a SoftReference,
// differing only in when the JVM's GC clears it. With no JVM/GC we shim both as
// strong references that are never cleared on our own, so they share one body.
// Preside's `_createWeakReference()` helpers (WebflowSpecLibrary /
// DatamanagerWorkflowSpecLibrary) construct → init(referent) → store → later
// get() the referent back.
pub fn handle_java_weakreference(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    handle_java_reference("java.lang.ref.weakreference", method, args, object)
}

fn handle_java_reference(
    class: &str,
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => {
            // new SoftReference(referent[, queue]) — hold the referent strongly.
            // The optional ReferenceQueue arg is accepted and ignored (nothing is
            // ever enqueued without a GC clearing the reference).
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string(class.to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert(
                "__referent".to_string(),
                args.first().cloned().unwrap_or(CfmlValue::Null),
            );
            Ok(CfmlValue::strukt(shim))
        }
        "get" => {
            // The strongly-held referent (a soft ref is never cleared here).
            if let CfmlValue::Struct(ref shim) = object {
                return Ok(shim.get("__referent").unwrap_or(CfmlValue::Null));
            }
            Ok(CfmlValue::Null)
        }
        "clear" => {
            // Java's clear() drops the referent; mirror that so a subsequent
            // get() returns null.
            if let CfmlValue::Struct(ref shim) = object {
                shim.insert("__referent".to_string(), CfmlValue::Null);
            }
            Ok(CfmlValue::Null)
        }
        "isenqueued" => Ok(CfmlValue::Bool(false)),
        "enqueue" => Ok(CfmlValue::Bool(false)),
        "hashcode" => {
            // A stable, per-reference identity hash. CacheBox keys its
            // soft-ref-key map on "hc-#softRef.hashCode()#", so distinct
            // SoftReference instances must hash distinctly and stably. The shim
            // struct's Arc backing pointer is exactly that identity.
            if let CfmlValue::Struct(ref shim) = object {
                let raw = shim.backing_ptr() as u64;
                let folded = (raw ^ (raw >> 32)) & 0x7fff_ffff;
                return Ok(CfmlValue::Int(folded as i64));
            }
            Ok(CfmlValue::Int(0))
        }
        _ => Ok(CfmlValue::Null),
    }
}

pub fn handle_java_referencequeue(
    method: &str,
    _args: Vec<CfmlValue>,
    _object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => {
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.lang.ref.referencequeue".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            Ok(CfmlValue::strukt(shim))
        }
        // Nothing is ever enqueued (no GC clears the soft refs), so the queue is
        // permanently empty: poll() returns null, remove(timeout) returns null.
        "poll" | "remove" => Ok(CfmlValue::Null),
        _ => Ok(CfmlValue::Null),
    }
}

// ---- .properties-file reader chain: FileInputStream / InputStreamReader /
//      PropertyResourceBundle / Enumeration ----
// Preside's i18n ResourceBundleService._propertiesFileToStruct() reads a Java
// `.properties` resource bundle with the classic three-class dance:
//   fis = FileInputStream(path)
//   fir = InputStreamReader(fis, "UTF-8")
//   prb = PropertyResourceBundle(fir)
//   keys = prb.getKeys(); while (keys.hasMoreElements()) prb.handleGetObject(keys.nextElement())
// There is no JVM, but this is pure data: read the file off disk and parse the
// `.properties` text into a struct. FileInputStream just carries the path,
// InputStreamReader carries path+charset, and PropertyResourceBundle does the
// actual read+parse and exposes the key enumeration.

pub fn handle_java_fileinputstream(
    method: &str,
    args: Vec<CfmlValue>,
    _object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => {
            // new FileInputStream(path) — path may be a String or a java.io.File
            // shim (which stores its path under __file_path).
            let path = match args.first() {
                Some(CfmlValue::Struct(s)) => s
                    .get("__file_path")
                    .map(|v| v.as_string())
                    .unwrap_or_default(),
                Some(v) => v.as_string(),
                None => String::new(),
            };
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.io.fileinputstream".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert("__stream_path".to_string(), CfmlValue::string(path));
            Ok(CfmlValue::strukt(shim))
        }
        // close() is a no-op (no underlying OS handle is held open). Return an
        // empty string rather than Null so it isn't treated as "unhandled" and
        // re-dispatched against the caller.
        "close" => Ok(CfmlValue::string(String::new())),
        _ => Ok(CfmlValue::Null),
    }
}

pub fn handle_java_inputstreamreader(
    method: &str,
    args: Vec<CfmlValue>,
    _object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => {
            // new InputStreamReader(fis[, charset]) — carry the path forward off
            // the FileInputStream shim; record the charset for completeness.
            let path = match args.first() {
                Some(CfmlValue::Struct(s)) => s
                    .get("__stream_path")
                    .map(|v| v.as_string())
                    .unwrap_or_default(),
                _ => String::new(),
            };
            let charset = args
                .get(1)
                .map(|v| v.as_string())
                .unwrap_or_else(|| "UTF-8".to_string());
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.io.inputstreamreader".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert("__stream_path".to_string(), CfmlValue::string(path));
            shim.insert("__charset".to_string(), CfmlValue::string(charset));
            Ok(CfmlValue::strukt(shim))
        }
        "close" => Ok(CfmlValue::string(String::new())),
        _ => Ok(CfmlValue::Null),
    }
}

pub fn handle_java_propertyresourcebundle(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => {
            // new PropertyResourceBundle(reader) — read the file the reader points
            // at and parse the `.properties` text into an ordered struct. The
            // bare construction (no reader yet, before the chained .init(reader)
            // member call) just returns an empty shim — don't try to read.
            let path = match args.first() {
                Some(CfmlValue::Struct(s)) => s
                    .get("__stream_path")
                    .map(|v| v.as_string())
                    .unwrap_or_default(),
                _ => String::new(),
            };
            if path.is_empty() {
                let mut shim = ValueMap::default();
                shim.insert(
                    "__java_class".to_string(),
                    CfmlValue::string("java.util.propertyresourcebundle".to_string()),
                );
                shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
                shim.insert("__prb_data".to_string(), CfmlValue::strukt(ValueMap::default()));
                return Ok(CfmlValue::strukt(shim));
            }
            let content = std::fs::read_to_string(&path).map_err(|e| {
                CfmlError::runtime(format!(
                    "PropertyResourceBundle: cannot read properties file [{}]: {}",
                    path, e
                ))
            })?;
            let mut data = ValueMap::default();
            for (k, v) in parse_properties(&content) {
                data.insert(k, CfmlValue::string(v));
            }
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.util.propertyresourcebundle".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert("__prb_data".to_string(), CfmlValue::strukt(data));
            Ok(CfmlValue::strukt(shim))
        }
        "getkeys" | "keyset" => {
            // getKeys() returns an Enumeration; keySet() returns a Set. Both feed
            // a hasMoreElements()/nextElement() (or iterator) loop here, so we
            // return an Enumeration shim over the parsed key order.
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::Struct(data)) = shim.get("__prb_data") {
                    let keys: Vec<CfmlValue> =
                        data.keys().into_iter().map(CfmlValue::string).collect();
                    return Ok(make_enumeration(keys));
                }
            }
            Ok(make_enumeration(Vec::new()))
        }
        "handlegetobject" | "getobject" | "getstring" => {
            // Look the key up in the parsed data.
            if let (CfmlValue::Struct(ref shim), Some(key)) = (object, args.first()) {
                if let Some(CfmlValue::Struct(data)) = shim.get("__prb_data") {
                    return Ok(data.get(&key.as_string()).unwrap_or(CfmlValue::Null));
                }
            }
            Ok(CfmlValue::Null)
        }
        "containskey" => {
            if let (CfmlValue::Struct(ref shim), Some(key)) = (object, args.first()) {
                if let Some(CfmlValue::Struct(data)) = shim.get("__prb_data") {
                    return Ok(CfmlValue::Bool(data.get(&key.as_string()).is_some()));
                }
            }
            Ok(CfmlValue::Bool(false))
        }
        _ => Ok(CfmlValue::Null),
    }
}

// A java.util.Enumeration shim (also serves Iterator) over a fixed list of
// values, advancing an in-place cursor. Yielded by
// PropertyResourceBundle.getKeys().
fn make_enumeration(items: Vec<CfmlValue>) -> CfmlValue {
    let mut shim = ValueMap::default();
    shim.insert(
        "__java_class".to_string(),
        CfmlValue::string("java.util.enumeration".to_string()),
    );
    shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    shim.insert("__enum_items".to_string(), CfmlValue::array(items));
    shim.insert("__enum_pos".to_string(), CfmlValue::Int(0));
    CfmlValue::strukt(shim)
}

pub fn handle_java_enumeration(
    method: &str,
    _args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    if let CfmlValue::Struct(ref s) = object {
        let pos = s.get("__enum_pos").map(|v| java_int_arg(&v)).unwrap_or(0);
        let len = match s.get("__enum_items") {
            Some(CfmlValue::Array(ref a)) => a.len() as i64,
            _ => 0,
        };
        match method {
            "hasmoreelements" | "hasnext" => {
                return Ok(CfmlValue::Bool(pos < len));
            }
            "nextelement" | "next" => {
                if pos < len {
                    // Advance the cursor in place (the shim struct is Arc-backed).
                    s.insert("__enum_pos".to_string(), CfmlValue::Int(pos + 1));
                    if let Some(CfmlValue::Array(ref a)) = s.get("__enum_items") {
                        return Ok(a.get(pos as usize).unwrap_or(CfmlValue::Null));
                    }
                }
                return Ok(CfmlValue::Null);
            }
            _ => {}
        }
    }
    Ok(CfmlValue::Null)
}

// Minimal but faithful java.util.Properties / .properties text parser: one
// logical entry per line, `#` or `!` comment lines, `=`/`:`/whitespace as the
// first key/value separator, trailing-backslash line continuation, and the
// standard `\t \n \r \f \uXXXX \\` (and escaped separator) escapes.
fn parse_properties(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let raw_lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    while i < raw_lines.len() {
        // Strip leading whitespace; skip blank and comment lines.
        let mut logical = raw_lines[i].trim_start().to_string();
        i += 1;
        if logical.is_empty() || logical.starts_with('#') || logical.starts_with('!') {
            continue;
        }
        // Join continuation lines (line ends with an odd number of backslashes).
        while ends_with_odd_backslash(&logical) {
            logical.pop(); // drop the trailing backslash
            if i < raw_lines.len() {
                logical.push_str(raw_lines[i].trim_start());
                i += 1;
            } else {
                break;
            }
        }
        // Find the first unescaped separator (= or :), else first unescaped
        // whitespace, to split key from value.
        let chars: Vec<char> = logical.chars().collect();
        let mut sep = None;
        let mut j = 0;
        while j < chars.len() {
            let c = chars[j];
            if c == '\\' {
                j += 2; // skip escaped char
                continue;
            }
            if c == '=' || c == ':' {
                sep = Some(j);
                break;
            }
            if (c == ' ' || c == '\t' || c == '\u{c}') && sep.is_none() {
                // Whitespace separator only if no '='/':' found later; remember
                // it but keep scanning in case an explicit separator follows.
                let rest: String = chars[j..].iter().collect();
                if rest.trim_start().starts_with(['=', ':']) {
                    // explicit separator follows the whitespace — let it win
                    let mut k = j;
                    while k < chars.len()
                        && (chars[k] == ' ' || chars[k] == '\t' || chars[k] == '\u{c}')
                    {
                        k += 1;
                    }
                    sep = Some(k);
                } else {
                    sep = Some(j);
                }
                break;
            }
            j += 1;
        }
        let (key_part, val_part) = match sep {
            Some(s) => {
                let key: String = chars[..s].iter().collect();
                // value starts after the separator char, skipping surrounding ws
                let mut vs = s;
                if chars.get(vs).is_some_and(|c| *c == '=' || *c == ':') {
                    vs += 1;
                }
                while vs < chars.len()
                    && (chars[vs] == ' ' || chars[vs] == '\t' || chars[vs] == '\u{c}')
                {
                    vs += 1;
                }
                let val: String = chars[vs..].iter().collect();
                (key.trim_end().to_string(), val)
            }
            None => (logical.trim_end().to_string(), String::new()),
        };
        out.push((unescape_properties(&key_part), unescape_properties(&val_part)));
    }
    out
}

fn ends_with_odd_backslash(s: &str) -> bool {
    let mut count = 0;
    for c in s.chars().rev() {
        if c == '\\' {
            count += 1;
        } else {
            break;
        }
    }
    count % 2 == 1
}

fn unescape_properties(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('f') => out.push('\u{c}'),
            Some('u') => {
                let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                    if let Some(ch) = char::from_u32(cp) {
                        out.push(ch);
                    }
                }
            }
            Some(other) => out.push(other), // \\, \=, \:, \<space>, etc.
            None => {}
        }
    }
    out
}

// ---- ConcurrentHashMap ----
// Preside/ColdBox Cachebox uses ConcurrentHashMap as a thread-safe cache
// pool: init, put, get, remove (returns old value), containsKey, size,
// keys() (fed into Collections.list), clear, isEmpty.
pub fn handle_java_concurrenthashmap(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => {
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.util.concurrent.concurrenthashmap".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            Ok(CfmlValue::strukt(shim))
        }
        "put" | "putifabsent" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some((k, v)) = args.get(0).zip(args.get(1)) {
                    let key = k.as_string();
                    // putIfAbsent is a no-op if key present
                    if method == "putifabsent" && shim.contains_key(&key) {
                        return Ok(object.clone());
                    }
                    // Mutate the shared backing IN PLACE (interior mutability) and
                    // return the SAME handle. Real Java maps are reference types:
                    // `outer.get(k).put(...)` must mutate the nested map still held
                    // inside `outer`. Snapshotting into a fresh struct broke that —
                    // the put landed only in a throwaway copy (the value returned by
                    // `get`), so nested-map writes (e.g. Wheels Channel subscribers)
                    // never persisted. Returning `object.clone()` shares the Arc, so
                    // the VM's mutating-method write-back is a harmless re-assign.
                    shim.insert(key, v.clone());
                }
                return Ok(object.clone());
            }
            Ok(CfmlValue::Null)
        }
        "get" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(k) = args.first() {
                    return Ok(shim.get(&k.as_string()).unwrap_or(CfmlValue::Null));
                }
            }
            Ok(CfmlValue::Null)
        }
        "containskey" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(k) = args.first() {
                    return Ok(CfmlValue::Bool(shim.contains_key(&k.as_string())));
                }
            }
            Ok(CfmlValue::Bool(false))
        }
        "size" | "len" => {
            if let CfmlValue::Struct(ref shim) = object {
                return Ok(CfmlValue::Int(
                    shim.iter().filter(|(k, _)| !k.starts_with("__")).count() as i64,
                ));
            }
            Ok(CfmlValue::Int(0))
        }
        "isempty" => {
            if let CfmlValue::Struct(ref shim) = object {
                return Ok(CfmlValue::Bool(
                    shim.iter().all(|(k, _)| k.starts_with("__")),
                ));
            }
            Ok(CfmlValue::Bool(true))
        }
        "keys" | "keyset" | "values" => {
            // keys() returns an Enumeration in real Java; keySet() returns a
            // Set. Callers typically either iterate or feed into
            // Collections.list(). Returning a CFML Array satisfies both —
            // arrayLen, indexing, and Collections.list() all work on it.
            if let CfmlValue::Struct(ref shim) = object {
                let values = method == "values";
                let items: Vec<CfmlValue> = shim
                    .iter()
                    .filter(|(k, _)| !k.starts_with("__"))
                    .map(|(k, v)| {
                        if values {
                            v.clone()
                        } else {
                            CfmlValue::string(k.clone())
                        }
                    })
                    .collect();
                return Ok(CfmlValue::array(items));
            }
            Ok(CfmlValue::array(Vec::new()))
        }
        "entryset" => {
            // entrySet() returns a Set<Map.Entry>. Callers do
            // `map.entrySet().toArray()` then iterate calling entry.getKey()/
            // getValue() (e.g. Wheels Channel.publish). Return a CFML Array of
            // Map.Entry shims; .toArray() on a CFML array is the existing no-op.
            if let CfmlValue::Struct(ref shim) = object {
                let entries: Vec<CfmlValue> = shim
                    .iter()
                    .filter(|(k, _)| !k.starts_with("__"))
                    .map(|(k, v)| {
                        let mut e = ValueMap::default();
                        e.insert(
                            "__java_class".to_string(),
                            CfmlValue::string("java.util.map.entry".to_string()),
                        );
                        e.insert("__java_shim".to_string(), CfmlValue::Bool(true));
                        e.insert("__entry_key".to_string(), CfmlValue::string(k.clone()));
                        e.insert("__entry_value".to_string(), v.clone());
                        CfmlValue::strukt(e)
                    })
                    .collect();
                return Ok(CfmlValue::array(entries));
            }
            Ok(CfmlValue::array(Vec::new()))
        }
        "clear" => {
            if let CfmlValue::Struct(ref shim) = object {
                // Remove the data keys in place (preserve the `__java_*` markers)
                // so all aliases of the shared map observe the clear.
                for k in shim.keys() {
                    if !k.starts_with("__") {
                        shim.remove(&k);
                    }
                }
                return Ok(object.clone());
            }
            Ok(CfmlValue::Null)
        }
        // remove is handled in the VM dispatch (needs return-and-mutate
        // semantics identical to Queue.poll); this arm is a no-op safety net.
        _ => Ok(CfmlValue::Null),
    }
}

// ---- java.lang.Class (returned by value.getClass()) ----
// A minimal Class reflection shim. The carried `__class_name` is the runtime
// class name picked at getClass() time; getName()/getSimpleName() let TestBox's
// instanceOf matcher and Wheels' toXML read a type string off a non-component
// value (boolean/string/array/struct/...).
/// Build a `java.lang.Class` shim carrying the given class name. Its
/// getName()/getSimpleName()/forName() members come from `handle_java_class`.
pub fn make_class_shim(class_name: &str) -> CfmlValue {
    let mut shim = ValueMap::default();
    shim.insert(
        "__java_class".to_string(),
        CfmlValue::string("java.lang.class".to_string()),
    );
    shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    shim.insert("__class_name".to_string(), CfmlValue::string(class_name.to_string()));
    CfmlValue::strukt(shim)
}

pub fn handle_java_class(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    let class_name = if let CfmlValue::Struct(ref shim) = object {
        shim.get("__class_name").map(|v| v.as_string()).unwrap_or_default()
    } else {
        String::new()
    };
    match method {
        // createObject("java","java.lang.Class") — the receiver represents the
        // Class class itself; the actual class is supplied later via forName().
        "init" => Ok(make_class_shim("java.lang.Class")),
        // Class.forName("a.b.C") — static factory returning a Class shim for the
        // named class. Used by cbjavaloader's JavaLoader to build URL[] arrays.
        "forname" => {
            let name = args.first().map(|a| a.as_string()).unwrap_or_default();
            Ok(make_class_shim(&name))
        }
        "getname" | "getcanonicalname" | "gettypename" => Ok(CfmlValue::string(class_name)),
        "getsimplename" => {
            let simple = class_name.rsplit('.').next().unwrap_or(&class_name).to_string();
            Ok(CfmlValue::string(simple))
        }
        "tostring" => Ok(CfmlValue::string(format!("class {}", class_name))),
        _ => Ok(CfmlValue::Null),
    }
}

/// Build a "deferred Java object" shim: a stand-in for a class/instance that
/// RustCFML cannot truly provide because there is no JVM (java.net.URLClassLoader,
/// coldfusion.runtime.java.JavaProxy, java.lang.ClassLoader, and the classes
/// they "load"). It no-ops the classloader-plumbing calls cbjavaloader makes
/// during boot and throws loudly the moment a genuinely-loaded class is invoked.
/// See `handle_java_classloader`.
pub fn make_deferred_java(class_name: &str) -> CfmlValue {
    let mut shim = ValueMap::default();
    shim.insert(
        "__java_class".to_string(),
        CfmlValue::string("java.lang.__deferred".to_string()),
    );
    shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    shim.insert("__class_name".to_string(), CfmlValue::string(class_name.to_string()));
    CfmlValue::strukt(shim)
}

/// Coerce a CfmlValue argument to an i64 the way the other Java shims do.
fn java_int_arg(v: &CfmlValue) -> i64 {
    match v {
        CfmlValue::Int(n) => *n,
        CfmlValue::Double(d) => *d as i64,
        other => other.as_string().trim().parse::<i64>().unwrap_or(0),
    }
}

/// java.lang.reflect.Array — the static helper cbjavaloader uses to build the
/// `URL[]` it hands to URLClassLoader. We model a Java array as a plain CFML
/// (1-based) array; newInstance(class, n) makes one of `n` nulls and set/get
/// index it 0-based, exactly as the Java API does.
pub fn handle_java_reflect_array(
    method: &str,
    args: Vec<CfmlValue>,
    _object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => {
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.lang.reflect.array".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            Ok(CfmlValue::strukt(shim))
        }
        // Array.newInstance(componentType, length) -> a new array of `length`
        // nulls. The component type is irrelevant for our untyped CFML array.
        "newinstance" => {
            let len = args.get(1).map(java_int_arg).unwrap_or(0).max(0) as usize;
            Ok(CfmlValue::array(vec![CfmlValue::Null; len]))
        }
        // Array.set(array, index, value) — 0-based; mutate a copy and write it
        // back is not possible here (static call, no receiver writeback), so the
        // caller must hold the array reference. CfmlArray is Arc-shared, so
        // mutating in place propagates to the holding variable.
        "set" => {
            if let (Some(CfmlValue::Array(arr)), Some(idx), Some(val)) =
                (args.first(), args.get(1), args.get(2))
            {
                let i = java_int_arg(idx);
                if i >= 0 {
                    arr.with_write(|v| {
                        let i = i as usize;
                        if i < v.len() {
                            v[i] = val.clone();
                        } else {
                            while v.len() < i {
                                v.push(CfmlValue::Null);
                            }
                            v.push(val.clone());
                        }
                    });
                }
            }
            Ok(CfmlValue::Null)
        }
        "get" => {
            if let (Some(CfmlValue::Array(arr)), Some(idx)) = (args.first(), args.get(1)) {
                let i = java_int_arg(idx);
                if i >= 0 {
                    return Ok(arr.get(i as usize).unwrap_or(CfmlValue::Null));
                }
            }
            Ok(CfmlValue::Null)
        }
        "getlength" => {
            if let Some(CfmlValue::Array(arr)) = args.first() {
                return Ok(CfmlValue::Int(arr.len() as i64));
            }
            Ok(CfmlValue::Int(0))
        }
        _ => Ok(CfmlValue::Null),
    }
}

/// The "deferred Java object" dispatcher (see `make_deferred_java`). Covers
/// java.net.URLClassLoader, coldfusion.runtime.java.JavaProxy,
/// java.lang.ClassLoader and the classes they pretend to load. cbjavaloader's
/// `JavaLoader.cfc` builds a URLClassLoader / NetworkClassLoader tower at module
/// boot; none of the classes it loads are actually used during Preside boot
/// (only at runtime, e.g. GoogleAuthenticator 2FA), so we let the *plumbing*
/// calls succeed and throw clearly only when a genuinely-loaded class is used.
pub fn handle_java_classloader(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    let class_name = if let CfmlValue::Struct(ref s) = object {
        s.get("__class_name").map(|v| v.as_string()).unwrap_or_default()
    } else {
        String::new()
    };
    match method {
        // JavaProxy.init(class) adopts the wrapped class so a later loadClass /
        // create reports the right name. URLClassLoader.init(urls) and the
        // proxy's no-arg .init() just return the (deferred) receiver.
        "init" => match args.first() {
            Some(CfmlValue::Struct(s))
                if s.get("__java_class").map(|v| v.as_string()).as_deref()
                    == Some("java.lang.class") =>
            {
                let adopted = s.get("__class_name").map(|v| v.as_string()).unwrap_or_default();
                Ok(make_deferred_java(&adopted))
            }
            Some(CfmlValue::String(name)) => Ok(make_deferred_java(name)),
            _ => Ok(object.clone()),
        },
        // loadClass / forName hand back a Class shim (deferred) — instantiation
        // is what ultimately throws, not the class lookup.
        "loadclass" | "forname" => {
            let name = args.first().map(|a| a.as_string()).unwrap_or_default();
            Ok(make_class_shim(&name))
        }
        "getsystemclassloader" | "getcontextclassloader" | "getparent" | "getclassloader" => {
            Ok(object.clone())
        }
        // void plumbing methods — return a non-null so the dispatcher doesn't
        // treat the result as "method unhandled" and fall through.
        "setcontextclassloader" | "addurl" => Ok(CfmlValue::Bool(true)),
        "geturls" => Ok(CfmlValue::array(vec![])),
        "getname" | "getcanonicalname" | "gettypename" => Ok(CfmlValue::string(class_name)),
        "tostring" => Ok(CfmlValue::string(class_name)),
        _ => Err(CfmlError::runtime(format!(
            "Java class [{}] cannot be used: RustCFML has no JVM, so classes loaded \
             dynamically via cbjavaloader / java.net.URLClassLoader are unavailable \
             (attempted method [{}]).",
            if class_name.is_empty() { "java.net.URLClassLoader" } else { &class_name },
            method
        ))),
    }
}

/// java.lang.Runtime — JVM runtime singleton. ColdBox's CacheBoxProvider builds
/// it eagerly at init for an optional free-memory threshold check (which Preside
/// leaves disabled), so it must construct without error. We back the handful of
/// methods with real host values where we can; gc() is a no-op.
pub fn handle_java_runtime(method: &str, _args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    match method {
        "init" => {
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.lang.runtime".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            Ok(CfmlValue::strukt(shim))
        }
        // Runtime.getRuntime() — static accessor returning the singleton; the
        // receiver already IS that singleton shim.
        "getruntime" => Ok(object.clone()),
        "availableprocessors" => Ok(CfmlValue::Int(
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as i64,
        )),
        // We have no JVM heap; report plausible non-zero byte counts so callers
        // computing free/max ratios don't divide by zero. (256 MiB free of 4 GiB.)
        "freememory" => Ok(CfmlValue::Double(268_435_456.0)),
        "totalmemory" => Ok(CfmlValue::Double(536_870_912.0)),
        "maxmemory" => Ok(CfmlValue::Double(4_294_967_296.0)),
        "gc" | "runfinalization" => Ok(CfmlValue::Null),
        _ => Ok(CfmlValue::Null),
    }
}

/// java.util.Iterator — produced by `array.iterator()`. Holds a snapshot of the
/// array plus a cursor. `hasNext()` is pure (handled here); `next()` advances
/// the cursor and so is handled at the call site with a receiver write-back
/// (mirroring the Matcher.find pattern in lib.rs).
pub fn handle_java_iterator(
    method: &str,
    _args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    if method == "hasnext" {
        if let CfmlValue::Struct(ref s) = object {
            let pos = s.get("__iter_pos").map(|v| java_int_arg(&v)).unwrap_or(0);
            let len = match s.get("__iter_items") {
                Some(CfmlValue::Array(a)) => a.len() as i64,
                _ => 0,
            };
            return Ok(CfmlValue::Bool(pos < len));
        }
        return Ok(CfmlValue::Bool(false));
    }
    Ok(CfmlValue::Null)
}

// ---- java.util.Map.Entry (yielded by ConcurrentHashMap.entrySet()) ----
pub fn handle_java_map_entry(method: &str, _args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    if let CfmlValue::Struct(ref e) = object {
        match method {
            "getkey" => return Ok(e.get("__entry_key").unwrap_or(CfmlValue::Null)),
            "getvalue" => return Ok(e.get("__entry_value").unwrap_or(CfmlValue::Null)),
            "tostring" => {
                let k = e.get("__entry_key").map(|v| v.as_string()).unwrap_or_default();
                let v = e.get("__entry_value").map(|v| v.as_string()).unwrap_or_default();
                return Ok(CfmlValue::string(format!("{}={}", k, v)));
            }
            _ => {}
        }
    }
    Ok(CfmlValue::Null)
}

// ---- Collections (static utility class) ----
// Preside/ColdBox use-case: Collections.list(map.keys()) converts a legacy
// Enumeration into an ArrayList. Since our ConcurrentHashMap.keys() already
// returns a CFML Array, Collections.list(array) is identity. We also handle
// a handful of other common static helpers so real code runs unchanged.
pub fn handle_java_collections(
    method: &str,
    args: Vec<CfmlValue>,
    _object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => {
            // Collections is static-only; return a stub shim so static calls
            // dispatch through to this handler.
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.util.collections".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            Ok(CfmlValue::strukt(shim))
        }
        "list" => {
            // Collections.list(Enumeration) → ArrayList. Our callers hand in
            // a CFML Array already, so this is an identity operation.
            match args.into_iter().next() {
                Some(CfmlValue::Array(a)) => Ok(CfmlValue::Array(a)),
                Some(other) => Ok(other),
                None => Ok(CfmlValue::array(Vec::new())),
            }
        }
        "emptylist" | "emptyset" => Ok(CfmlValue::array(Vec::new())),
        "emptymap" => Ok(CfmlValue::strukt(ValueMap::default())),
        "unmodifiablelist" | "unmodifiableset" | "synchronizedlist" | "synchronizedset" => {
            // No true immutability in CFML; behave as identity like Lucee.
            match args.into_iter().next() {
                Some(v) => Ok(v),
                None => Ok(CfmlValue::array(Vec::new())),
            }
        }
        "unmodifiablemap" | "synchronizedmap" => match args.into_iter().next() {
            Some(v) => Ok(v),
            None => Ok(CfmlValue::strukt(ValueMap::default())),
        },
        "sort" => {
            if let Some(CfmlValue::Array(a)) = args.into_iter().next() {
                // Collections.sort mutates the list in place (reference semantics).
                a.with_write(|v| v.sort_by(|x, y| x.as_string().cmp(&y.as_string())));
                return Ok(CfmlValue::Array(a));
            }
            Ok(CfmlValue::Null)
        }
        "reverse" => {
            if let Some(CfmlValue::Array(a)) = args.into_iter().next() {
                a.with_write(|v| v.reverse());
                return Ok(CfmlValue::Array(a));
            }
            Ok(CfmlValue::Null)
        }
        _ => Ok(CfmlValue::Null),
    }
}

pub fn handle_java_paths(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    match method {
        "init" => {
            // Paths is a static-only class; return a stub shim so that
            // the subsequent .get(path) static call dispatches here.
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.nio.file.paths".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            Ok(CfmlValue::strukt(shim))
        }
        "get" => {
            let path = args.first().map(|a| a.as_string()).unwrap_or_default();
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.nio.file.paths".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert("__path".to_string(), CfmlValue::string(path));
            Ok(CfmlValue::strukt(shim))
        }
        "getparent" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::String(path)) = shim.get("__path") {
                    if let Some(p) = std::path::Path::new(path.as_str()).parent() {
                        let mut ps = ValueMap::default();
                        ps.insert(
                            "__java_class".to_string(),
                            CfmlValue::string("java.nio.file.paths".to_string()),
                        );
                        ps.insert("__java_shim".to_string(), CfmlValue::Bool(true));
                        ps.insert(
                            "__path".to_string(),
                            CfmlValue::string(p.to_string_lossy().to_string()),
                        );
                        return Ok(CfmlValue::strukt(ps));
                    }
                }
                Ok(CfmlValue::Null)
            } else {
                Ok(CfmlValue::Null)
            }
        }
        "isabsolute" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::String(path)) = shim.get("__path") {
                    return Ok(CfmlValue::Bool(std::path::Path::new(path.as_str()).is_absolute()));
                }
                Ok(CfmlValue::Bool(false))
            } else {
                Ok(CfmlValue::Bool(false))
            }
        }
        "tostring" => {
            if let CfmlValue::Struct(ref shim) = object {
                Ok(shim
                    .get("__path")
                    .unwrap_or(CfmlValue::string(String::new())))
            } else {
                Ok(CfmlValue::string(String::new()))
            }
        }
        "toabsolute" | "toabsolutepath" => {
            if let CfmlValue::Struct(ref shim) = object {
                if let Some(CfmlValue::String(path)) = shim.get("__path") {
                    let p = std::path::Path::new(path.as_str());
                    if p.is_absolute() {
                        return Ok(shim.get("__path").unwrap_or(CfmlValue::Null));
                    }
                    if let Ok(cwd) = std::env::current_dir() {
                        let full = cwd.join(path.as_str());
                        let mut ns = shim.snapshot();
                        ns.insert(
                            "__path".to_string(),
                            CfmlValue::string(full.to_string_lossy().to_string()),
                        );
                        return Ok(CfmlValue::strukt(ns));
                    }
                }
                Ok(CfmlValue::Null)
            } else {
                Ok(CfmlValue::Null)
            }
        }
        _ => Ok(CfmlValue::Null),
    }
}

fn rand_u128() -> u128 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    cfml_common::clock::now_unix_nanos().hash(&mut h);
    0x12345678u64.hash(&mut h);
    h.finish() as u128
}

/// Shim for `java.util.regex.Pattern` and the `Matcher` it produces — used by
/// Lucee apps for dynamic route matching. Backed by Rust's `regex` crate, whose
/// syntax is a close superset for the patterns these apps use.
///
/// Flow: `createObject("java","java.util.regex.Pattern")` → `init`; then
/// `.compile(regex)` → a compiled Pattern shim; `.matcher(input)` → a Matcher
/// shim. `find()`/`matches()`/`lookingAt()` advance the matcher's cursor and
/// stash the capture groups; they are handled inline in the VM (see
/// `java_matcher_step`) because they mutate matcher state that must be written
/// back to the variable. `group(n)`/`groupCount()` read that stashed state and
/// stay here (pure reads).
pub fn handle_java_pattern(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    use regex::Regex;

    let object_regex = || -> String {
        if let CfmlValue::Struct(s) = object {
            s.get("__regex").map(|v| v.as_string()).unwrap_or_default()
        } else {
            String::new()
        }
    };
    let compile = |pattern: &str| -> Result<Regex, CfmlError> {
        Regex::new(pattern).map_err(|e| {
            CfmlError::runtime(format!(
                "java.util.regex.Pattern: invalid pattern [{}]: {}",
                pattern, e
            ))
        })
    };

    match method {
        // createObject(...) with no pattern yet — an uncompiled Pattern handle.
        "init" => {
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.util.regex.pattern".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            Ok(CfmlValue::strukt(shim))
        }
        // Pattern.compile(regex[, flags]) — returns a compiled Pattern shim.
        "compile" => {
            let regex_str = args.first().map(|a| a.as_string()).unwrap_or_default();
            compile(&regex_str)?; // validate up front
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.util.regex.pattern".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert("__regex".to_string(), CfmlValue::string(regex_str));
            Ok(CfmlValue::strukt(shim))
        }
        "pattern" | "tostring" => Ok(CfmlValue::string(object_regex())),
        // Pattern.matcher(input) — create a Matcher positioned before the first
        // match. find()/matches()/lookingAt() (handled inline in the VM so they
        // can write the advanced state back) populate the capture groups.
        "matcher" => {
            let regex_str = object_regex();
            let input = args.first().map(|a| a.as_string()).unwrap_or_default();
            let re = compile(&regex_str)?;
            let group_count = re.captures_len() as i64 - 1;
            let mut shim = ValueMap::default();
            shim.insert(
                "__java_class".to_string(),
                CfmlValue::string("java.util.regex.matcher".to_string()),
            );
            shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
            shim.insert("__regex".to_string(), CfmlValue::string(regex_str));
            shim.insert("__input".to_string(), CfmlValue::string(input));
            shim.insert("__groupcount".to_string(), CfmlValue::Int(group_count));
            shim.insert("__matched".to_string(), CfmlValue::Bool(false));
            shim.insert("__findindex".to_string(), CfmlValue::Int(0));
            shim.insert("__groups".to_string(), CfmlValue::array(Vec::new()));
            Ok(CfmlValue::strukt(shim))
        }
        // Matcher.group([n]) — group 0 is the whole match.
        "group" => {
            if let CfmlValue::Struct(s) = object {
                let idx = args
                    .first()
                    .and_then(|a| a.as_string().trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if let Some(CfmlValue::Array(groups)) = s.get("__groups") {
                    return Ok(groups.snapshot().get(idx).cloned().unwrap_or(CfmlValue::Null));
                }
            }
            Ok(CfmlValue::Null)
        }
        "groupcount" => {
            if let CfmlValue::Struct(s) = object {
                return Ok(s.get("__groupcount").unwrap_or(CfmlValue::Int(0)));
            }
            Ok(CfmlValue::Int(0))
        }
        // Matcher.start([n]) / end([n]) — 0-based char offset of the most recent
        // match (or group n). Populated by `java_matcher_step` after find()/etc.
        "start" | "end" => {
            if let CfmlValue::Struct(s) = object {
                let group_arg = args.first().and_then(|a| a.as_string().trim().parse::<usize>().ok());
                let (single_key, group_key) = if method == "start" {
                    ("__start", "__startgroups")
                } else {
                    ("__end", "__endgroups")
                };
                return Ok(match group_arg {
                    None | Some(0) => s.get(single_key).unwrap_or(CfmlValue::Int(-1)),
                    Some(n) => {
                        if let Some(CfmlValue::Array(g)) = s.get(group_key) {
                            g.snapshot().get(n).cloned().unwrap_or(CfmlValue::Int(-1))
                        } else {
                            CfmlValue::Int(-1)
                        }
                    }
                });
            }
            Ok(CfmlValue::Int(-1))
        }
        _ => Ok(CfmlValue::Null),
    }
}

/// Which matcher operation `java_matcher_step` performs.
pub enum MatchMode {
    /// `find()` — next non-overlapping match from the cursor; advances it.
    Find,
    /// `matches()` — the whole input must match; does not move the cursor.
    Matches,
    /// `lookingAt()` — match anchored at the start; does not move the cursor.
    LookingAt,
}

/// Advance a `java.util.regex.Matcher` shim one step. Returns
/// `(matched, updated_matcher)`: the updated struct carries the refreshed
/// `__groups`/`__matched` (and, for `Find`, the incremented `__findindex`) and
/// must be written back to the matcher variable so a subsequent `group(n)`
/// sees this step's captures. `find()` walks non-overlapping matches
/// left-to-right exactly like Java's `Matcher.find()`, so `while (m.find())`
/// terminates.
pub fn java_matcher_step(
    s: &cfml_common::dynamic::CfmlStruct,
    mode: MatchMode,
) -> Result<(bool, CfmlValue), CfmlError> {
    use regex::Regex;
    let regex_str = s.get("__regex").map(|v| v.as_string()).unwrap_or_default();
    let input = s.get("__input").map(|v| v.as_string()).unwrap_or_default();
    let re = Regex::new(&regex_str).map_err(|e| {
        CfmlError::runtime(format!(
            "java.util.regex.Matcher: invalid pattern [{}]: {}",
            regex_str, e
        ))
    })?;

    let find_index = s
        .get("__findindex")
        .and_then(|v| v.as_string().trim().parse::<usize>().ok())
        .unwrap_or(0);

    let caps = match mode {
        MatchMode::Find => re.captures_iter(&input).nth(find_index),
        MatchMode::Matches => re
            .captures(&input)
            .filter(|c| c.get(0).map(|m| m.start() == 0 && m.end() == input.len()).unwrap_or(false)),
        MatchMode::LookingAt => re
            .captures(&input)
            .filter(|c| c.get(0).map(|m| m.start() == 0).unwrap_or(false)),
    };

    let mut ns = s.snapshot();
    let matched = caps.is_some();
    // Convert a byte offset into the (0-based) char offset Java's Matcher uses.
    let char_off = |byte: usize| -> i64 { input[..byte].chars().count() as i64 };
    let groups: Vec<CfmlValue> = match &caps {
        Some(caps) => (0..re.captures_len())
            .map(|i| {
                caps.get(i)
                    .map(|m| CfmlValue::string(m.as_str().to_string()))
                    .unwrap_or(CfmlValue::Null)
            })
            .collect(),
        None => Vec::new(),
    };
    // Stash per-group start/end char offsets so Matcher.start([n])/end([n]) can
    // read them. Group 0 is the whole match; -1 marks an unmatched group (Java
    // semantics). On no-match the arrays are empty and start()/end() return -1.
    let (start_groups, end_groups): (Vec<CfmlValue>, Vec<CfmlValue>) = match &caps {
        Some(caps) => (0..re.captures_len())
            .map(|i| match caps.get(i) {
                Some(m) => (CfmlValue::Int(char_off(m.start())), CfmlValue::Int(char_off(m.end()))),
                None => (CfmlValue::Int(-1), CfmlValue::Int(-1)),
            })
            .unzip(),
        None => (Vec::new(), Vec::new()),
    };
    let group0_start = start_groups.first().cloned().unwrap_or(CfmlValue::Int(-1));
    let group0_end = end_groups.first().cloned().unwrap_or(CfmlValue::Int(-1));
    ns.insert("__matched".to_string(), CfmlValue::Bool(matched));
    ns.insert("__groups".to_string(), CfmlValue::array(groups));
    ns.insert("__start".to_string(), group0_start);
    ns.insert("__end".to_string(), group0_end);
    ns.insert("__startgroups".to_string(), CfmlValue::array(start_groups));
    ns.insert("__endgroups".to_string(), CfmlValue::array(end_groups));
    if matches!(mode, MatchMode::Find) && matched {
        ns.insert("__findindex".to_string(), CfmlValue::Int((find_index + 1) as i64));
    }
    Ok((matched, CfmlValue::strukt(ns)))
}

// ===============================================================
// Servlet bridge: getPageContext().getRequest() / .getResponse()
// ===============================================================
//
// On Lucee and Adobe CF the page context exposes live servlet request/
// response objects in EVERY execution context — even CLI/task contexts,
// where Lucee synthesizes them (getRequestURL() returns
// "http://localhost/index.cfm" with no real HTTP request in sight).
// Wheels builds request URLs through this exact chain
// (`GetPageContext().getRequest().getRequestURL()`), so the bridge must be
// non-null and method-faithful in both serve and CLI mode.
//
// We model Lucee's behaviour (real servlet objects with the full
// HttpServletRequest/Response surface) rather than BoxLang's narrower
// FakePageContext (whose getRequest()/getResponse() return the page context
// itself). For broad compatibility the page-context shim also forwards the
// request-side accessors BoxLang exposes directly (getRequestURL et al.),
// making the surface a superset of both engines.
//
// Request values are synthesized from the request's CGI scope when present
// (serve mode); absent in bare CLI, we fall back to Lucee's task-context
// defaults (localhost:80, /index.cfm, GET, http). The response side is
// dispatched in `lib.rs` (it mutates `self.response_status` /
// `self.response_headers` so setStatus()/setHeader() are faithful in serve
// mode, not no-ops).

pub const SERVLET_PAGE_CONTEXT_CLASS: &str = "lucee.runtime.pagecontextimpl";
pub const SERVLET_REQUEST_CLASS: &str = "lucee.runtime.net.http.httpservletrequestwrap";
pub const SERVLET_RESPONSE_CLASS: &str = "lucee.runtime.net.http.httpservletresponsedummy";

/// Build the `HttpServletRequest` shim returned by getPageContext().getRequest().
/// `cgi` is the request's CGI scope (serve mode) or `None` in bare CLI.
pub fn build_servlet_request_shim(cgi: Option<&ValueMap>) -> CfmlValue {
    let nonempty = |k: &str| {
        cgi.and_then(|c| c.get(k))
            .map(|v| v.as_string())
            .filter(|s| !s.is_empty())
    };
    let server_name = nonempty("server_name").unwrap_or_else(|| "localhost".to_string());
    let port: i64 = nonempty("server_port")
        .and_then(|s| s.parse().ok())
        .unwrap_or(80);
    let method = nonempty("request_method").unwrap_or_else(|| "GET".to_string());
    let script = nonempty("script_name").unwrap_or_else(|| "/index.cfm".to_string());
    let query = cgi
        .and_then(|c| c.get("query_string"))
        .map(|v| v.as_string())
        .unwrap_or_default();
    let remote = nonempty("remote_addr").unwrap_or_else(|| "127.0.0.1".to_string());
    let secure = cgi
        .and_then(|c| c.get("https"))
        .map(|v| v.as_string().eq_ignore_ascii_case("on"))
        .unwrap_or(false);
    let scheme = if secure { "https" } else { "http" };
    // Lucee omits the port from getRequestURL() when it is the scheme default.
    let port_part = if (!secure && port == 80) || (secure && port == 443) {
        String::new()
    } else {
        format!(":{}", port)
    };
    let request_url = format!("{}://{}{}{}", scheme, server_name, port_part, script);
    let content_type = nonempty("content_type");

    let mut s = ValueMap::default();
    s.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    s.insert(
        "__java_class".to_string(),
        CfmlValue::string(SERVLET_REQUEST_CLASS.to_string()),
    );
    s.insert("__req_url".to_string(), CfmlValue::string(request_url));
    s.insert("__req_uri".to_string(), CfmlValue::string(script.clone()));
    s.insert("__req_query".to_string(), CfmlValue::string(query));
    s.insert("__req_method".to_string(), CfmlValue::string(method));
    s.insert("__req_scheme".to_string(), CfmlValue::string(scheme.to_string()));
    s.insert("__req_server_name".to_string(), CfmlValue::string(server_name));
    s.insert("__req_server_port".to_string(), CfmlValue::Int(port));
    s.insert("__req_servlet_path".to_string(), CfmlValue::string(script));
    s.insert("__req_remote_addr".to_string(), CfmlValue::string(remote));
    s.insert("__req_secure".to_string(), CfmlValue::Bool(secure));
    s.insert(
        "__req_content_type".to_string(),
        content_type.map(CfmlValue::string).unwrap_or(CfmlValue::Null),
    );
    // Retain the CGI snapshot so getHeader(name) can resolve http_* keys.
    if let Some(c) = cgi {
        s.insert("__req_cgi".to_string(), CfmlValue::strukt(c.clone()));
    }
    CfmlValue::strukt(s)
}

/// Build the `HttpServletResponse` shim. State lives on the VM
/// (`response_status`/`response_headers`); the shim itself is just a marker
/// dispatched in `lib.rs`.
pub fn build_servlet_response_shim() -> CfmlValue {
    let mut s = ValueMap::default();
    s.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    s.insert(
        "__java_class".to_string(),
        CfmlValue::string(SERVLET_RESPONSE_CLASS.to_string()),
    );
    CfmlValue::strukt(s)
}

/// Build the page-context shim returned by getPageContext().
pub fn build_page_context_shim(cgi: Option<&ValueMap>) -> CfmlValue {
    let mut s = ValueMap::default();
    s.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    s.insert(
        "__java_class".to_string(),
        CfmlValue::string(SERVLET_PAGE_CONTEXT_CLASS.to_string()),
    );
    s.insert("__pc_request".to_string(), build_servlet_request_shim(cgi));
    s.insert("__pc_response".to_string(), build_servlet_response_shim());
    CfmlValue::strukt(s)
}

/// Dispatch a method call on the `HttpServletRequest` shim. Read-only: every
/// value was synthesized at construction time, so this needs no VM access.
pub fn handle_servlet_request(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    let s = match object {
        CfmlValue::Struct(s) => s,
        _ => return Ok(CfmlValue::Null),
    };
    let get = |k: &str| s.get(k).unwrap_or(CfmlValue::Null);
    Ok(match method {
        "getrequesturl" => get("__req_url"),
        "getrequesturi" => get("__req_uri"),
        "getquerystring" => get("__req_query"),
        "getmethod" => get("__req_method"),
        "getscheme" => get("__req_scheme"),
        "getservername" => get("__req_server_name"),
        "getserverport" => get("__req_server_port"),
        "getservletpath" => get("__req_servlet_path"),
        "getremoteaddr" | "getremotehost" => get("__req_remote_addr"),
        "getcontenttype" => get("__req_content_type"),
        "issecure" => get("__req_secure"),
        "getprotocol" => CfmlValue::string("HTTP/1.1".to_string()),
        // Lucee serves apps at the context root, so contextPath is empty and
        // pathInfo is null for a plain script request.
        "getcontextpath" => CfmlValue::string(String::new()),
        "getpathinfo" => CfmlValue::Null,
        "getcharacterencoding" => CfmlValue::string("UTF-8".to_string()),
        "getlocaladdr" => get("__req_remote_addr"),
        "getlocalport" => get("__req_server_port"),
        "getheader" => {
            let name = args.first().map(|v| v.as_string()).unwrap_or_default();
            let key = format!("http_{}", name.to_lowercase().replace('-', "_"));
            match s.get("__req_cgi") {
                Some(CfmlValue::Struct(cgi)) => cgi.get(&key).unwrap_or(CfmlValue::Null),
                _ => CfmlValue::Null,
            }
        }
        // Unknown method: a non-null receiver is enough to keep call chains
        // alive; return null rather than throwing (matches a servlet getter
        // with no value).
        _ => CfmlValue::Null,
    })
}

// ============================================================================
// java.util.Locale / java.util.TimeZone / java.util.GregorianCalendar and the
// java.text.* date/number-formatting classes. ColdBox's cbi18n module
// (`models/i18n.cfc`) is a thin wrapper over these JVM classes; on a real JVM
// Lucee/ACF hand back the genuine objects, but RustCFML has no JVM, so we shim
// them. These are sufficient to construct + configure cbi18n at boot
// (`buildLocale()` calls Locale.getDefault()/init()/getAvailableLocales(), and
// the GregorianCalendar/DateFormatSymbols `.init(buildLocale())` chains must
// return a non-null receiver). Request-time date/number formatting is
// best-effort.
// ============================================================================

/// Build a base java-shim ValueMap flagged with the given (already-lowercase)
/// class name.
fn jshim(class: &str) -> ValueMap {
    let mut shim = ValueMap::default();
    shim.insert(
        "__java_class".to_string(),
        CfmlValue::string(class.to_string()),
    );
    shim.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    shim
}

/// A reasonable subset of the locale ids the JVM ships from
/// `Locale.getAvailableLocales()`. cbi18n's `isValidLocale()` does
/// `listFind( arrayToList( getAvailableLocales() ), "<id>" )`, so the list must
/// contain the exact Java-style ids (e.g. `en_US`) it validates.
const AVAILABLE_LOCALES: &[&str] = &[
    "ar", "ar_AE", "ar_EG", "ar_SA", "bg", "bg_BG", "ca", "ca_ES", "cs", "cs_CZ", "da", "da_DK",
    "de", "de_AT", "de_CH", "de_DE", "el", "el_GR", "en", "en_AU", "en_CA", "en_GB", "en_IE",
    "en_IN", "en_NZ", "en_US", "en_ZA", "es", "es_AR", "es_ES", "es_MX", "et", "et_EE", "fi",
    "fi_FI", "fr", "fr_BE", "fr_CA", "fr_CH", "fr_FR", "he", "he_IL", "hi", "hi_IN", "hr", "hr_HR",
    "hu", "hu_HU", "id", "id_ID", "is", "is_IS", "it", "it_CH", "it_IT", "iw", "iw_IL", "ja",
    "ja_JP", "ko", "ko_KR", "lt", "lt_LT", "lv", "lv_LV", "nl", "nl_BE", "nl_NL", "no", "no_NO",
    "pl", "pl_PL", "pt", "pt_BR", "pt_PT", "ro", "ro_RO", "ru", "ru_RU", "sk", "sk_SK", "sl",
    "sl_SI", "sr", "sr_RS", "sv", "sv_SE", "th", "th_TH", "tr", "tr_TR", "uk", "uk_UA", "vi",
    "vi_VN", "zh", "zh_CN", "zh_HK", "zh_SG", "zh_TW",
];

fn locale_language_name(code: &str) -> &'static str {
    match code {
        "ar" => "Arabic",
        "bg" => "Bulgarian",
        "ca" => "Catalan",
        "cs" => "Czech",
        "da" => "Danish",
        "de" => "German",
        "el" => "Greek",
        "en" => "English",
        "es" => "Spanish",
        "et" => "Estonian",
        "fi" => "Finnish",
        "fr" => "French",
        "he" | "iw" => "Hebrew",
        "hi" => "Hindi",
        "hr" => "Croatian",
        "hu" => "Hungarian",
        "id" => "Indonesian",
        "is" => "Icelandic",
        "it" => "Italian",
        "ja" => "Japanese",
        "ko" => "Korean",
        "lt" => "Lithuanian",
        "lv" => "Latvian",
        "nl" => "Dutch",
        "no" => "Norwegian",
        "pl" => "Polish",
        "pt" => "Portuguese",
        "ro" => "Romanian",
        "ru" => "Russian",
        "sk" => "Slovak",
        "sl" => "Slovenian",
        "sr" => "Serbian",
        "sv" => "Swedish",
        "th" => "Thai",
        "tr" => "Turkish",
        "uk" => "Ukrainian",
        "vi" => "Vietnamese",
        "zh" => "Chinese",
        _ => "",
    }
}

/// ISO 639-2/T 3-letter language codes for the languages we tabulate (matching
/// `Locale.getISO3Language()`). Empty string => not tabulated.
fn locale_iso3_language(code: &str) -> &'static str {
    match code {
        "ar" => "ara",
        "bg" => "bul",
        "ca" => "cat",
        "cs" => "ces",
        "da" => "dan",
        "de" => "deu",
        "el" => "ell",
        "en" => "eng",
        "es" => "spa",
        "et" => "est",
        "fi" => "fin",
        "fr" => "fra",
        "he" => "heb",
        "iw" => "heb",
        "hi" => "hin",
        "hr" => "hrv",
        "hu" => "hun",
        "id" => "ind",
        "is" => "isl",
        "it" => "ita",
        "ja" => "jpn",
        "ko" => "kor",
        "lt" => "lit",
        "lv" => "lav",
        "nl" => "nld",
        "no" => "nor",
        "pl" => "pol",
        "pt" => "por",
        "ro" => "ron",
        "ru" => "rus",
        "sk" => "slk",
        "sl" => "slv",
        "sr" => "srp",
        "sv" => "swe",
        "th" => "tha",
        "tr" => "tur",
        "uk" => "ukr",
        "vi" => "vie",
        "zh" => "zho",
        _ => "",
    }
}

/// ISO 3166-1 alpha-3 country codes for the countries we tabulate (matching
/// `Locale.getISO3Country()`). Empty string => not tabulated.
fn locale_iso3_country(code: &str) -> &'static str {
    match code {
        "AU" => "AUS",
        "BR" => "BRA",
        "CA" => "CAN",
        "CH" => "CHE",
        "CN" => "CHN",
        "DE" => "DEU",
        "ES" => "ESP",
        "FR" => "FRA",
        "GB" => "GBR",
        "IE" => "IRL",
        "IN" => "IND",
        "IT" => "ITA",
        "JP" => "JPN",
        "KR" => "KOR",
        "MX" => "MEX",
        "NL" => "NLD",
        "NZ" => "NZL",
        "PT" => "PRT",
        "RU" => "RUS",
        "TW" => "TWN",
        "US" => "USA",
        "ZA" => "ZAF",
        _ => "",
    }
}

/// The server/JVM default locale. Java reads the `user.language`/`user.country`
/// system properties, which the JVM derives from the OS locale; we read the
/// POSIX `LC_ALL`/`LANG` environment the same way (e.g. `en_GB.UTF-8` → en/GB).
/// Falls back to `en`/`US` if unset.
fn default_locale_parts() -> (String, String) {
    let raw = std::env::var("LC_ALL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("LANG").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    // Strip the `.charset`/`@modifier` suffix: `en_GB.UTF-8` → `en_GB`.
    let base = raw.split(['.', '@']).next().unwrap_or("").trim();
    if base.is_empty() || base.eq_ignore_ascii_case("C") || base.eq_ignore_ascii_case("POSIX") {
        return ("en".to_string(), "US".to_string());
    }
    let mut parts = base.split(['_', '-']);
    let lang = parts.next().unwrap_or("en").to_lowercase();
    let country = parts.next().unwrap_or("").to_uppercase();
    (lang, country)
}

fn locale_country_name(code: &str) -> &'static str {
    match code {
        "AU" => "Australia",
        "BR" => "Brazil",
        "CA" => "Canada",
        "CH" => "Switzerland",
        "CN" => "China",
        "DE" => "Germany",
        "ES" => "Spain",
        "FR" => "France",
        "GB" => "United Kingdom",
        "IE" => "Ireland",
        "IN" => "India",
        "IT" => "Italy",
        "JP" => "Japan",
        "KR" => "South Korea",
        "MX" => "Mexico",
        "NL" => "Netherlands",
        "NZ" => "New Zealand",
        "PT" => "Portugal",
        "RU" => "Russia",
        "TW" => "Taiwan",
        "US" => "United States",
        "ZA" => "South Africa",
        _ => "",
    }
}

/// Build a Locale instance shim carrying its language/country/variant and the
/// Java-style id (`en`, `en_US`, `en_US_POSIX`).
fn make_locale(lang: &str, country: &str, variant: &str) -> CfmlValue {
    let mut id = lang.to_string();
    if !country.is_empty() {
        id.push('_');
        id.push_str(country);
    }
    if !variant.is_empty() {
        // Java emits language_COUNTRY_VARIANT; if country is empty it uses a
        // double underscore, but cbi18n only feeds us 1-3 well-formed parts.
        id.push('_');
        id.push_str(variant);
    }
    let mut shim = jshim("java.util.locale");
    shim.insert("__locale_lang".to_string(), CfmlValue::string(lang.to_string()));
    shim.insert(
        "__locale_country".to_string(),
        CfmlValue::string(country.to_string()),
    );
    shim.insert(
        "__locale_variant".to_string(),
        CfmlValue::string(variant.to_string()),
    );
    shim.insert("__locale_id".to_string(), CfmlValue::string(id));
    CfmlValue::strukt(shim)
}

pub fn handle_java_locale(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    // Static factory methods first (callable on the class-ref shim).
    match method {
        "init" => {
            // createObject(...) → 0 args → class-ref shim.
            // new Locale(lang[,country[,variant]]) → instance.
            if args.is_empty() {
                return Ok(CfmlValue::strukt(jshim("java.util.locale")));
            }
            let lang = args.first().map(|v| v.as_string()).unwrap_or_default();
            let country = args.get(1).map(|v| v.as_string()).unwrap_or_default();
            let variant = args.get(2).map(|v| v.as_string()).unwrap_or_default();
            return Ok(make_locale(&lang, &country, &variant));
        }
        "getdefault" => {
            let (lang, country) = default_locale_parts();
            return Ok(make_locale(&lang, &country, ""));
        }
        "getavailablelocales" => {
            // Return real Locale shim objects (not strings): cbi18n's
            // isValidLocale() does arrayToList(...) for a listFind — and a
            // Locale shim stringifies to its id (see as_string) — while
            // getLocaleNames() calls `.getDisplayName()` on each element, so
            // they must be Locale objects, matching the JVM's Locale[].
            let arr: Vec<CfmlValue> = AVAILABLE_LOCALES
                .iter()
                .map(|id| {
                    let mut it = id.split('_');
                    let lang = it.next().unwrap_or("");
                    let country = it.next().unwrap_or("");
                    let variant = it.next().unwrap_or("");
                    make_locale(lang, country, variant)
                })
                .collect();
            return Ok(CfmlValue::array(arr));
        }
        "getisolanguages" => {
            let langs = [
                "ar", "bg", "ca", "cs", "da", "de", "el", "en", "es", "et", "fi", "fr", "he", "hi",
                "hr", "hu", "id", "is", "it", "ja", "ko", "lt", "lv", "nl", "no", "pl", "pt", "ro",
                "ru", "sk", "sl", "sr", "sv", "th", "tr", "uk", "vi", "zh",
            ];
            return Ok(CfmlValue::array(
                langs.iter().map(|s| CfmlValue::string(s.to_string())).collect(),
            ));
        }
        "getisocountries" => {
            let countries = [
                "AU", "BR", "CA", "CH", "CN", "DE", "ES", "FR", "GB", "IE", "IN", "IT", "JP", "KR",
                "MX", "NL", "NZ", "PT", "RU", "TW", "US", "ZA",
            ];
            return Ok(CfmlValue::array(
                countries.iter().map(|s| CfmlValue::string(s.to_string())).collect(),
            ));
        }
        _ => {}
    }
    // Instance getters.
    let s = match object {
        CfmlValue::Struct(s) => s,
        _ => return Ok(CfmlValue::Null),
    };
    let lang = s.get("__locale_lang").map(|v| v.as_string()).unwrap_or_default();
    let country = s.get("__locale_country").map(|v| v.as_string()).unwrap_or_default();
    let variant = s.get("__locale_variant").map(|v| v.as_string()).unwrap_or_default();
    let id = s.get("__locale_id").map(|v| v.as_string()).unwrap_or_default();
    Ok(match method {
        "getlanguage" => CfmlValue::string(lang),
        "getcountry" => CfmlValue::string(country),
        "getvariant" => CfmlValue::string(variant),
        "tostring" => CfmlValue::string(id),
        "getdisplaylanguage" => {
            let n = locale_language_name(&lang);
            CfmlValue::string(if n.is_empty() { lang } else { n.to_string() })
        }
        "getdisplaycountry" => {
            let n = locale_country_name(&country);
            CfmlValue::string(if n.is_empty() { country } else { n.to_string() })
        }
        "getdisplayname" => {
            let l = locale_language_name(&lang);
            let lname = if l.is_empty() { lang.clone() } else { l.to_string() };
            let c = locale_country_name(&country);
            if country.is_empty() || c.is_empty() {
                CfmlValue::string(lname)
            } else {
                CfmlValue::string(format!("{} ({})", lname, c))
            }
        }
        "getiso3language" => {
            let i = locale_iso3_language(&lang);
            CfmlValue::string(if i.is_empty() { lang } else { i.to_string() })
        }
        "getiso3country" => {
            let i = locale_iso3_country(&country);
            CfmlValue::string(if i.is_empty() { country } else { i.to_string() })
        }
        _ => CfmlValue::Null,
    })
}

pub fn handle_java_timezone(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    let make_tz = |id: &str| -> CfmlValue {
        let mut shim = jshim("java.util.timezone");
        shim.insert("__tz_id".to_string(), CfmlValue::string(id.to_string()));
        CfmlValue::strukt(shim)
    };
    match method {
        "init" => {
            // createObject → class-ref shim. Carry the LONG/SHORT static int
            // constants (TimeZone.LONG=1, TimeZone.SHORT=0) as fields so
            // `tz.LONG` property access resolves.
            let mut shim = jshim("java.util.timezone");
            shim.insert("long".to_string(), CfmlValue::Int(1));
            shim.insert("short".to_string(), CfmlValue::Int(0));
            return Ok(CfmlValue::strukt(shim));
        }
        "getdefault" => {
            let id = std::env::var("TZ").unwrap_or_else(|_| "UTC".to_string());
            return Ok(make_tz(&id));
        }
        "gettimezone" => {
            let id = args.first().map(|v| v.as_string()).unwrap_or_else(|| "UTC".to_string());
            return Ok(make_tz(&id));
        }
        "getavailableids" => {
            let ids = [
                "UTC", "GMT", "Europe/London", "Europe/Paris", "Europe/Berlin", "America/New_York",
                "America/Chicago", "America/Denver", "America/Los_Angeles", "Asia/Tokyo",
                "Asia/Shanghai", "Asia/Kolkata", "Australia/Sydney",
            ];
            return Ok(CfmlValue::array(
                ids.iter().map(|s| CfmlValue::string(s.to_string())).collect(),
            ));
        }
        _ => {}
    }
    let s = match object {
        CfmlValue::Struct(s) => s,
        _ => return Ok(CfmlValue::Null),
    };
    let id = s.get("__tz_id").map(|v| v.as_string()).unwrap_or_else(|| "UTC".to_string());
    Ok(match method {
        "getid" => CfmlValue::string(id),
        "getdisplayname" => CfmlValue::string(id),
        "getrawoffset" | "getdstsavings" | "getoffset" => CfmlValue::Int(0),
        "usedaylighttime" | "indaylighttime" => CfmlValue::Bool(false),
        _ => CfmlValue::Null,
    })
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn handle_java_gregoriancalendar(
    method: &str,
    _args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => {
            let mut shim = jshim("java.util.gregoriancalendar");
            shim.insert("__millis".to_string(), CfmlValue::Int(now_millis()));
            Ok(CfmlValue::strukt(shim))
        }
        "gettime" => {
            // Returns a java.util.Date. Reuse the Date shim shape (`__millis`).
            let millis = match object {
                CfmlValue::Struct(s) => {
                    s.get("__millis").map(|v| v.as_string().parse::<i64>().unwrap_or(0)).unwrap_or(0)
                }
                _ => now_millis(),
            };
            let mut shim = jshim("java.util.date");
            shim.insert("__millis".to_string(), CfmlValue::Int(millis));
            Ok(CfmlValue::strukt(shim))
        }
        "gettimeinmillis" => match object {
            CfmlValue::Struct(s) => Ok(s.get("__millis").unwrap_or(CfmlValue::Int(now_millis()))),
            _ => Ok(CfmlValue::Int(now_millis())),
        },
        _ => Ok(CfmlValue::Null),
    }
}

pub fn handle_java_dateformatsymbols(
    method: &str,
    _args: Vec<CfmlValue>,
    _object: &CfmlValue,
) -> CfmlResult {
    let arr = |items: &[&str]| {
        CfmlValue::array(items.iter().map(|s| CfmlValue::string(s.to_string())).collect())
    };
    match method {
        "init" => Ok(CfmlValue::strukt(jshim("java.text.dateformatsymbols"))),
        "getmonths" => Ok(arr(&[
            "January", "February", "March", "April", "May", "June", "July", "August", "September",
            "October", "November", "December", "",
        ])),
        "getshortmonths" => Ok(arr(&[
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec", "",
        ])),
        "getweekdays" => Ok(arr(&[
            "", "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday",
        ])),
        "getshortweekdays" => {
            Ok(arr(&["", "Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]))
        }
        "getampmstrings" => Ok(arr(&["AM", "PM"])),
        "geteras" => Ok(arr(&["BC", "AD"])),
        _ => Ok(CfmlValue::Null),
    }
}

pub fn handle_java_decimalformatsymbols(
    method: &str,
    _args: Vec<CfmlValue>,
    _object: &CfmlValue,
) -> CfmlResult {
    // cbi18n calls `.toString()` on each returned symbol; we return Strings, and
    // String.toString() is the identity, so the chain works.
    let s = |t: &str| Ok(CfmlValue::string(t.to_string()));
    match method {
        "init" => Ok(CfmlValue::strukt(jshim("java.text.decimalformatsymbols"))),
        "getpercent" => s("%"),
        "getminussign" => s("-"),
        "getcurrencysymbol" => s("$"),
        "getinternationalcurrencysymbol" => s("USD"),
        "getmonetarydecimalseparator" | "getdecimalseparator" => s("."),
        "getgroupingseparator" => s(","),
        "getexponentseparator" => s("E"),
        "getpermill" => s("\u{2030}"),
        "getplussign" => s("+"),
        "getzerodigit" => s("0"),
        "getinfinity" => s("\u{221e}"),
        "getnan" => s("NaN"),
        _ => Ok(CfmlValue::Null),
    }
}

/// Locale-aware date/time pattern for `(kind, dateStyle, timeStyle)`, matching
/// what the JVM's `DateFormat.getXInstance(style, locale).toPattern()` returns
/// (verified against Lucee 7.0.4 / OpenJDK 21). Only locales we have explicitly
/// ground-truthed are tabulated; an unverified locale returns Err so we never
/// emit a guessed pattern. Style ints: FULL=0, LONG=1, MEDIUM=2, SHORT=3.
fn java_date_time_pattern(
    locale_id: &str,
    kind: &str,
    date_style: i64,
    time_style: i64,
) -> Result<String, CfmlError> {
    let lc = locale_id.to_lowercase();
    // en, en_US, en_CA, … default to the US CLDR forms; en_GB (and en_IE/en_AU
    // which share the day-first/24h forms) use the GB forms. We only claim the
    // ones verified below.
    let date_pat = |style: i64| -> Option<&'static str> {
        match lc.as_str() {
            "en" | "en_us" => Some(match style {
                0 => "EEEE, MMMM d, y",
                1 => "MMMM d, y",
                2 => "MMM d, y",
                3 => "M/d/yy",
                _ => return None,
            }),
            "en_gb" => Some(match style {
                0 => "EEEE, d MMMM y",
                1 => "d MMMM y",
                2 => "d MMM y",
                3 => "dd/MM/y",
                _ => return None,
            }),
            _ => None,
        }
    };
    let time_pat = |style: i64| -> Option<&'static str> {
        match lc.as_str() {
            // NB: the separator before the AM/PM marker is U+202F (narrow
            // no-break space), matching the JDK 21 / CLDR pattern — NOT an ASCII
            // space. The JVM emits e.g. "2:05\u{202f}PM"; a plain space would
            // diverge byte-for-byte from Lucee.
            "en" | "en_us" => Some(match style {
                0 => "h:mm:ss\u{202f}a zzzz",
                1 => "h:mm:ss\u{202f}a z",
                2 => "h:mm:ss\u{202f}a",
                3 => "h:mm\u{202f}a",
                _ => return None,
            }),
            "en_gb" => Some(match style {
                0 => "HH:mm:ss zzzz",
                1 => "HH:mm:ss z",
                2 => "HH:mm:ss",
                3 => "HH:mm",
                _ => return None,
            }),
            _ => None,
        }
    };
    let unsupported = || {
        CfmlError::runtime(format!(
            "java.text.DateFormat: locale [{}] is not supported by RustCFML's \
             Java shim (only en/en_US/en_GB are CLDR-verified). Add it after \
             ground-truthing its patterns against the JVM, or use CFML's \
             lsDateFormat()/lsDateTimeFormat() which are locale-aware.",
            locale_id
        ))
    };
    match kind {
        "date" => date_pat(date_style).map(|s| s.to_string()).ok_or_else(unsupported),
        "time" => time_pat(time_style).map(|s| s.to_string()).ok_or_else(unsupported),
        // DateFormat.getDateTimeInstance joins the two with ", " (verified:
        // "M/d/yy, h:mm a").
        _ => {
            let d = date_pat(date_style).ok_or_else(unsupported)?;
            let t = time_pat(time_style).ok_or_else(unsupported)?;
            Ok(format!("{}, {}", d, t))
        }
    }
}

enum OffsetStyle {
    /// RFC822: `+0000`, `-0400` (Java `Z`).
    Rfc822,
    /// ISO8601: `Z`, `-04`, `+0530` (Java `X`/`XX`).
    Iso8601,
    /// ISO8601 with colon: `Z`, `-04:00` (Java `XXX`).
    Iso8601Colon,
    /// Localized GMT short: `GMT`, `GMT-4`, `GMT+5:30` (Java `O`).
    GmtShort,
    /// Localized GMT long: `GMT`, `GMT-04:00` (Java `OOOO`).
    GmtColon,
}

/// Format a signed (east-positive) UTC offset in seconds per a Java zone style.
fn format_offset(offset_secs: i64, style: OffsetStyle) -> String {
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let abs = offset_secs.abs();
    let h = abs / 3600;
    let m = (abs % 3600) / 60;
    match style {
        OffsetStyle::Rfc822 => format!("{}{:02}{:02}", sign, h, m),
        OffsetStyle::Iso8601 => {
            if offset_secs == 0 {
                "Z".to_string()
            } else if m == 0 {
                format!("{}{:02}", sign, h)
            } else {
                format!("{}{:02}{:02}", sign, h, m)
            }
        }
        OffsetStyle::Iso8601Colon => {
            if offset_secs == 0 {
                "Z".to_string()
            } else {
                format!("{}{:02}:{:02}", sign, h, m)
            }
        }
        OffsetStyle::GmtShort => {
            if offset_secs == 0 {
                "GMT".to_string()
            } else if m == 0 {
                format!("GMT{}{}", sign, h)
            } else {
                format!("GMT{}{}:{:02}", sign, h, m)
            }
        }
        OffsetStyle::GmtColon => {
            if offset_secs == 0 {
                "GMT".to_string()
            } else {
                format!("GMT{}{:02}:{:02}", sign, h, m)
            }
        }
    }
}

/// The timezone facts a formatter needs to render zone pattern fields, resolved
/// for the specific instant being formatted (so DST is already decided).
struct ZoneCtx {
    /// Abbreviation for this instant (e.g. "EDT" / "EST"), from the verified
    /// table. `None` when the zone is valid but not tabulated — a `z`/`zzzz`
    /// field then fails loudly rather than guessing.
    short: Option<String>,
    /// Long display name for this instant (e.g. "Eastern Daylight Time").
    long: Option<String>,
    /// Canonical zone id, for the error message when names are missing.
    id: String,
    /// Signed UTC offset in seconds, east positive (EDT = -14400).
    offset_secs: i64,
}

/// Render a `NaiveDateTime` per a Java `SimpleDateFormat` pattern, using English
/// month/weekday names (we only support en* locales). Timezone fields
/// (`z`/`Z`/`X`/`O`) are rendered from `zone`; if `zone` is `None` (no resolvable
/// zone) or the field is unsupported (`v` generic), it returns Err rather than
/// guess.
fn format_java_pattern(
    dt: &NaiveDateTime,
    pattern: &str,
    zone: Option<&ZoneCtx>,
) -> Result<String, CfmlError> {
    const MONTHS_FULL: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September",
        "October", "November", "December",
    ];
    const MONTHS_SHORT: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    // chrono Weekday::num_days_from_sunday(): Sun=0 .. Sat=6.
    const WEEKDAYS_FULL: [&str; 7] = [
        "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday",
    ];
    const WEEKDAYS_SHORT: [&str; 7] =
        ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let month0 = (dt.month() as usize).saturating_sub(1).min(11);
    let wd = dt.weekday().num_days_from_sunday() as usize;
    let hour24 = dt.hour();
    let hour12 = match hour24 % 12 {
        0 => 12,
        h => h,
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            // Java quotes literal text in single quotes; '' is a literal quote.
            i += 1;
            if i < chars.len() && chars[i] == '\'' {
                out.push('\'');
                i += 1;
                continue;
            }
            while i < chars.len() && chars[i] != '\'' {
                out.push(chars[i]);
                i += 1;
            }
            i += 1; // skip closing quote
            continue;
        }
        if !c.is_ascii_alphabetic() {
            out.push(c);
            i += 1;
            continue;
        }
        // Count the run of the same pattern letter.
        let mut n = 0;
        while i < chars.len() && chars[i] == c {
            n += 1;
            i += 1;
        }
        match c {
            'y' | 'Y' => {
                if n == 2 {
                    out.push_str(&format!("{:02}", (dt.year() % 100).abs()));
                } else {
                    out.push_str(&dt.year().to_string());
                }
            }
            'M' | 'L' => match n {
                1 => out.push_str(&dt.month().to_string()),
                2 => out.push_str(&format!("{:02}", dt.month())),
                3 => out.push_str(MONTHS_SHORT[month0]),
                _ => out.push_str(MONTHS_FULL[month0]),
            },
            'd' => {
                if n >= 2 {
                    out.push_str(&format!("{:02}", dt.day()));
                } else {
                    out.push_str(&dt.day().to_string());
                }
            }
            'E' | 'e' | 'c' => {
                if n >= 4 {
                    out.push_str(WEEKDAYS_FULL[wd]);
                } else {
                    out.push_str(WEEKDAYS_SHORT[wd]);
                }
            }
            'h' => {
                if n >= 2 {
                    out.push_str(&format!("{:02}", hour12));
                } else {
                    out.push_str(&hour12.to_string());
                }
            }
            'H' => {
                if n >= 2 {
                    out.push_str(&format!("{:02}", hour24));
                } else {
                    out.push_str(&hour24.to_string());
                }
            }
            'm' => {
                if n >= 2 {
                    out.push_str(&format!("{:02}", dt.minute()));
                } else {
                    out.push_str(&dt.minute().to_string());
                }
            }
            's' => {
                if n >= 2 {
                    out.push_str(&format!("{:02}", dt.second()));
                } else {
                    out.push_str(&dt.second().to_string());
                }
            }
            'a' => out.push_str(if hour24 < 12 { "AM" } else { "PM" }),
            'z' | 'Z' | 'X' | 'O' => {
                let z = zone.ok_or_else(|| {
                    CfmlError::runtime(format!(
                        "java.text.DateFormat: pattern timezone field '{}' needs a \
                         resolvable timezone but none is available.",
                        c
                    ))
                })?;
                let need_name = |name: &Option<String>| -> Result<String, CfmlError> {
                    name.clone().ok_or_else(|| {
                        CfmlError::runtime(format!(
                            "java.text.DateFormat: timezone [{}] is valid but its display \
                             name is not in the verified table (only common zones are \
                             tabulated). See docs/lucee-differences.md.",
                            z.id
                        ))
                    })
                };
                match c {
                    // z/zz/zzz → short abbreviation; zzzz → long display name.
                    'z' if n >= 4 => out.push_str(&need_name(&z.long)?),
                    'z' => out.push_str(&need_name(&z.short)?),
                    // Z → RFC822 numeric offset, e.g. "-0400".
                    'Z' => out.push_str(&format_offset(z.offset_secs, OffsetStyle::Rfc822)),
                    // X/XX/XXX → ISO8601 ("Z" for UTC); XXX uses a colon.
                    'X' => out.push_str(&format_offset(
                        z.offset_secs,
                        if n >= 3 { OffsetStyle::Iso8601Colon } else { OffsetStyle::Iso8601 },
                    )),
                    // O/OOOO → localized GMT offset, e.g. "GMT-4" / "GMT-04:00".
                    'O' => out.push_str(&format_offset(
                        z.offset_secs,
                        if n >= 4 { OffsetStyle::GmtColon } else { OffsetStyle::GmtShort },
                    )),
                    _ => unreachable!(),
                }
            }
            'v' => {
                // Generic non-location zone name ("ET") — needs CLDR generic
                // data we don't carry. Fail loudly rather than guess.
                return Err(CfmlError::runtime(
                    "java.text.DateFormat: generic timezone field 'v' is not supported."
                        .to_string(),
                ));
            }
            _ => {
                // Unhandled pattern letter — fail rather than drop or guess.
                return Err(CfmlError::runtime(format!(
                    "java.text.DateFormat: unsupported pattern field '{}'",
                    c
                )));
            }
        }
    }
    Ok(out)
}

/// Parse the argument handed to `DateFormat.format(...)` into a wall-clock
/// `NaiveDateTime` *in `tz`* plus the zone's offset facts at that instant.
/// cbi18n passes either a numeric Java epoch-millis instant (the `i18n*Format`
/// methods) or a CFML date value/string (the `*LocaleFormat` methods):
///   - epoch millis is an absolute instant → shifted into `tz`'s wall clock,
///     with the offset/DST taken at that instant.
///   - a CFML date string is already a wall clock → used verbatim, with the
///     offset/DST computed by interpreting it as local time in `tz`.
/// Returns `(wall_clock, offset_secs, is_dst)`.
fn parse_dateformat_arg(arg: &CfmlValue, tz: &chrono_tz::Tz) -> Option<(NaiveDateTime, i64, bool)> {
    let from_epoch_millis = |ms: i64| -> Option<(NaiveDateTime, i64, bool)> {
        let utc = chrono::DateTime::from_timestamp_millis(ms)?.naive_utc();
        let info = crate::tz::offset_info_at(tz, utc);
        Some((crate::tz::utc_to_local(tz, utc), info.total_secs, info.is_dst()))
    };
    match arg {
        CfmlValue::Int(n) => from_epoch_millis(*n),
        CfmlValue::Double(d) => from_epoch_millis(*d as i64),
        CfmlValue::Struct(s) if s.contains_key("__millis") => {
            let ms = s.get("__millis").map(|v| match v {
                CfmlValue::Int(n) => n,
                other => other.as_string().trim().parse::<i64>().unwrap_or(0),
            })?;
            from_epoch_millis(ms)
        }
        other => {
            // A CFML date value — a wall-clock string; parse directly. The zone
            // offset/DST is decided by interpreting it as local time in `tz`.
            let s = other.as_string();
            let s = s.trim();
            let wall = {
                let mut found = None;
                for fmt in [
                    "%Y-%m-%d %H:%M:%S",
                    "%Y-%m-%dT%H:%M:%S",
                    "%Y-%m-%d %H:%M",
                    "%m/%d/%Y %H:%M:%S",
                    "%m/%d/%Y",
                ] {
                    if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
                        found = Some(dt);
                        break;
                    }
                }
                found.or_else(|| {
                    for fmt in ["%Y-%m-%d", "%m/%d/%Y", "%d/%m/%Y"] {
                        if let Ok(d) = chrono::NaiveDate::parse_from_str(s, fmt) {
                            return d.and_hms_opt(0, 0, 0);
                        }
                    }
                    None
                })?
            };
            let info = crate::tz::offset_info_for_local(tz, wall);
            Some((wall, info.total_secs, info.is_dst()))
        }
    }
}

/// Shared implementation for java.text.DateFormat and java.text.SimpleDateFormat.
///
/// The factory methods (`getDateInstance`/`getTimeInstance`/`getDateTimeInstance`)
/// return a formatter shim carrying its kind, style(s), locale id and (optional)
/// timezone; `format()` renders a date faithfully per the locale's CLDR pattern
/// (verified against the JVM via Lucee). Unsupported locales and timezone-name
/// pattern fields raise a clear error rather than emitting a guessed string.
pub fn handle_java_dateformat(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    // Read the locale id from a Locale shim arg (its as_string is its id).
    let locale_of = |v: Option<&CfmlValue>| -> String {
        v.map(|x| x.as_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "en".to_string())
    };
    let as_int = |v: Option<&CfmlValue>, default: i64| -> i64 {
        v.map(|x| match x {
            CfmlValue::Int(n) => *n,
            CfmlValue::Double(d) => *d as i64,
            other => other.as_string().trim().parse::<i64>().unwrap_or(default),
        })
        .unwrap_or(default)
    };
    let make_formatter = |kind: &str, ds: i64, ts: i64, loc: String| {
        let mut shim = jshim("java.text.dateformat");
        shim.insert("__df_kind".to_string(), CfmlValue::string(kind.to_string()));
        shim.insert("__df_date_style".to_string(), CfmlValue::Int(ds));
        shim.insert("__df_time_style".to_string(), CfmlValue::Int(ts));
        shim.insert("__df_locale".to_string(), CfmlValue::string(loc));
        CfmlValue::strukt(shim)
    };
    match method {
        "init" => {
            // createObject → class-ref shim carrying DateFormat's style int
            // constants (FULL=0, LONG=1, MEDIUM=2, SHORT=3) for `df[style]` /
            // `df.SHORT` access.
            let mut shim = jshim("java.text.dateformat");
            shim.insert("full".to_string(), CfmlValue::Int(0));
            shim.insert("long".to_string(), CfmlValue::Int(1));
            shim.insert("medium".to_string(), CfmlValue::Int(2));
            shim.insert("short".to_string(), CfmlValue::Int(3));
            Ok(CfmlValue::strukt(shim))
        }
        "getdateinstance" => {
            let style = as_int(args.first(), 2); // DateFormat.DEFAULT == MEDIUM
            Ok(make_formatter("date", style, 2, locale_of(args.get(1))))
        }
        "gettimeinstance" => {
            let style = as_int(args.first(), 2);
            Ok(make_formatter("time", 2, style, locale_of(args.get(1))))
        }
        "getdatetimeinstance" => {
            let ds = as_int(args.first(), 2);
            let ts = as_int(args.get(1), 2);
            Ok(make_formatter("datetime", ds, ts, locale_of(args.get(2))))
        }
        "getinstance" => Ok(make_formatter("datetime", 3, 3, "en".to_string())),
        "settimezone" => {
            // Store the bound zone id; return the (updated) receiver so the
            // chained `.format()` sees it.
            if let CfmlValue::Struct(ref s) = object {
                let mut ns = s.snapshot();
                let tz = args
                    .first()
                    .map(|v| match v {
                        CfmlValue::Struct(ts) => {
                            ts.get("__tz_id").map(|t| t.as_string()).unwrap_or_else(|| v.as_string())
                        }
                        other => other.as_string(),
                    })
                    .unwrap_or_default();
                ns.insert("__df_tz".to_string(), CfmlValue::string(tz));
                return Ok(CfmlValue::strukt(ns));
            }
            Ok(object.clone())
        }
        "setlenient" | "setcalendar" | "applypattern" => Ok(object.clone()),
        "format" => {
            let s = match object {
                CfmlValue::Struct(s) => s,
                _ => return Ok(CfmlValue::Null),
            };
            let kind = s.get("__df_kind").map(|v| v.as_string()).unwrap_or_else(|| "date".to_string());
            let ds = s.get("__df_date_style").map(|v| v.as_string().parse().unwrap_or(2)).unwrap_or(2);
            let ts = s.get("__df_time_style").map(|v| v.as_string().parse().unwrap_or(2)).unwrap_or(2);
            let locale = s.get("__df_locale").map(|v| v.as_string()).unwrap_or_else(|| "en".to_string());
            // The formatter's bound zone (setTimeZone) or, when unset, the JVM
            // default — i.e. the host system zone (TimeZone.getDefault()).
            let tz_id = s
                .get("__df_tz")
                .map(|v| v.as_string())
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(crate::tz::system_tz_id);
            let zone = crate::tz::resolve_tz(&tz_id).ok_or_else(|| {
                CfmlError::runtime(format!(
                    "java.text.DateFormat.format(): unknown timezone id [{}].",
                    tz_id
                ))
            })?;
            let pattern = java_date_time_pattern(&locale, &kind, ds, ts)?;
            let (dt, offset_secs, is_dst) =
                parse_dateformat_arg(args.first().unwrap_or(&CfmlValue::Null), &zone).ok_or_else(
                    || {
                        CfmlError::runtime(
                            "java.text.DateFormat.format(): could not interpret the date \
                             argument."
                                .to_string(),
                        )
                    },
                )?;
            let names = crate::tz::names_for(&zone);
            let zone_ctx = ZoneCtx {
                short: names.map(|(std, dst, _, _)| {
                    if is_dst { dst.to_string() } else { std.to_string() }
                }),
                long: names.map(|(_, _, std, dst)| {
                    if is_dst { dst.to_string() } else { std.to_string() }
                }),
                id: crate::tz::canonical_name(&zone),
                offset_secs,
            };
            Ok(CfmlValue::string(format_java_pattern(&dt, &pattern, Some(&zone_ctx))?))
        }
        _ => Ok(CfmlValue::Null),
    }
}
