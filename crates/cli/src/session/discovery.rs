//! Peer discovery strategies for the clustered session store.
//!
//! Three methods are supported:
//!
//! - **Static** — fixed `host:port` list. Useful for tests and small fixed
//!   clusters. No periodic refresh.
//! - **DNS** — periodically resolves a hostname's A/AAAA records and yields
//!   the resulting addresses. This is the strategy to use on Fly.io
//!   (`<FLY_APP_NAME>.internal`), Kubernetes headless services, ECS Service
//!   Discovery, etc.
//! - **Kubernetes** — lists the pods matching a label selector through the
//!   Kubernetes API (what JGroups' `KUBE_PING` does), using the pod's service
//!   account. Needs RBAC `list` on `pods` in the namespace; no headless
//!   Service is required.
//! - **Multicast** — UDP multicast announce/listen on an admin-scoped group.
//!   Works on LANs, bare metal, VMware, and on Kubernetes CNIs that carry
//!   multicast (Calico VXLAN, Weave, Flannel VXLAN). Does **not** work on
//!   AWS VPC CNI, Fly.io 6PN, or most cloud-default networks.
//!
//! `Discovery::discover()` returns the currently-known peer addresses;
//! `ClusterStore` calls it on a timer and feeds new entries into
//! `memberlist.join_many()`.

use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

const DEFAULT_DNS_INTERVAL: Duration = Duration::from_secs(10);
const DEFAULT_MCAST_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_K8S_INTERVAL: Duration = Duration::from_secs(10);
const K8S_SA_DIR: &str = "/var/run/secrets/kubernetes.io/serviceaccount";
const MCAST_ANNOUNCE_TAG: &[u8] = b"RCFM1";

#[derive(Clone)]
pub enum Discovery {
    Static(StaticSeeds),
    Dns(DnsDiscovery),
    Multicast(MulticastDiscovery),
    Kubernetes(KubernetesDiscovery),
}

impl Discovery {
    pub async fn discover(&self) -> Vec<SocketAddr> {
        match self {
            Discovery::Static(s) => s.snapshot(),
            Discovery::Dns(d) => d.resolve().await,
            Discovery::Multicast(m) => m.snapshot(),
            Discovery::Kubernetes(k) => k.list().await,
        }
    }

    /// Periodic re-discovery interval. `None` means "one-shot, never refresh".
    pub fn interval(&self) -> Option<Duration> {
        match self {
            Discovery::Static(_) => None,
            Discovery::Dns(d) => Some(d.interval),
            Discovery::Multicast(m) => Some(m.interval),
            Discovery::Kubernetes(k) => Some(k.interval),
        }
    }

    /// Short human-readable label for log lines.
    pub fn label(&self) -> String {
        match self {
            Discovery::Static(s) => format!("static[{}]", s.seeds.len()),
            Discovery::Dns(d) => format!("dns({}:{})", d.name, d.port),
            Discovery::Multicast(m) => format!("multicast({}:{})", m.group, m.port),
            Discovery::Kubernetes(k) => {
                format!("kubernetes({}/{}:{})", k.namespace, k.label_selector, k.port)
            }
        }
    }
}

// ─────────────────────────────────────────────
// Static
// ─────────────────────────────────────────────

#[derive(Clone)]
pub struct StaticSeeds {
    seeds: Vec<SocketAddr>,
}

impl StaticSeeds {
    pub fn new(raw: &[String]) -> Self {
        let seeds = raw
            .iter()
            .filter_map(|s| match s.parse::<SocketAddr>() {
                Ok(sa) => Some(sa),
                Err(e) => {
                    eprintln!("[session/cluster] discovery: bad seed '{}': {}", s, e);
                    None
                }
            })
            .collect();
        Self { seeds }
    }

    fn snapshot(&self) -> Vec<SocketAddr> {
        self.seeds.clone()
    }
}

// ─────────────────────────────────────────────
// DNS
// ─────────────────────────────────────────────

#[derive(Clone)]
pub struct DnsDiscovery {
    pub name: String,
    pub port: u16,
    pub interval: Duration,
}

impl DnsDiscovery {
    pub fn new(name: String, port: u16, interval_secs: u64) -> Self {
        let interval = if interval_secs == 0 {
            DEFAULT_DNS_INTERVAL
        } else {
            Duration::from_secs(interval_secs)
        };
        Self { name, port, interval }
    }

