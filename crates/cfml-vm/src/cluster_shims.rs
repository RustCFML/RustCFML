//! Java classes behind `cbjgroups`, backed natively by [`crate::cluster_bus`].
//!
//! `cbjgroups`' CFML is engine-neutral except for one Java object,
//! `org.pixl8.cbjgroups.CbJGroupsClusterWrapper` (plus the JGroups `Message` /
//! `View` it hands its listener). Shimming exactly those runs the module — and
//! everything built on it (`preside-ext-cluster-helpers`,
//! `preside-ext-k8s-essentials`) — unchanged. The OSGi bundle install that
//! precedes the `createObject` goes through the engine's inert OSGi layer
//! (`osgi_shim`), as every bundled library's does: the class request is then
//! answered on its own merits.

use super::*;
use crate::cluster_bus;

pub(crate) const WRAPPER_CLASS: &str = "org.pixl8.cbjgroups.cbjgroupsclusterwrapper";
pub(crate) const MESSAGE_CLASS: &str = "org.jgroups.message";
pub(crate) const VIEW_CLASS: &str = "org.jgroups.view";
pub(crate) const APP_CONTEXT_CLASS: &str = "lucee.runtime.listener.applicationcontext";

/// Classes `createObject("java", …)` can construct here.
pub(crate) fn constructs(class_lower: &str) -> bool {
    class_lower == WRAPPER_CLASS
}

/// Classes whose methods this module dispatches.
pub(crate) fn handles(class_lower: &str) -> bool {
    matches!(
        class_lower,
        WRAPPER_CLASS | MESSAGE_CLASS | VIEW_CLASS | APP_CONTEXT_CLASS
    )
}

fn shim(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m
}

fn obj_field(object: &CfmlValue, key: &str) -> Option<CfmlValue> {
    match object {
        CfmlValue::Struct(s) => s.get(key),
        _ => None,
    }
}

fn sub_id(object: &CfmlValue) -> Option<u64> {
    obj_field(object, "__cluster_sub").and_then(|v| match v {
        CfmlValue::Int(i) => Some(i as u64),
        _ => None,
    })
}

fn unsupported(class: &str, method: &str) -> CfmlError {
    CfmlError::runtime(format!(
        "{}.{}() is not supported by RustCFML",
        class, method
    ))
}

impl CfmlVirtualMachine {
    /// `createObject("java", <class>)` for the classes in [`constructs`].
    pub(crate) fn construct_cluster_shim(&self, class_lower: &str) -> CfmlResult {
        let class = match class_lower {
            WRAPPER_CLASS => "org.pixl8.cbjgroups.CbJGroupsClusterWrapper",
            other => return Err(unsupported(other, "<init>")),
        };
        Ok(CfmlValue::strukt(shim(class)))
    }

    /// Method dispatch for the classes in [`handles`].
    pub(crate) fn dispatch_cluster_shim(
        &mut self,
        class_lower: &str,
        method: &str,
        args: Vec<CfmlValue>,
        object: &CfmlValue,
    ) -> CfmlResult {
        match class_lower {
            WRAPPER_CLASS => self.cluster_wrapper_method(method, args, object),
            MESSAGE_CLASS => match method {
                "getbuffer" | "getrawbuffer" | "getarray" => {
                    Ok(obj_field(object, "__buffer").unwrap_or(CfmlValue::Binary(Vec::new())))
                }
                "getlength" => Ok(CfmlValue::Int(match obj_field(object, "__buffer") {
                    Some(CfmlValue::Binary(b)) => b.len() as i64,
                    _ => 0,
                })),
                other => Err(unsupported("org.jgroups.Message", other)),
            },
            VIEW_CLASS => {
                let members = obj_field(object, "__members").unwrap_or(CfmlValue::array(Vec::new()));
                match method {
                    "getmembers" | "getmembersraw" => Ok(members),
                    "size" => Ok(CfmlValue::Int(match &members {
                        CfmlValue::Array(a) => a.len() as i64,
                        _ => 0,
                    })),
                    "tostring" => {
                        let list = match &members {
                            CfmlValue::Array(a) => a
                                .snapshot()
                                .iter()
                                .map(|m| m.as_string())
                                .collect::<Vec<_>>()
                                .join(", "),
                            _ => String::new(),
                        };
                        Ok(CfmlValue::string(format!("[{}]", list)))
                    }
                    other => Err(unsupported("org.jgroups.View", other)),
                }
            }
            APP_CONTEXT_CLASS => match method {
                "getname" => Ok(obj_field(object, "__app_name").unwrap_or(CfmlValue::Null)),
                other => Err(unsupported("lucee.runtime.listener.ApplicationContext", other)),
            },
            other => Err(unsupported(other, method)),
        }
    }