    async fn resolve(&self) -> Vec<SocketAddr> {
        let target = format!("{}:{}", self.name, self.port);
        let result = tokio::net::lookup_host(target.clone()).await;
        match result {
            Ok(iter) => iter.collect(),
            Err(e) => {
                eprintln!(
                    "[session/cluster] discovery: DNS lookup of '{}' failed: {}",
                    target, e
                );
                Vec::new()
            }
        }
    }
}

// ─────────────────────────────────────────────
// Multicast
// ─────────────────────────────────────────────
//
// We send our own listen address as a small UDP datagram to a multicast
// group on a schedule, and listen for the same kind of datagram from
// peers. Discovered addresses accumulate in a Mutex<HashSet> which
// `snapshot()` returns. A peer is only forgotten when this process
// restarts — memberlist will mark genuinely-dead peers as failed and we
// don't need to expire them here.

#[derive(Clone)]
pub struct MulticastDiscovery {
    pub group: String,
    pub port: u16,
    pub interval: Duration,
    seen: Arc<Mutex<HashSet<SocketAddr>>>,
}

impl MulticastDiscovery {
    /// Start the announcer + listener tasks. `self_addr` is what we
    /// advertise to peers — must be reachable from them (so don't
    /// advertise `0.0.0.0:7946`; pass the real bind / advertise addr).
    pub fn start(
        group: String,
        port: u16,
        interval_secs: u64,
        self_addr: SocketAddr,
    ) -> Result<Self, String> {
        use socket2::{Domain, Protocol, Socket, Type};
        use std::net::{Ipv4Addr, SocketAddrV4};

        let interval = if interval_secs == 0 {
            DEFAULT_MCAST_INTERVAL
        } else {
            Duration::from_secs(interval_secs)
        };

        let group_ip: Ipv4Addr = group
            .parse()
            .map_err(|e| format!("invalid multicast group '{}': {}", group, e))?;
        if !group_ip.is_multicast() {
            return Err(format!("{} is not a multicast address", group_ip));
        }

        // Build the socket via socket2 so we can SO_REUSEADDR + join group
        // before handing the fd to tokio.
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
            .map_err(|e| format!("multicast socket() failed: {}", e))?;
        socket
            .set_reuse_address(true)
            .map_err(|e| format!("SO_REUSEADDR failed: {}", e))?;
        #[cfg(unix)]
        socket
            .set_reuse_port(true)
            .map_err(|e| format!("SO_REUSEPORT failed: {}", e))?;
        let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
        socket
            .bind(&bind_addr.into())
            .map_err(|e| format!("multicast bind {} failed: {}", bind_addr, e))?;
        socket
            .join_multicast_v4(&group_ip, &Ipv4Addr::UNSPECIFIED)
            .map_err(|e| format!("join_multicast_v4 {} failed: {}", group_ip, e))?;
        socket
            .set_multicast_loop_v4(true)
            .map_err(|e| format!("set_multicast_loop_v4 failed: {}", e))?;
        socket
            .set_nonblocking(true)
            .map_err(|e| format!("set_nonblocking failed: {}", e))?;

        let std_sock: std::net::UdpSocket = socket.into();
        let tokio_sock = tokio::net::UdpSocket::from_std(std_sock)
            .map_err(|e| format!("tokio UdpSocket::from_std failed: {}", e))?;
        let tokio_sock = Arc::new(tokio_sock);

        let seen = Arc::new(Mutex::new(HashSet::new()));

        // Announcer task: send `MCAST_ANNOUNCE_TAG || self_addr.to_string()`
        // to the group every `interval`.
        let send_sock = tokio_sock.clone();
        let group_target: SocketAddr = SocketAddr::new(group_ip.into(), port);
        let self_label = self_addr.to_string();
        tokio::spawn(async move {
            let mut payload = Vec::with_capacity(MCAST_ANNOUNCE_TAG.len() + self_label.len());
            payload.extend_from_slice(MCAST_ANNOUNCE_TAG);
            payload.extend_from_slice(self_label.as_bytes());
            let mut tick = tokio::time::interval(interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                if let Err(e) = send_sock.send_to(&payload, group_target).await {
                    eprintln!(
                        "[session/cluster] discovery: multicast send to {} failed: {}",
                        group_target, e
                    );
                }
            }
        });

        // Listener task: parse `MCAST_ANNOUNCE_TAG || addr_str` and stash.
        let recv_sock = tokio_sock.clone();
        let seen_w = seen.clone();
        let self_addr_filter = self_addr;
        tokio::spawn(async move {
            let mut buf = [0u8; 256];
            loop {
                let (n, _from) = match recv_sock.recv_from(&mut buf).await {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!(
                            "[session/cluster] discovery: multicast recv failed: {}",
                            e
                        );
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                };
                if n < MCAST_ANNOUNCE_TAG.len() {
                    continue;
                }
                if &buf[..MCAST_ANNOUNCE_TAG.len()] != MCAST_ANNOUNCE_TAG {
                    continue;
                }
                let addr_bytes = &buf[MCAST_ANNOUNCE_TAG.len()..n];
                let addr_str = match std::str::from_utf8(addr_bytes) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let parsed: SocketAddr = match addr_str.parse() {
                    Ok(a) => a,
                    Err(_) => continue,
                };
                if parsed == self_addr_filter {
                    continue;
                }
                if let Ok(mut s) = seen_w.lock() {
                    s.insert(parsed);
                }
            }
        });

        Ok(Self {
            group,
            port,
            interval,
            seen,
        })
    }