    /// `getPageContext().getApplicationContext()`: a handle naming the current
    /// application, which `setApplicationContext` accepts back.
    pub(crate) fn page_application_context(&self) -> CfmlValue {
        let mut m = shim("lucee.runtime.listener.ApplicationContext");
        m.insert(
            "__app_name".to_string(),
            self.current_application_name
                .clone()
                .map(CfmlValue::string)
                .unwrap_or(CfmlValue::Null),
        );
        CfmlValue::strukt(m)
    }

    /// `getPageContext().setApplicationContext(ctx)`. A cluster delivery already
    /// runs in the application that created the channel, so re-selecting that
    /// same application is satisfied; switching to a different one is not
    /// something this engine can do mid-request, and says so.
    pub(crate) fn set_page_application_context(&self, ctx: Option<&CfmlValue>) -> CfmlResult {
        let wanted = ctx.and_then(|c| obj_field(c, "__app_name")).map(|v| v.as_string());
        let current = self.current_application_name.clone();
        match (wanted, current) {
            (None, _) => Ok(CfmlValue::Null),
            (Some(w), Some(c)) if w.eq_ignore_ascii_case(&c) => Ok(CfmlValue::Null),
            (Some(w), c) => Err(CfmlError::runtime(format!(
                "setApplicationContext cannot switch this request from application [{}] to [{}]",
                c.unwrap_or_default(),
                w
            ))),
        }
    }

    fn cluster_wrapper_method(
        &mut self,
        method: &str,
        args: Vec<CfmlValue>,
        object: &CfmlValue,
    ) -> CfmlResult {
        const CLASS: &str = "org.pixl8.cbjgroups.CbJGroupsClusterWrapper";
        match method {
            // init(configFilePath, discardOwnMessages, listenerCfc, loggerCfc, contextRoot)
            "init" => {
                let config_path = args.first().map(|v| v.as_string()).unwrap_or_default();
                let discard_own = args.get(1).map(|v| v.is_true()).unwrap_or(true);
                let listener = args.get(2).cloned().unwrap_or(CfmlValue::Null);
                if matches!(listener, CfmlValue::Null) {
                    return Err(CfmlError::runtime(format!(
                        "{}.init() requires a listener component",
                        CLASS
                    )));
                }
                let spawn = match self.thread_spawn_fn {
                    Some(f) => f,
                    None => {
                        return Err(CfmlError::runtime(format!(
                            "{} needs background threads to deliver cluster messages; \
                             they are available under --serve",
                            CLASS
                        )))
                    }
                };
                if !config_path.trim().is_empty() {
                    static WARNED: std::sync::Once = std::sync::Once::new();
                    WARNED.call_once(|| {
                        eprintln!(
                            "[cluster] cbjgroups: ignoring the JGroups configuration [{}]; \
                             cluster membership comes from RustCFML's `cluster` config{}",
                            config_path,
                            if cluster_bus::transport().is_some() {
                                ""
                            } else {
                                " (none configured: running as a single-node cluster)"
                            }
                        );
                    });
                }
                let seed = self.build_task_seed(CfmlValue::Null, None, None);
                let id = cluster_bus::subscribe(listener, seed, spawn, discard_own);
                if let CfmlValue::Struct(s) = object {
                    s.insert("__cluster_sub".to_string(), CfmlValue::Int(id as i64));
                }
                Ok(object.clone())
            }
            "connect" => {
                let id = sub_id(object).ok_or_else(|| unsupported(CLASS, "connect before init"))?;
                let name = args.first().map(|v| v.as_string()).unwrap_or_default();
                if name.trim().is_empty() {
                    return Err(CfmlError::runtime(format!("{}.connect() requires a cluster name", CLASS)));
                }
                cluster_bus::connect(id, name.trim());
                Ok(CfmlValue::Bool(true))
            }
            "isconnected" => Ok(CfmlValue::Bool(sub_id(object).is_some_and(cluster_bus::is_connected))),
            "close" => {
                if let Some(id) = sub_id(object) {
                    cluster_bus::close(id);
                }
                Ok(CfmlValue::Null)
            }
            "sendmessage" => {
                let id = sub_id(object).ok_or_else(|| unsupported(CLASS, "sendMessage before init"))?;
                let payload = match args.first() {
                    Some(CfmlValue::Binary(b)) => b.clone(),
                    Some(v) => v.as_string().into_bytes(),
                    None => Vec::new(),
                };
                cluster_bus::send(id, payload)
                    .map(|_| CfmlValue::Null)
                    .map_err(|e| CfmlError::runtime(format!("{}.sendMessage(): {}", CLASS, e)))
            }
            "getstats" => {
                let id = sub_id(object).ok_or_else(|| unsupported(CLASS, "getStats before init"))?;
                Ok(CfmlValue::strukt(cluster_bus::stats(id)))
            }
            "iscoordinator" => Ok(CfmlValue::Bool(sub_id(object).map(cluster_bus::is_coordinator).unwrap_or(true))),
            other => Err(unsupported(CLASS, other)),
        }
    }
}