    fn snapshot(&self) -> Vec<SocketAddr> {
        match self.seen.lock() {
            Ok(s) => s.iter().copied().collect(),
            Err(_) => Vec::new(),
        }
    }
}

// ─────────────────────────────────────────────
// Kubernetes
// ─────────────────────────────────────────────

#[derive(Clone)]
pub struct KubernetesDiscovery {
    pub api_server: String,
    pub namespace: String,
    pub label_selector: String,
    pub port: u16,
    pub interval: Duration,
    token_path: String,
    ca_path: String,
}

impl KubernetesDiscovery {
    /// Empty `namespace` / `api_server` take the in-cluster defaults: the
    /// service account's own namespace, and the API service advertised in the
    /// pod's environment.
    pub fn new(
        api_server: &str,
        namespace: &str,
        label_selector: &str,
        port: u16,
        interval_secs: u64,
    ) -> Self {
        let namespace = if namespace.trim().is_empty() {
            std::fs::read_to_string(format!("{K8S_SA_DIR}/namespace"))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "default".to_string())
        } else {
            namespace.trim().to_string()
        };
        let api_server = if api_server.trim().is_empty() {
            let host = std::env::var("KUBERNETES_SERVICE_HOST").unwrap_or_default();
            let port = std::env::var("KUBERNETES_SERVICE_PORT").unwrap_or_else(|_| "443".into());
            if host.contains(':') {
                format!("https://[{host}]:{port}")
            } else {
                format!("https://{host}:{port}")
            }
        } else {
            api_server.trim().trim_end_matches('/').to_string()
        };
        let interval = if interval_secs == 0 {
            DEFAULT_K8S_INTERVAL
        } else {
            Duration::from_secs(interval_secs)
        };
        Self {
            api_server,
            namespace,
            label_selector: label_selector.trim().to_string(),
            port,
            interval,
            token_path: format!("{K8S_SA_DIR}/token"),
            ca_path: format!("{K8S_SA_DIR}/ca.crt"),
        }
    }

    async fn list(&self) -> Vec<SocketAddr> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.list_blocking())
            .await
            .unwrap_or_default()
    }

    fn list_blocking(&self) -> Vec<SocketAddr> {
        let url = format!(
            "{}/api/v1/namespaces/{}/pods?labelSelector={}",
            self.api_server,
            url_component(&self.namespace),
            url_component(&self.label_selector)
        );
        let agent = match self.agent() {
            Ok(a) => a,
            Err(e) => {
                eprintln!("[cluster] kubernetes discovery: {e}");
                return Vec::new();
            }
        };
        let mut req = agent.get(&url);
        // The token rotates (projected service-account tokens), so read it per
        // call rather than once.
        if let Ok(token) = std::fs::read_to_string(&self.token_path) {
            req = req.set("Authorization", &format!("Bearer {}", token.trim()));
        }
        match req.call() {
            Ok(resp) => match resp.into_string() {
                Ok(body) => parse_pod_list(&body, self.port),
                Err(e) => {
                    eprintln!("[cluster] kubernetes discovery: reading the pod list failed: {e}");
                    Vec::new()
                }
            },
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                eprintln!(
                    "[cluster] kubernetes discovery: {url} answered HTTP {code}{}",
                    if code == 403 {
                        " — the service account needs RBAC `list` on `pods` in this namespace"
                    } else {
                        ""
                    }
                );
                if !body.is_empty() {
                    eprintln!("[cluster] kubernetes discovery: {}", body.chars().take(300).collect::<String>());
                }
                Vec::new()
            }
            Err(e) => {
                eprintln!("[cluster] kubernetes discovery: {url}: {e}");
                Vec::new()
            }
        }
    }

    /// An agent that trusts the cluster CA when the service-account mount has
    /// one (the API server's certificate is signed by it), and the public roots
    /// otherwise.
    fn agent(&self) -> Result<ureq::Agent, String> {
        let builder = ureq::AgentBuilder::new().timeout(Duration::from_secs(5));
        if !self.api_server.starts_with("https://") {
            return Ok(builder.build());
        }
        let Ok(pem) = std::fs::read(&self.ca_path) else {
            return Ok(builder.build());
        };
        use rustls_pki_types::{pem::PemObject, CertificateDer};
        let mut roots = rustls::RootCertStore::empty();
        for cert in CertificateDer::pem_slice_iter(&pem) {
            let cert = cert.map_err(|e| format!("bad certificate in {}: {e}", self.ca_path))?;
            roots
                .add(cert)
                .map_err(|e| format!("unusable certificate in {}: {e}", self.ca_path))?;
        }
        let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("TLS setup failed: {e}"))?
            .with_root_certificates(roots)
            .with_no_client_auth();
        Ok(builder.tls_config(std::sync::Arc::new(config)).build())
    }
}

/// Percent-encode a query/path component (label selectors carry `=`, `,`, `!`).
fn url_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Peer addresses from a `PodList`: pods that are Running, have an IP, and are
/// not being deleted.
pub fn parse_pod_list(body: &str, port: u16) -> Vec<SocketAddr> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        eprintln!("[cluster] kubernetes discovery: the API server's answer was not JSON");
        return Vec::new();
    };
    let mut out = Vec::new();
    for pod in v.get("items").and_then(|i| i.as_array()).into_iter().flatten() {
        if pod.pointer("/metadata/deletionTimestamp").is_some_and(|d| !d.is_null()) {
            continue;
        }
        if pod.pointer("/status/phase").and_then(|p| p.as_str()) != Some("Running") {
            continue;
        }
        let Some(ip) = pod.pointer("/status/podIP").and_then(|p| p.as_str()) else {
            continue;
        };
        if let Ok(ip) = ip.parse::<std::net::IpAddr>() {
            out.push(SocketAddr::new(ip, port));
        }
    }
    out
}

#[cfg(test)]
mod kubernetes_tests {
    use super::*;

    #[test]
    fn pod_list_keeps_running_pods_with_an_ip() {
        let body = r#"{"kind":"PodList","items":[
            {"metadata":{"name":"a"},"status":{"phase":"Running","podIP":"10.0.0.5"}},
            {"metadata":{"name":"b"},"status":{"phase":"Pending","podIP":"10.0.0.6"}},
            {"metadata":{"name":"c","deletionTimestamp":"2026-09-25T10:00:00Z"},"status":{"phase":"Running","podIP":"10.0.0.7"}},
            {"metadata":{"name":"d"},"status":{"phase":"Running"}},
            {"metadata":{"name":"e"},"status":{"phase":"Running","podIP":"fd00::9"}}
        ]}"#;
        let addrs = parse_pod_list(body, 7946);
        assert_eq!(
            addrs,
            vec![
                "10.0.0.5:7946".parse::<SocketAddr>().unwrap(),
                "[fd00::9]:7946".parse::<SocketAddr>().unwrap(),
            ]
        );
    }

    #[test]
    fn bad_answers_yield_no_peers() {
        assert!(parse_pod_list("not json", 7946).is_empty());
        assert!(parse_pod_list(r#"{"kind":"Status","code":403}"#, 7946).is_empty());
    }

    #[test]
    fn label_selectors_are_encoded() {
        assert_eq!(url_component("app=my-proj,tier!=db"), "app%3Dmy-proj%2Ctier%21%3Ddb");
    }
}

