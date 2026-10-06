//! The sync manager: owns the device identity, advertises and discovers peers
//! over mDNS, listens for incoming connections, runs pairing flows, schedules
//! sync sessions, and reports status to the frontend.
//!
//! Networking model (deliberately low-churn):
//! - One TCP listener on a stable per-device port, advertised via mDNS/Bonjour.
//! - A continuous mDNS browse (event-driven; no polling of peers).
//! - A sync session starts when a paired peer is discovered, when the user
//!   clicks "Sync now", or shortly after local data changes (debounced).
//! - Failed attempts back off exponentially up to 10 minutes.

use anyhow::{anyhow, Result};
use mdns_sd::{IfKind, IfPredicate, ServiceDaemon, ServiceEvent, ServiceInfo};
use rusqlite::OptionalExtension;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_rustls::server::TlsStream;

use crate::commands::validate_setting;
use crate::data::store::{self, SettingsHandle};
use crate::DbHandle;

use super::engine::{self, SyncHost};
use super::identity::{self, DeviceIdentity};
use super::pairing::{self, IdentityExchange};
use super::protocol::{read_message, send_message, Hello, Message, PROTOCOL_VERSION};
use super::store::{self as sync_store, SyncPeer};
use super::transport;

const SERVICE_TYPE: &str = "_verenu._tcp.local.";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(6);
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);
pub(crate) const MAX_INCOMING_CONNECTIONS: usize = 8;
const PAIRING_TIMEOUT: Duration = Duration::from_secs(180);
const PAIRING_PROMPT_LIFETIME: Duration = Duration::from_secs(180);
const MAX_BACKOFF: Duration = Duration::from_secs(600);
const CHANGE_CHECK_INTERVAL: Duration = Duration::from_millis(750);
const IDLE_SYNC_INTERVAL: Duration = Duration::from_secs(30);
const PRIVATE_PORT_START: u16 = 49_152;
const PRIVATE_PORT_COUNT: u32 = 16_384;
const VIRTUAL_INTERFACE_MARKERS: &[&str] = &[
    "tailscale",
    "wireguard",
    "zerotier",
    "hamachi",
    "openvpn",
    "anyconnect",
    "wintun",
    "hyper-v",
    "hyperv",
    "vethernet",
    "virtualbox",
    "vmware",
    "virtual",
    "docker",
    "wsl",
    "teredo",
    "isatap",
];

fn is_tailscale_address(address: Ipv4Addr) -> bool {
    let [first, second, ..] = address.octets();
    first == 100 && (64..=127).contains(&second)
}

fn is_link_local_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => address.is_link_local(),
        IpAddr::V6(address) => address.is_unicast_link_local(),
    }
}

fn is_discovery_advertised_address_allowed(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            !address.is_loopback()
                && !address.is_unspecified()
                && !address.is_link_local()
                && !is_tailscale_address(address)
        }
        IpAddr::V6(_) => false,
    }
}

pub(crate) fn is_discovery_interface_allowed(name: &str, address: IpAddr, is_p2p: bool) -> bool {
    if is_p2p || address.is_loopback() || address.is_unspecified() || is_link_local_address(address)
    {
        return false;
    }

    if matches!(address, IpAddr::V4(address) if is_tailscale_address(address)) {
        return false;
    }

    let normalized_name = name.trim().to_ascii_lowercase();
    !VIRTUAL_INTERFACE_MARKERS
        .iter()
        .any(|marker| normalized_name.contains(marker))
}

/// Keep a device on the same private TCP port across app restarts. mDNS
/// caches can retain the previous advertisement for part of its TTL; a random
/// listener port turns that otherwise harmless stale record into an immediate
/// connection refusal during pairing.
pub(crate) fn listener_port_for_uuid(uuid: &str) -> u16 {
    let hash = uuid
        .as_bytes()
        .iter()
        .fold(2_166_136_261_u32, |hash, byte| {
            (hash ^ u32::from(*byte)).wrapping_mul(16_777_619)
        });
    PRIVATE_PORT_START + (hash % PRIVATE_PORT_COUNT) as u16
}

/// Pick exactly one automatic initiator for a peer pair. Manual sync remains
/// available from either device, but discovery/pairing must not start two
/// competing sessions at once.
pub(crate) fn should_auto_initiate(local_uuid: &str, peer_uuid: &str) -> bool {
    local_uuid < peer_uuid
}

#[derive(Clone)]
pub struct SyncManager {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub db: DbHandle,
    pub app: AppHandle,
    pub data_dir: PathBuf,
    pub identity: RwLock<Option<Arc<DeviceIdentity>>>,
    pub pending: tokio::sync::Mutex<Option<PendingPairing>>,
    pub pairing_status: Mutex<Option<PairingStatus>>,
    pub pairing_in_progress: AtomicBool,
    pub pairing_generation: AtomicU64,
    pub sessions: Mutex<HashSet<String>>,
    pub discovered: Mutex<HashMap<String, DiscoveredDevice>>,
    pub status: Mutex<HashMap<String, PeerStatus>>,
    pub backoff: Mutex<HashMap<String, Backoff>>,
    pub dirty: AtomicBool,
    pub last_change: Mutex<Option<ChangeStamp>>,
    pub pending_changes: Mutex<HashSet<String>>,
    pub mdns: tokio::sync::Mutex<Option<ServiceDaemon>>,
    pub listener_port: AtomicU16,
    pub available: AtomicBool,
    pub listener_failed: AtomicBool,
    pub incoming_slots: Arc<Semaphore>,
}

pub(crate) enum PendingPairing {
    Incoming {
        peer_uuid: String,
        peer_name: String,
        spake_msg: Vec<u8>,
        stream: Box<TlsStream<tokio::net::TcpStream>>,
        permit: tokio::sync::OwnedSemaphorePermit,
        created: Instant,
        generation: u64,
    },
    Outgoing {
        generation: u64,
        abort: Option<tokio::task::AbortHandle>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct PairingStatus {
    kind: &'static str,
    phase: &'static str,
    peer_uuid: String,
    peer_name: String,
    code: Option<String>,
    error: Option<String>,
    generation: u64,
}

#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    pub uuid: String,
    pub name: String,
    pub addresses: Vec<String>,
    pub port: u16,
    pub last_seen_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerState {
    Offline,
    Connecting,
    Syncing,
    Synced,
    Error,
}

#[derive(Debug, Clone)]
pub struct PeerStatus {
    pub state: PeerState,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Backoff {
    failures: u32,
    next_attempt: Instant,
}

/// Read committed change capture and local counters, including counter-only
/// dictations. sqlite_sequence survives log compaction, avoiding false edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChangeStamp {
    local: (i64, i64, i64),
    settings: Vec<(String, i64, String)>,
    remote_counters: Vec<(String, i64, i64)>,
}

impl ChangeStamp {
    pub(crate) fn read(conn: &rusqlite::Connection) -> Result<Self> {
        let local = conn.query_row(
            "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='sync_log'),0),
                    COALESCE((SELECT total_words FROM lifetime_stats WHERE id=1),0),
                    COALESCE((SELECT dictionary_fixes FROM lifetime_stats WHERE id=1),0)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        // Imported settings and counter-only gossip do not create sync_log
        // rows. Observe their committed metadata too, so a hub promptly relays
        // them to its other peers rather than waiting for the idle interval.
        let mut settings = sync_store::list_setting_stamps(conn)?.into_iter()
            .map(|(key, stamp)| (key, stamp.ts_ms, stamp.origin)).collect::<Vec<_>>();
        let mut remote_counters = sync_store::list_remote_stats(conn)?.into_iter()
            .map(|stats| (stats.device_id, stats.total_words, stats.dictionary_fixes)).collect::<Vec<_>>();
        settings.sort_unstable();
        remote_counters.sort_unstable();
        Ok(Self { local, settings, remote_counters })
    }
}

pub(crate) fn automatic_session_due(
    local_uuid: &str,
    peer_uuid: &str,
    changed: bool,
    active: bool,
    retry: Option<(u32, Instant)>,
    now: Instant,
) -> bool {
    !active
        && (changed || should_auto_initiate(local_uuid, peer_uuid))
        && retry.is_none_or(|(failures, next)| now >= next || (changed && failures == 0))
}

/// Explicit routes are restricted to Tailscale IPv4 addresses. The paired
/// certificate still authenticates the device, independently of its address.
pub(crate) fn parse_tailscale_address(value: &str) -> Result<SocketAddr> {
    let address: SocketAddr = value.trim().parse().map_err(|_| {
        anyhow!("Enter a Tailscale IPv4 address and sync port, such as 100.64.0.1:50000")
    })?;
    if address.port() == 0 || !matches!(address.ip(), IpAddr::V4(ip) if is_tailscale_address(ip)) {
        return Err(anyhow!(
            "Use the paired device's Tailscale IPv4 address and a nonzero sync port"
        ));
    }
    Ok(address)
}

pub(crate) fn parse_connection_details(value: &str) -> Result<(String, SocketAddr)> {
    let (uuid, address) = value.trim().strip_prefix("verenu-sync://")
        .and_then(|value| value.split_once('@'))
        .ok_or_else(|| anyhow!("Paste the connection details copied from Settings > Sync on the other device"))?;
    let uuid = uuid::Uuid::parse_str(uuid)
        .map_err(|_| anyhow!("These connection details have an invalid device ID. Copy them again from the other device"))?;
    Ok((uuid.to_string(), parse_tailscale_address(address)?))
}

#[derive(Debug)]
struct ConnectionFailure(String);

impl std::fmt::Display for ConnectionFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ConnectionFailure {}

fn connection_error(name: &str, address: SocketAddr, detail: &str) -> anyhow::Error {
    ConnectionFailure(format!("Could not connect to {name} at {address}: {detail}. A firewall or network rule may be blocking TCP port {}. Open Verenu on the other device and allow this connection through both firewalls. For Tailscale, also check that both devices are connected and tailnet access rules allow this port.", address.port())).into()
}

pub(crate) fn should_try_next_address(error: &anyhow::Error) -> bool {
    error.is::<ConnectionFailure>()
}

// ---- DTOs surfaced to the frontend ----

#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceInfoDto {
    pub uuid: String,
    pub name: String,
    pub port: u16,
    pub tailscale_ips: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredDto {
    pub uuid: String,
    pub name: String,
    pub addresses: Vec<String>,
    pub port: u16,
    pub paired: bool,
    pub last_seen_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PeerDto {
    pub uuid: String,
    pub name: String,
    pub added_at: Option<String>,
    pub last_sync_at: Option<String>,
    pub state: String,
    pub error: Option<String>,
    pub online: bool,
    pub connection_address: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PairingStateDto {
    pub kind: String,
    pub phase: String,
    pub peer_uuid: String,
    pub peer_name: String,
    pub code: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncStatusSnapshot {
    pub this_device: DeviceInfoDto,
    pub listener_active: bool,
    pub listener_failed: bool,
    pub pairing: Option<PairingStateDto>,
    pub discovered: Vec<DiscoveredDto>,
    pub peers: Vec<PeerDto>,
}

impl SyncManager {
    /// Initializes the identity and starts all background tasks. Sync stays
    /// soft-failed if identity/listener setup fails - the rest of the app must
    /// never refuse to start over LAN sync.
    pub fn start(app: AppHandle, db: DbHandle) -> SyncManager {
        let data_dir = crate::app_setup::app_data_dir();
        let inner = Arc::new(Inner {
            db,
            app: app.clone(),
            data_dir: data_dir.clone(),
            identity: RwLock::new(None),
            pending: tokio::sync::Mutex::new(None),
            pairing_status: Mutex::new(None),
            pairing_in_progress: AtomicBool::new(false),
            pairing_generation: AtomicU64::new(0),
            sessions: Mutex::new(HashSet::new()),
            discovered: Mutex::new(HashMap::new()),
            status: Mutex::new(HashMap::new()),
            backoff: Mutex::new(HashMap::new()),
            dirty: AtomicBool::new(false),
            last_change: Mutex::new(None),
            pending_changes: Mutex::new(HashSet::new()),
            mdns: tokio::sync::Mutex::new(None),
            listener_port: AtomicU16::new(0),
            available: AtomicBool::new(false),
            listener_failed: AtomicBool::new(false),
            incoming_slots: Arc::new(Semaphore::new(MAX_INCOMING_CONNECTIONS)),
        });
        let manager = SyncManager { inner };

        let init = manager.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(err) = init.initialize().await {
                init.inner.listener_failed.store(true, Ordering::Relaxed);
                log::error!("sync: disabled - {err:#}");
            }
        });
        let _ = app;
        manager
    }

    async fn initialize(&self) -> Result<()> {
        let known_uuid = {
            let conn = self.lock_db()?;
            sync_store::self_uuid(&conn)?
        };
        let identity = identity::load_or_create(&self.inner.data_dir, known_uuid)
            .map_err(|e| anyhow!("identity setup failed ({e}); check the OS credential store"))?;
        let identity = Arc::new(identity);

        // Prefer a user-set name persisted in the DB over the hostname default.
        let name = {
            let conn = self.lock_db()?;
            let stored = stored_device_name(&conn)?;
            let name = match stored {
                Some(name) if !name.trim().is_empty() => name,
                _ => identity.name.clone(),
            };
            sync_store::ensure_self_identity(&conn, &identity.uuid, &name)?;
            name
        };
        {
            let mut guard = self.inner.identity.write().expect("identity lock");
            let mut identity = (*identity).clone();
            identity.name = name;
            *guard = Some(Arc::new(identity));
        }

        // TLS configs (client configs are built per connection; the identity
        // can be rotated by re-initialization).
        let (uuid, cert, key) = {
            let guard = self.inner.identity.read().expect("identity lock");
            let identity = guard.as_ref().ok_or_else(|| anyhow!("identity missing"))?;
            (
                identity.uuid.clone(),
                identity.cert_der().clone(),
                identity.tls_key(),
            )
        };
        let server_cfg = transport::server_config(cert, key)?;

        // Reuse a deterministic port so an mDNS record cached across an app
        // restart still points at the live listener. Fall back only for a real
        // local collision, such as two Verenu identities running together.
        let preferred_port = listener_port_for_uuid(&uuid);
        let listener = match TcpListener::bind(("0.0.0.0", preferred_port)).await {
            Ok(listener) => listener,
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => {
                log::warn!(
                    "sync: preferred listener port {preferred_port} is occupied; using an ephemeral port"
                );
                TcpListener::bind("0.0.0.0:0")
                    .await
                    .map_err(|fallback| anyhow!("could not bind sync listener: {fallback}"))?
            }
            Err(err) => return Err(anyhow!("could not bind sync listener: {err}")),
        };
        let port = listener
            .local_addr()
            .map_err(|e| anyhow!("sync listener has no address: {e}"))?
            .port();
        self.inner.listener_port.store(port, Ordering::Relaxed);
        self.inner.available.store(true, Ordering::Relaxed);
        log::info!("sync: listening on port {port}");

        let accept_inner = self.inner.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((tcp, _)) => {
                        let permit = match accept_inner.incoming_slots.clone().try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                log::debug!("sync: incoming connection limit reached");
                                continue;
                            }
                        };
                        let inner = accept_inner.clone();
                        let cfg = server_cfg.clone();
                        tauri::async_runtime::spawn(async move {
                            let acceptor = tokio_rustls::TlsAcceptor::from(cfg);
                            match transport::accept_with_timeout(
                                &acceptor,
                                tcp,
                                TLS_HANDSHAKE_TIMEOUT,
                            )
                            .await
                            {
                                Ok(tls) => {
                                    handle_connection(inner, tls, permit).await;
                                }
                                Err(err) => {
                                    log::debug!("sync: tls accept failed: {err}");
                                }
                            }
                        });
                    }
                    Err(err) => {
                        log::error!("sync: listener accept failed: {err}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        });

        // One-time repair for exe targets written before cross-platform
        // resolution existed (or before it was applied to that row): a target
        // that doesn't look like this platform's own naming convention and
        // was never tagged gets a chance to resolve against apps installed
        // right now. Best-effort — sync must start regardless.
        if let Err(err) = self.reconcile_stale_context_targets() {
            log::warn!("sync: stale context target reconciliation failed: {err:#}");
        }

        // Discovery.
        let initial_change = {
            let conn = self.lock_db()?;
            ChangeStamp::read(&conn)?
        };
        *self.inner.last_change.lock().expect("change stamp lock") = Some(initial_change);
        self.start_discovery().await?;

        // Change-driven + fallback sync scheduling.
        let monitor = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(CHANGE_CHECK_INTERVAL).await;
                monitor.monitor_tick().await;
            }
        });

        let _ = self.inner.app.emit("verenu:sync-devices-changed", ());
        Ok(())
    }

    async fn start_discovery(&self) -> Result<()> {
        if let Some(previous) = self.inner.mdns.lock().await.take() {
            let _ = previous.shutdown();
        }
        let daemon =
            ServiceDaemon::new().map_err(|e| anyhow!("mDNS daemon failed to start: {e}"))?;
        daemon
            .disable_interface(IfKind::Predicate(IfPredicate::new(|interface| {
                !is_discovery_interface_allowed(&interface.name, interface.ip(), interface.is_p2p())
            })))
            .map_err(|e| anyhow!("mDNS interface filtering failed: {e}"))?;
        let (uuid, name, port) = {
            let guard = self.inner.identity.read().expect("identity lock");
            let identity = guard.as_ref().ok_or_else(|| anyhow!("identity missing"))?;
            (
                identity.uuid.clone(),
                identity.name.clone(),
                self.inner.listener_port.load(Ordering::Relaxed),
            )
        };
        let instance = format!("verenu-{uuid}");
        // Namespace the DNS host separately from the service instance. Older
        // builds published the UUID host as 127.0.0.1 on macOS; using a fresh,
        // app-specific host prevents that stale A record from hiding the LAN IP.
        let host = format!("verenu-{uuid}.local.");
        let props: HashMap<String, String> = [
            ("uuid", uuid.clone()),
            ("name", name),
            ("ver", PROTOCOL_VERSION.to_string()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        // mdns-sd's automatic address selection can classify active macOS
        // interfaces as Unknown and publish only 127.0.0.1. Enumerate usable
        // IPv4 interfaces ourselves so peers receive a reachable LAN address.
        let mut lan_addresses: Vec<std::net::IpAddr> = if_addrs::get_if_addrs()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|interface| {
                let address = interface.ip();
                (is_discovery_interface_allowed(&interface.name, address, interface.is_p2p())
                    && is_discovery_advertised_address_allowed(address))
                .then_some(address)
            })
            .collect();
        // Some directly launched macOS app binaries see only loopback through
        // getifaddrs. A UDP route lookup does not send traffic and reliably
        // reveals the primary interface address in that environment.
        if lan_addresses.is_empty() {
            if let Ok(route_probe) = std::net::UdpSocket::bind("0.0.0.0:0") {
                if route_probe.connect("192.0.2.1:9").is_ok() {
                    if let Ok(local) = route_probe.local_addr() {
                        let address = local.ip();
                        if is_discovery_advertised_address_allowed(address) {
                            lan_addresses.push(address);
                        }
                    }
                }
            }
        }
        lan_addresses.sort_unstable();
        lan_addresses.dedup();
        if lan_addresses.is_empty() {
            log::warn!(
                "sync: no usable LAN address found; keeping discovery alive without advertising"
            );
        } else {
            log::info!("sync: advertising {host} on {lan_addresses:?}");
            let service = ServiceInfo::new(
                SERVICE_TYPE,
                &instance,
                &host,
                lan_addresses.as_slice(),
                port,
                Some(props),
            )
            .map_err(|e| anyhow!("mDNS service info invalid: {e}"))?;
            daemon
                .register(service)
                .map_err(|e| anyhow!("mDNS registration failed: {e}"))?;
        }

        let receiver = daemon
            .browse(SERVICE_TYPE)
            .map_err(|e| anyhow!("mDNS browse failed: {e}"))?;
        *self.inner.mdns.lock().await = Some(daemon);

        let inner = self.inner.clone();
        tauri::async_runtime::spawn(async move {
            while let Ok(event) = receiver.recv_async().await {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        handle_resolved(&inner, *info);
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        let instance_name = fullname.split('.').next().unwrap_or("");
                        let instance = instance_name
                            .strip_prefix("verenu-")
                            .unwrap_or(instance_name);
                        let changed = inner
                            .discovered
                            .lock()
                            .map(|mut d| d.remove(instance).is_some())
                            .unwrap_or(false);
                        if changed {
                            inner.emit_devices_changed();
                        }
                    }
                    _ => {}
                }
            }
        });
        Ok(())
    }

    fn lock_db(&self) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>> {
        self.inner
            .db
            .lock()
            .map_err(|_| anyhow!("database lock was poisoned"))
    }

    /// Repairs `context_targets` rows that predate cross-platform resolution
    /// (or were synced in before this device ever ran it): an untagged row
    /// whose executable doesn't look like this platform's own naming
    /// convention gets resolved against apps installed right now, mirroring
    /// what `resolve_context_targets_in_ops` already does for every live
    /// incoming sync op. A match is tagged with this platform so it now
    /// displays only here; no match gets the same "?::" unresolved marker the
    /// live path uses, so the UI shows a clear "(not found)" chip instead of
    /// a bare foreign string a user can't act on. Best-effort: errors are
    /// logged by the caller, never fatal to sync startup.
    fn reconcile_stale_context_targets(&self) -> Result<()> {
        let conn = self.lock_db()?;
        let rows: Vec<(i64, String)> = {
            let mut stmt =
                conn.prepare("SELECT id, executable FROM context_targets WHERE platform IS NULL")?;
            let mapped = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            mapped
        };
        if rows.is_empty() {
            return Ok(());
        }

        // Linux desktop ids and window classes carry no suffix, so every
        // untagged row there goes through matching below.
        let native_suffix = if cfg!(target_os = "macos") {
            Some(".app")
        } else if cfg!(target_os = "linux") {
            None
        } else {
            Some(".exe")
        };
        let installed_apps = crate::system::apps::list_installed_apps();
        let platform_tag = crate::data::db::current_platform_tag();

        for (id, executable) in rows {
            if executable.starts_with("?::")
                || native_suffix.is_some_and(|suffix| executable.to_lowercase().ends_with(suffix))
            {
                continue;
            }
            let mut resolved = closest_installed_app(&executable, &installed_apps)
                .map(|app| app.exe.clone())
                .unwrap_or_else(|| engine::unresolved_app_target(&executable));
            if resolved.starts_with("?::") {
                let existing_id: Option<i64> = conn
                    .query_row(
                        "SELECT id FROM context_targets WHERE executable = ?1",
                        rusqlite::params![resolved],
                        |row| row.get(0),
                    )
                    .optional()?;
                if existing_id.is_some_and(|existing_id| existing_id != id) {
                    resolved = format!("{resolved}#{id}");
                }
            }
            let new_platform = (!resolved.starts_with("?::"))
                .then_some(platform_tag)
                .flatten();
            let updated = conn.execute(
                "UPDATE context_targets SET executable = ?1, platform = ?2 WHERE id = ?3",
                rusqlite::params![resolved, new_platform, id],
            );
            match updated {
                Ok(_) => {}
                // The resolved executable already belongs to another row for
                // this context (both devices' targets turned out to be the
                // same app) — drop the now-redundant stale row instead of
                // erroring the whole pass.
                Err(rusqlite::Error::SqliteFailure(err, _))
                    if err.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    conn.execute(
                        "DELETE FROM context_targets WHERE id = ?1",
                        rusqlite::params![id],
                    )?;
                }
                Err(err) => return Err(err.into()),
            }
        }
        Ok(())
    }

    pub fn identity_exchange(&self) -> Result<IdentityExchange> {
        let guard = self.inner.identity.read().expect("identity lock");
        let identity = guard
            .as_ref()
            .ok_or_else(|| anyhow!("sync is unavailable"))?;
        Ok(IdentityExchange {
            device_uuid: identity.uuid.clone(),
            device_name: identity.name.clone(),
            cert_der: identity.cert_der().as_ref().to_vec(),
        })
    }

    fn device_uuid(&self) -> String {
        self.inner
            .identity
            .read()
            .expect("identity lock")
            .as_ref()
            .map(|identity| identity.uuid.clone())
            .unwrap_or_default()
    }

    pub fn device_info(&self) -> DeviceInfoDto {
        let guard = self.inner.identity.read().expect("identity lock");
        match guard.as_ref() {
            Some(identity) => DeviceInfoDto {
                uuid: identity.uuid.clone(),
                name: identity.name.clone(),
                port: self.inner.listener_port.load(Ordering::Relaxed),
                tailscale_ips: if_addrs::get_if_addrs().unwrap_or_default().into_iter()
                    .filter_map(|interface| match interface.ip() {
                        IpAddr::V4(ip) if is_tailscale_address(ip) => Some(ip.to_string()),
                        _ => None,
                    }).collect(),
            },
            None => DeviceInfoDto {
                uuid: String::new(),
                name: String::new(),
                port: 0,
                tailscale_ips: Vec::new(),
            },
        }
    }

    pub fn snapshot(&self) -> SyncStatusSnapshot {
        let this_device = self.device_info();
        let peers = conn_peers(&self.inner.db);
        let paired: HashSet<&str> = peers.iter().map(|peer| peer.device_uuid.as_str()).collect();
        let discovered = self
            .inner
            .discovered
            .lock()
            .map(|map| {
                let mut list: Vec<DiscoveredDto> = map
                    .values()
                    .map(|d| DiscoveredDto {
                        uuid: d.uuid.clone(),
                        name: d.name.clone(),
                        addresses: d.addresses.clone(),
                        port: d.port,
                        paired: paired.contains(d.uuid.as_str()),
                        last_seen_ms: d.last_seen_ms,
                    })
                    .collect();
                list.sort_by_key(|a| a.name.to_lowercase());
                list
            })
            .unwrap_or_default();
        let status = self.inner.status.lock().ok();
        let peers = peers
            .into_iter()
            .map(|peer| {
                let online = discovered.iter().any(|d| d.uuid == peer.device_uuid);
                let state = status
                    .as_ref()
                    .and_then(|s| s.get(&peer.device_uuid))
                    .cloned()
                    .unwrap_or(PeerStatus {
                        state: PeerState::Offline,
                        error: peer.last_error.clone(),
                    });
                PeerDto {
                    connection_address: self
                        .saved_peer_address(&peer.device_uuid)
                        .map(|address| address.to_string()),
                    uuid: peer.device_uuid.clone(),
                    name: if peer.name.is_empty() {
                        discovered
                            .iter()
                            .find(|d| d.uuid == peer.device_uuid)
                            .map(|d| d.name.clone())
                            .unwrap_or_else(|| "Paired device".to_string())
                    } else {
                        peer.name
                    },
                    added_at: Some(peer.added_at),
                    last_sync_at: peer.last_sync_at,
                    state: state_string(state.state),
                    error: state.error.or(peer.last_error),
                    online,
                }
            })
            .collect();
        let pairing = self
            .inner
            .pairing_status
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().cloned())
            .map(|pairing| PairingStateDto {
                kind: pairing.kind.to_string(),
                phase: pairing.phase.to_string(),
                peer_uuid: pairing.peer_uuid,
                peer_name: pairing.peer_name,
                code: pairing.code,
                error: pairing.error,
            });
        SyncStatusSnapshot {
            this_device,
            listener_active: self.inner.available.load(Ordering::Relaxed),
            listener_failed: self.inner.listener_failed.load(Ordering::Relaxed),
            pairing,
            discovered,
            peers,
        }
    }

    /// Initiates pairing with a discovered device. Returns the code to show.
    pub async fn start_pairing(&self, peer_uuid: String) -> Result<String> {
        let target = self
            .inner
            .discovered
            .lock()
            .map_err(|_| anyhow!("discovery lock poisoned"))?
            .get(&peer_uuid)
            .cloned()
            .ok_or_else(|| anyhow!("that device is no longer visible on the network"))?;
        self.start_pairing_target(target, None).await
    }

    pub async fn pair_connection(&self, details: &str) -> Result<String> {
        let (uuid, address) = parse_connection_details(details)?;
        if uuid == self.device_uuid() {
            return Err(anyhow!("These are this device's details. Copy the details from your other device"));
        }
        let paired = {
            let conn = self.lock_db()?;
            sync_store::get_peer(&conn, &uuid)?.is_some()
        };
        if paired {
            return Err(anyhow!("This device is already paired. Use its Connection button to update the address"));
        }
        let target = DiscoveredDevice {
            uuid, name: "Tailscale device".into(), addresses: vec![address.ip().to_string()],
            port: address.port(), last_seen_ms: now_ms_u64(),
        };
        self.start_pairing_target(target, Some(address)).await
    }

    async fn start_pairing_target(&self, target: DiscoveredDevice, route: Option<SocketAddr>) -> Result<String> {
        let peer_uuid = target.uuid.clone();
        let generation;
        {
            let mut pending = self.inner.pending.lock().await;
            if pending.is_some() {
                return Err(anyhow!("a pairing is already in progress"));
            }
            generation = self
                .inner
                .pairing_generation
                .fetch_add(1, Ordering::Relaxed)
                + 1;
            *pending = Some(PendingPairing::Outgoing {
                generation,
                abort: None,
            });
        }
        let code = pairing::generate_pairing_code();
        self.inner.set_pairing_status(PairingStatus {
            kind: "outgoing",
            phase: "connecting",
            peer_uuid: peer_uuid.clone(),
            peer_name: target.name.clone(),
            code: Some(code.clone()),
            error: None,
            generation,
        });

        let manager = self.clone();
        let task_code = code.clone();
        let task = tokio::spawn(async move {
            let result = manager
                .run_outgoing_pairing(&target, &task_code, generation, route)
                .await;
            let result = result.and_then(|()| match route {
                Some(address) => manager.set_peer_address(&target.uuid, &address.to_string()),
                None => Ok(()),
            });
            if let Err(err) = result {
                log::warn!("sync: pairing with {} failed: {err:#}", target.name);
                manager.inner.fail_pairing(generation, format!("{err:#}"));
                manager
                    .inner
                    .app
                    .emit(
                        "verenu:sync-pair-result",
                        serde_json::json!({
                            "uuid": target.uuid,
                            "ok": false,
                            "message": format!("{err:#}"),
                        }),
                    )
                    .ok();
            }
            manager.clear_pending_if_generation(generation).await;
            manager.inner.emit_devices_changed();
        });
        let abort = task.abort_handle();
        {
            let mut pending = self.inner.pending.lock().await;
            if let Some(PendingPairing::Outgoing {
                abort: slot,
                generation: g,
                ..
            }) = pending.as_mut()
            {
                if *g == generation {
                    *slot = Some(abort);
                }
            }
        }
        Ok(code)
    }

    async fn run_outgoing_pairing(
        &self,
        target: &DiscoveredDevice,
        code: &str,
        generation: u64,
        route: Option<SocketAddr>,
    ) -> Result<()> {
        let identity = self.identity_exchange()?;
        if target.addresses.is_empty() {
            return Err(anyhow!("device has no reachable address"));
        }
        let client_cfg = {
            let guard = self.inner.identity.read().expect("identity lock");
            let identity = guard.as_ref().ok_or_else(|| anyhow!("sync unavailable"))?;
            transport::client_config(identity.cert_der().clone(), identity.tls_key())?
        };
        let connector = transport::tls_connector(client_cfg);
        let mut tcp = None;
        let mut failures = Vec::new();
        let candidates = route.map(|address| vec![address]).unwrap_or_else(||
            connection_candidates(&target.addresses, target.port, &target.uuid));
        for addr in candidates {
            match tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::TcpStream::connect(addr)).await
            {
                Ok(Ok(stream)) => {
                    tcp = Some(stream);
                    break;
                }
                Ok(Err(err)) => failures.push(connection_error(&target.name, addr, &err.to_string()).to_string()),
                Err(_) => failures.push(connection_error(&target.name, addr, "timed out").to_string()),
            }
        }
        let tcp = tcp.ok_or_else(|| {
            anyhow!(
                "could not reach {} on any discovered address: {}",
                target.name,
                failures.join("; ")
            )
        })?;
        let mut tls = tokio::time::timeout(
            CONNECT_TIMEOUT,
            connector.connect(transport::server_name_for(&target.uuid), tcp),
        )
        .await
        .map_err(|_| anyhow!("timed out during TLS handshake"))?
        .map_err(|e| anyhow!("TLS handshake with {} failed: {e}", target.name))?;

        let (spake_state, spake_msg) = pairing::initiator_start(code);
        send_message(
            &mut tls,
            &Message::PairRequest {
                device_uuid: identity.device_uuid.clone(),
                device_name: identity.device_name.clone(),
                protocol: PROTOCOL_VERSION,
                spake_msg,
            },
        )
        .await?;
        self.inner
            .update_pairing_phase(generation, "waiting_for_code");

        // The responder replies only after its user approves, so the whole
        // exchange runs under the generous pairing timeout.
        let exchange = async {
            let responder_msg = match super::protocol::read_message_with_timeout(
                &mut tls,
                PAIRING_TIMEOUT,
            )
            .await?
            {
                Message::PairAccept { spake_msg } => spake_msg,
                Message::PairReject { reason } => return Err(anyhow!("rejected: {reason}")),
                Message::PairBusy => {
                    return Err(anyhow!(
                        "{} is already handling another pairing",
                        target.name
                    ))
                }
                Message::Error { message } => return Err(anyhow!("{message}")),
                other => return Err(anyhow!("unexpected pairing response: {other:?}")),
            };
            let cipher = pairing::initiator_cipher(spake_state, &responder_msg)?;
            pairing::initiator_exchange(&mut tls, &cipher, &identity, &target.uuid).await
        };
        let outcome = tokio::time::timeout(PAIRING_TIMEOUT, exchange)
            .await
            .map_err(|_| anyhow!("{} didn't complete the pairing in time", target.name))??;
        self.complete_pairing(outcome, generation).await
    }

    /// Responder side: user approved (or rejected) with the typed code.
    pub async fn respond_to_pairing(&self, code: String, approve: bool) -> Result<()> {
        let pending = {
            let mut guard = self.inner.pending.lock().await;
            let pending = match guard.take() {
                Some(PendingPairing::Incoming {
                    peer_uuid,
                    peer_name,
                    spake_msg,
                    stream,
                    permit,
                    created,
                    generation,
                }) => {
                    let mut stream = stream;
                    if !approve {
                        let _ = send_message(
                            &mut stream,
                            &Message::PairReject {
                                reason: "declined".to_string(),
                            },
                        )
                        .await;
                        self.inner
                            .app
                            .emit(
                                "verenu:sync-pair-result",
                                serde_json::json!({ "uuid": peer_uuid, "ok": false, "message": "Pairing declined" }),
                            )
                            .ok();
                        self.inner.clear_pairing_status(generation);
                        return Ok(());
                    }
                    (
                        peer_uuid, peer_name, spake_msg, stream, created, generation, permit,
                    )
                }
                Some(PendingPairing::Outgoing { .. }) | None => {
                    return Err(anyhow!("no incoming pairing request to respond to"));
                }
            };
            self.inner
                .pairing_in_progress
                .store(true, Ordering::Release);
            pending
        };
        let _pairing_guard = PairingInProgressGuard(&self.inner.pairing_in_progress);
        let (peer_uuid, peer_name, spake_msg, mut stream, created, generation, permit) = pending;
        self.inner.update_pairing_phase(generation, "verifying");
        let identity = match self.identity_exchange() {
            Ok(identity) => identity,
            Err(err) => {
                self.restore_incoming_pairing(PendingPairing::Incoming {
                    peer_uuid,
                    peer_name,
                    spake_msg,
                    stream,
                    permit,
                    created,
                    generation,
                })
                .await;
                return Err(err);
            }
        };
        let (responder_msg, cipher) = match pairing::responder_start(&code, &spake_msg) {
            Ok(result) => result,
            Err(err) => {
                // A mistyped code is retryable; the SPAKE exchange has not
                // touched the stream yet, so keep the incoming request alive.
                self.restore_incoming_pairing(PendingPairing::Incoming {
                    peer_uuid,
                    peer_name,
                    spake_msg,
                    stream,
                    permit,
                    created,
                    generation,
                })
                .await;
                return Err(err);
            }
        };
        let result = match tokio::time::timeout(
            PAIRING_TIMEOUT,
            pairing::responder_exchange(&mut stream, &cipher, responder_msg, &identity, &peer_uuid),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(anyhow!("pairing timed out")),
        };
        match result {
            Ok(outcome) => {
                if let Err(err) = self.complete_pairing(outcome, generation).await {
                    self.inner.fail_pairing(generation, format!("{err:#}"));
                    let _ = send_message(
                        &mut stream,
                        &Message::Error {
                            message: "Could not save paired device trust".to_string(),
                        },
                    )
                    .await;
                    return Err(err);
                }
                send_message(&mut stream, &Message::PairComplete).await?;
                self.inner
                    .app
                    .emit(
                        "verenu:sync-pair-result",
                        serde_json::json!({ "uuid": peer_uuid, "ok": true, "message": format!("Paired with {peer_name}") }),
                    )
                    .ok();
            }
            Err(err) => {
                self.inner.fail_pairing(generation, format!("{err:#}"));
                self.inner
                    .app
                    .emit(
                        "verenu:sync-pair-result",
                        serde_json::json!({ "uuid": peer_uuid, "ok": false, "message": format!("{err:#}") }),
                    )
                    .ok();
                return Err(err);
            }
        }
        Ok(())
    }

    async fn restore_incoming_pairing(&self, pending_pairing: PendingPairing) {
        let PendingPairing::Incoming {
            peer_uuid,
            peer_name,
            spake_msg,
            stream,
            permit,
            created,
            generation,
        } = pending_pairing
        else {
            return;
        };
        let mut pending = self.inner.pending.lock().await;
        if pending.is_none() && self.inner.pairing_generation.load(Ordering::Relaxed) == generation
        {
            *pending = Some(PendingPairing::Incoming {
                peer_uuid: peer_uuid.clone(),
                peer_name: peer_name.clone(),
                spake_msg,
                stream,
                permit,
                created,
                generation,
            });
            self.inner.set_pairing_status(PairingStatus {
                kind: "incoming",
                phase: "awaiting_code",
                peer_uuid: peer_uuid.clone(),
                peer_name: peer_name.clone(),
                code: None,
                error: None,
                generation,
            });
            self.inner.emit_devices_changed();
        }
    }

    async fn complete_pairing(&self, outcome: IdentityExchange, generation: u64) -> Result<()> {
        if outcome.device_uuid.is_empty() || outcome.cert_der.is_empty() {
            return Err(anyhow!("peer sent an incomplete identity"));
        }
        // Serialize the generation check with cancellation/new pairing so a
        // cancelled exchange cannot persist a peer after the user moved on.
        let _pairing_generation_guard = self.inner.pending.lock().await;
        if self.inner.pairing_generation.load(Ordering::Relaxed) != generation {
            return Err(anyhow!("pairing was cancelled"));
        }
        let fp = identity::fingerprint_of(&outcome.cert_der);
        {
            let conn = self.lock_db()?;
            let tx = conn.unchecked_transaction()?;
            sync_store::upsert_peer(&tx, &outcome.device_uuid, &outcome.device_name, &fp)?;
            // Seed setting stamps at pairing time so the first session compares
            // real timestamps instead of treating existing values as ancient.
            let keys: Vec<String> = engine::SYNCABLE_SETTINGS
                .iter()
                .map(|s| s.to_string())
                .collect();
            sync_store::seed_setting_stamps(&tx, &self.device_uuid(), &keys)?;
            tx.commit()?;
        }
        drop(_pairing_generation_guard);
        self.clear_pending_if_generation(generation).await;
        self.inner.clear_pairing_status(generation);
        self.inner
            .app
            .emit(
                "verenu:sync-pair-result",
                serde_json::json!({
                    "uuid": outcome.device_uuid,
                    "ok": true,
                    "message": format!("Paired with {}", outcome.device_name),
                }),
            )
            .ok();
        self.inner.emit_devices_changed();
        // Pull right away, from one deterministic side only. Both peers finish
        // pairing at nearly the same time and used to open competing sessions.
        let uuid = outcome.device_uuid.clone();
        if should_auto_initiate(&self.device_uuid(), &uuid) {
            let manager = self.clone();
            tauri::async_runtime::spawn(async move {
                let _ = manager.sync_to_peer(&uuid).await;
            });
        }
        Ok(())
    }

    pub async fn cancel_pairing(&self) -> Result<()> {
        let mut guard = self.inner.pending.lock().await;
        self.inner
            .pairing_generation
            .fetch_add(1, Ordering::Relaxed);
        if let Some(PendingPairing::Outgoing {
            abort: Some(abort_handle),
            ..
        }) = guard.take()
        {
            abort_handle.abort();
        }
        if let Ok(mut pairing) = self.inner.pairing_status.lock() {
            *pairing = None;
        }
        self.inner.emit_devices_changed();
        Ok(())
    }

    async fn clear_pending_if_generation(&self, generation: u64) {
        let mut guard = self.inner.pending.lock().await;
        let stored_generation = match guard.as_ref() {
            Some(PendingPairing::Outgoing { generation: g, .. }) => Some(*g),
            Some(PendingPairing::Incoming { generation: g, .. }) => Some(*g),
            None => None,
        };
        if stored_generation == Some(generation) {
            *guard = None;
        }
    }

    /// Removes a paired device locally and best-effort notifies the peer.
    pub async fn remove_device(&self, peer_uuid: String) -> Result<()> {
        let existed = {
            let conn = self.lock_db()?;
            let existed = sync_store::remove_peer(&conn, &peer_uuid)?;
            // Their contribution to merged lifetime counters goes too.
            let _ = sync_store::remove_remote_stats(&conn, &peer_uuid);
            existed
        };
        if !existed {
            return Err(anyhow!("that device is not paired"));
        }
        {
            let mut status = self
                .inner
                .status
                .lock()
                .map_err(|_| anyhow!("status lock poisoned"))?;
            status.remove(&peer_uuid);
        }
        // Best-effort unpair notification so the peer forgets us too.
        let manager = self.clone();
        let uuid = peer_uuid.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(err) = manager.send_unpair(&uuid).await {
                log::info!("sync: unpair notification to {uuid} failed (harmless): {err:#}");
            }
        });
        self.inner.emit_devices_changed();
        Ok(())
    }

    async fn send_unpair(&self, peer_uuid: &str) -> Result<()> {
        let addr = self
            .addr_for_peer(peer_uuid)?
            .ok_or_else(|| anyhow!("device not visible"))?;
        let (cert, key) = {
            let guard = self.inner.identity.read().expect("identity lock");
            let identity = guard.as_ref().ok_or_else(|| anyhow!("sync unavailable"))?;
            (identity.cert_der().clone(), identity.tls_key())
        };
        let connector = transport::tls_connector(transport::client_config(cert, key)?);
        let tcp = tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::TcpStream::connect(addr))
            .await
            .map_err(|_| anyhow!("timeout"))??;
        let mut tls = tokio::time::timeout(
            CONNECT_TIMEOUT,
            connector.connect(transport::server_name_for(peer_uuid), tcp),
        )
        .await
        .map_err(|_| anyhow!("tls timeout"))??;
        let my_uuid = self.device_uuid();
        send_message(
            &mut tls,
            &Message::Unpair {
                device_uuid: my_uuid,
            },
        )
        .await?;
        Ok(())
    }

    fn addr_for_peer(&self, peer_uuid: &str) -> Result<Option<SocketAddr>> {
        Ok(self.addr_candidates_for_peer(peer_uuid)?.into_iter().next())
    }

    fn addr_candidates_for_peer(&self, peer_uuid: &str) -> Result<Vec<SocketAddr>> {
        let discovered = self
            .inner
            .discovered
            .lock()
            .map_err(|_| anyhow!("discovery lock poisoned"))?;
        let mut addresses = discovered
            .get(peer_uuid)
            .map(|d| connection_candidates(&d.addresses, d.port, peer_uuid))
            .unwrap_or_default();
        if let Some(address) = self.saved_peer_address(peer_uuid) {
            // Prefer the persistent route; mDNS is unavailable across networks.
            addresses.insert(0, address);
            addresses.dedup();
        }
        Ok(addresses)
    }

    fn saved_peer_address(&self, peer_uuid: &str) -> Option<SocketAddr> {
        store::settings_handle(&self.inner.app)
            .ok()?
            .get(store::SYNC_PEER_ADDRESSES)?
            .get(peer_uuid)?
            .as_str()
            .and_then(|value| parse_tailscale_address(value).ok())
    }

    pub fn set_peer_address(&self, peer_uuid: &str, address: &str) -> Result<()> {
        let conn = self.lock_db()?;
        if sync_store::get_peer(&conn, peer_uuid)?.is_none() {
            return Err(anyhow!(
                "Pair this device before saving a Tailscale connection"
            ));
        }
        let address = if address.trim().is_empty() {
            None
        } else if address.trim().starts_with("verenu-sync://") {
            let (uuid, address) = parse_connection_details(address)?;
            if uuid != peer_uuid {
                return Err(anyhow!("These details belong to a different device. Copy them from the device named on this connection"));
            }
            Some(address.to_string())
        } else {
            Some(parse_tailscale_address(address)?.to_string())
        };
        let settings = store::settings_handle(&self.inner.app).map_err(|e| anyhow!(e))?;
        let mut routes = settings
            .get(store::SYNC_PEER_ADDRESSES)
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        if let Some(address) = address {
            routes.insert(peer_uuid.to_string(), address.into());
        } else {
            routes.remove(peer_uuid);
        }
        settings
            .save_value(store::SYNC_PEER_ADDRESSES, routes.into())
            .map_err(|e| anyhow!(e))?;
        drop(conn);
        self.inner
            .backoff
            .lock()
            .expect("backoff lock")
            .remove(peer_uuid);
        self.inner
            .pending_changes
            .lock()
            .expect("pending changes lock")
            .insert(peer_uuid.to_string());
        self.inner.emit_devices_changed();
        Ok(())
    }

    /// Manual "Sync now". `None` attempts every paired peer and reports errors.
    pub async fn sync_now(&self, peer_uuid: Option<String>) -> Result<()> {
        let targets = match peer_uuid {
            Some(uuid) => vec![uuid],
            None => {
                let conn = self.lock_db()?;
                sync_store::list_peers(&conn)?
                    .into_iter()
                    .map(|peer| peer.device_uuid)
                    .collect()
            }
        };
        if targets.is_empty() {
            return Err(anyhow!("No paired devices to sync"));
        }
        let mut failures = Vec::new();
        for uuid in targets {
            if let Err(error) = self.sync_to_peer(&uuid).await {
                failures.push(error.to_string());
            }
        }
        if !failures.is_empty() {
            return Err(anyhow!(failures.join("; ")));
        }
        Ok(())
    }

    pub fn set_device_name(&self, name: String) -> Result<()> {
        let name = name.trim().chars().take(60).collect::<String>();
        if name.is_empty() {
            return Err(anyhow!("Device name cannot be empty"));
        }
        {
            let guard = self.inner.identity.read().expect("identity lock");
            let identity = guard.as_ref().ok_or_else(|| anyhow!("sync unavailable"))?;
            let mut updated = (**identity).clone();
            updated.name = name.clone();
            drop(guard);
            let mut guard = self.inner.identity.write().expect("identity lock");
            *guard = Some(Arc::new(updated));
        }
        {
            let conn = self.lock_db()?;
            sync_store::ensure_self_identity(&conn, &self.device_uuid(), &name)?;
        }
        // Re-advertise with the new name (best-effort; the next restart also fixes it).
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(err) = manager.start_discovery().await {
                log::warn!("sync: re-advertise after rename failed: {err:#}");
            }
        });
        self.inner.emit_devices_changed();
        Ok(())
    }

    /// Called by the settings layer after a synced setting changes locally.
    pub fn mark_dirty(&self) {
        self.inner.dirty.store(true, Ordering::Relaxed);
    }

    async fn monitor_tick(&self) {
        let paired = conn_peers(&self.inner.db);
        if paired.is_empty() {
            return;
        }
        let stamp = self.lock_db().and_then(|conn| ChangeStamp::read(&conn));
        let mut dirty = self.inner.dirty.swap(false, Ordering::Relaxed);
        if let Ok(stamp) = stamp {
            let mut previous = self.inner.last_change.lock().expect("change stamp lock");
            dirty |= previous.as_ref().is_some_and(|previous| previous != &stamp);
            *previous = Some(stamp);
        }
        if dirty {
            self.inner
                .pending_changes
                .lock()
                .expect("pending changes lock")
                .extend(paired.iter().map(|peer| peer.device_uuid.clone()));
        }
        let now = Instant::now();
        let device_uuid = self.device_uuid();
        let targets: Vec<String> = {
            let discovered = match self.inner.discovered.lock() {
                Ok(map) => map.values().map(|device| device.uuid.clone()).collect::<HashSet<_>>(),
                Err(_) => return,
            };
            let backoff = match self.inner.backoff.lock() {
                Ok(b) => b,
                Err(_) => return,
            };
            let pending = self
                .inner
                .pending_changes
                .lock()
                .expect("pending changes lock");
            let active = self.inner.sessions.lock().expect("sessions lock");
            paired
                .into_iter()
                .filter(|peer| {
                    discovered.contains(&peer.device_uuid)
                        || self.saved_peer_address(&peer.device_uuid).is_some()
                })
                .filter(|peer| {
                    automatic_session_due(
                        &device_uuid,
                        &peer.device_uuid,
                        pending.contains(&peer.device_uuid),
                        active.contains(&peer.device_uuid),
                        backoff
                            .get(&peer.device_uuid)
                            .map(|entry| (entry.failures, entry.next_attempt)),
                        now,
                    )
                })
                .map(|peer| peer.device_uuid)
                .collect()
        };
        for uuid in targets {
            self.inner
                .pending_changes
                .lock()
                .expect("pending changes lock")
                .remove(&uuid);
            let manager = self.clone();
            tauri::async_runtime::spawn(async move {
                if manager.sync_to_peer(&uuid).await.is_err() {
                    manager
                        .inner
                        .pending_changes
                        .lock()
                        .expect("pending changes lock")
                        .insert(uuid);
                }
            });
        }
    }

    /// Runs one sync session with a paired peer (if discovered and idle).
    pub async fn sync_to_peer(&self, peer_uuid: &str) -> Result<()> {
        if peer_uuid == self.device_uuid() {
            return Err(anyhow!("Cannot sync this device with itself"));
        }
        {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow!("session lock poisoned"))?;
            if !sessions.insert(peer_uuid.to_string()) {
                return Err(anyhow!(
                    "A sync session with this device is already running"
                ));
            }
        }
        let _guard = SessionGuard(self.inner.clone(), peer_uuid.to_string());

        let addrs = self.addr_candidates_for_peer(peer_uuid)?;
        if addrs.is_empty() {
            return Err(anyhow!("Device is offline or not visible on this network"));
        }
        let peer = {
            let conn = self.lock_db()?;
            sync_store::get_peer(&conn, peer_uuid)?.ok_or_else(|| anyhow!("not paired"))
        }?;

        self.set_status(peer_uuid, PeerState::Connecting, None);
        let mut result = Err(anyhow!("no connection candidates"));
        for addr in addrs {
            match self.run_client_session(&peer, addr).await {
                Ok(summary) => {
                    result = Ok(summary);
                    break;
                }
                Err(err) => {
                    // A busy/authenticated peer or failed transfer is the same
                    // peer on every address. Release this slot for its retry;
                    // probing unreachable LAN addresses can otherwise livelock
                    // simultaneous edits on a working Tailscale connection.
                    let try_next = should_try_next_address(&err);
                    result = Err(err);
                    if !try_next { break; }
                }
            }
        }
        match result {
            Ok(summary) => {
                {
                    let conn = self.lock_db()?;
                    let _ = sync_store::mark_peer_synced(&conn, peer_uuid, 0);
                    let _ = sync_store::compact_log(&conn);
                }
                self.reset_backoff(peer_uuid);
                self.set_status(peer_uuid, PeerState::Synced, None);
                if summary.applied.applied > 0 || summary.settings_applied > 0 {
                    self.inner
                        .app
                        .emit(
                            "verenu:sync-data-changed",
                            serde_json::json!({ "tables": summary.applied.touched_tables() }),
                        )
                        .ok();
                }
                Ok(())
            }
            Err(err) => {
                let message = format!("{err:#}");
                log::warn!("sync: session with {peer_uuid} failed: {message}");
                {
                    let conn = self.lock_db()?;
                    let _ = sync_store::mark_peer_error(&conn, peer_uuid, &message);
                }
                self.bump_backoff(peer_uuid);
                self.set_status(peer_uuid, PeerState::Error, Some(&message));
                self.inner
                    .app
                    .emit(
                        "verenu:sync-status",
                        serde_json::json!({
                            "uuid": peer_uuid,
                            "state": "error",
                            "error": message,
                        }),
                    )
                    .ok();
                Err(err)
            }
        }
    }

    async fn run_client_session(
        &self,
        peer: &SyncPeer,
        addr: SocketAddr,
    ) -> Result<engine::SessionSummary> {
        let (cert, key) = {
            let guard = self.inner.identity.read().expect("identity lock");
            let identity = guard.as_ref().ok_or_else(|| anyhow!("sync unavailable"))?;
            (identity.cert_der().clone(), identity.tls_key())
        };
        let connector = transport::tls_connector(transport::client_config(cert, key)?);
        let tcp = tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::TcpStream::connect(addr))
            .await
            .map_err(|_| connection_error(&peer.name, addr, "connection timed out"))?
            .map_err(|e| connection_error(&peer.name, addr, &e.to_string()))?;
        let mut tls = tokio::time::timeout(
            CONNECT_TIMEOUT,
            connector.connect(transport::server_name_for(&peer.device_uuid), tcp),
        )
        .await
        .map_err(|_| anyhow!("TLS handshake with {} timed out", peer.name))?
        .map_err(|e| anyhow!("TLS handshake with {} failed: {e}", peer.name))?;

        // Authenticate: the peer's certificate fingerprint must match the pin
        // from pairing time.
        let presented = transport::peer_fingerprint(
            tls.get_ref()
                .1
                .peer_certificates()
                .ok_or_else(|| anyhow!("peer presented no certificate"))?,
        )?;
        if presented != peer.cert_fp {
            return Err(anyhow!(
                "{}'s identity changed since pairing - remove and re-pair the device",
                peer.name
            ));
        }

        let host = ManagerHost::new(&self.inner);
        self.set_status(&peer.device_uuid, PeerState::Syncing, None);
        // A stalled peer must not hold the session slot forever.
        let summary = tokio::time::timeout(
            Duration::from_secs(600),
            engine::run_session(&self.inner.db, &host, &mut tls, true, peer),
        )
        .await
        .map_err(|_| anyhow!("sync session with {} timed out", peer.name))??;
        Ok(summary)
    }

    fn set_status(&self, peer_uuid: &str, state: PeerState, error: Option<&str>) {
        if let Ok(mut status) = self.inner.status.lock() {
            status.insert(
                peer_uuid.to_string(),
                PeerStatus {
                    state: state.clone(),
                    error: error.map(|e| e.to_string()),
                },
            );
        }
        self.inner
            .app
            .emit(
                "verenu:sync-status",
                serde_json::json!({
                    "uuid": peer_uuid,
                    "state": state_string(state),
                    "error": error,
                }),
            )
            .ok();
    }

    fn reset_backoff(&self, peer_uuid: &str) {
        if let Ok(mut backoff) = self.inner.backoff.lock() {
            backoff.insert(
                peer_uuid.to_string(),
                Backoff {
                    failures: 0,
                    next_attempt: Instant::now() + IDLE_SYNC_INTERVAL,
                },
            );
        }
    }

    fn bump_backoff(&self, peer_uuid: &str) {
        let mut backoff = match self.inner.backoff.lock() {
            Ok(b) => b,
            Err(_) => return,
        };
        let entry = backoff.entry(peer_uuid.to_string()).or_insert(Backoff {
            failures: 0,
            next_attempt: Instant::now(),
        });
        entry.failures = entry.failures.saturating_add(1);
        // Different retry delays resolve simultaneous edits without repeatedly
        // starting competing sessions on both devices.
        let base = if should_auto_initiate(&self.device_uuid(), peer_uuid) {
            2u64
        } else {
            3u64
        };
        let secs = base
            .saturating_mul(1u64 << entry.failures.saturating_sub(1).min(5))
            .min(MAX_BACKOFF.as_secs());
        entry.next_attempt = Instant::now() + Duration::from_secs(secs);
    }
}

/// RAII guard removing the peer from the active-session set.
struct SessionGuard(Arc<Inner>, String);

impl Drop for SessionGuard {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.0.sessions.lock() {
            sessions.remove(&self.1);
        }
    }
}

struct PairingInProgressGuard<'a>(&'a AtomicBool);

impl Drop for PairingInProgressGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Inner {
    /// Tells the frontend the discovered/paired device lists may have changed.
    pub(crate) fn emit_devices_changed(&self) {
        let _ = self.app.emit("verenu:sync-devices-changed", ());
    }

    fn set_pairing_status(&self, status: PairingStatus) {
        if let Ok(mut pairing) = self.pairing_status.lock() {
            *pairing = Some(status);
        }
        self.emit_devices_changed();
    }

    fn update_pairing_phase(&self, generation: u64, phase: &'static str) {
        if let Ok(mut pairing) = self.pairing_status.lock() {
            if let Some(status) = pairing.as_mut() {
                if status.generation == generation {
                    status.phase = phase;
                    status.error = None;
                }
            }
        }
        self.emit_devices_changed();
    }

    fn fail_pairing(&self, generation: u64, error: String) {
        if let Ok(mut pairing) = self.pairing_status.lock() {
            if let Some(status) = pairing.as_mut() {
                if status.generation == generation {
                    status.phase = "failed";
                    status.error = Some(error);
                }
            }
        }
        self.emit_devices_changed();
    }

    fn clear_pairing_status(&self, generation: u64) {
        if let Ok(mut pairing) = self.pairing_status.lock() {
            if pairing
                .as_ref()
                .is_some_and(|status| status.generation == generation)
            {
                *pairing = None;
            }
        }
        self.emit_devices_changed();
    }
}

pub(crate) fn connection_candidates(
    addresses: &[String],
    advertised_port: u16,
    peer_uuid: &str,
) -> Vec<SocketAddr> {
    let stable_port = listener_port_for_uuid(peer_uuid);
    let mut candidates = Vec::new();
    for address in addresses {
        let Ok(parsed) = address.parse::<SocketAddr>() else {
            let Ok(ip) = address.parse::<std::net::IpAddr>() else {
                continue;
            };
            for port in [stable_port, advertised_port] {
                let candidate = SocketAddr::new(ip, port);
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
            continue;
        };
        for port in [stable_port, advertised_port] {
            let candidate = SocketAddr::new(parsed.ip(), port);
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

fn now_ms_u64() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn state_string(state: PeerState) -> String {
    match state {
        PeerState::Offline => "offline".to_string(),
        PeerState::Connecting => "connecting".to_string(),
        PeerState::Syncing => "syncing".to_string(),
        PeerState::Synced => "synced".to_string(),
        PeerState::Error => "error".to_string(),
    }
}

fn paired_set(peers: Vec<SyncPeer>) -> HashSet<String> {
    peers.into_iter().map(|p| p.device_uuid).collect()
}

fn conn_peers(db: &DbHandle) -> Vec<SyncPeer> {
    let conn = match db.lock() {
        Ok(conn) => conn,
        Err(_) => return Vec::new(),
    };
    sync_store::list_peers(&conn).unwrap_or_default()
}

fn stored_device_name(conn: &rusqlite::Connection) -> Result<Option<String>> {
    let name: Option<String> = conn
        .query_row("SELECT name FROM sync_identity LIMIT 1", [], |r| r.get(0))
        .optional()?;
    Ok(name)
}

// ---- discovery ----

/// A peer advertised itself on the LAN. Update the discovered cache, tell the
/// UI, and - if it's a paired device we're not already syncing with and its
/// backoff has elapsed - start a sync session right away.
fn handle_resolved(inner: &Arc<Inner>, info: mdns_sd::ResolvedService) {
    let uuid = info
        .get_property_val_str("uuid")
        .map(str::to_string)
        .or_else(|| {
            let instance_name = info.get_fullname().split('.').next().unwrap_or("");
            let instance = instance_name
                .strip_prefix("verenu-")
                .unwrap_or(instance_name);
            (!instance.is_empty()).then(|| instance.to_string())
        });
    let Some(uuid) = uuid else { return };
    let self_uuid = {
        let guard = inner.identity.read().expect("identity lock");
        match guard.as_ref() {
            Some(identity) => identity.uuid.clone(),
            None => return,
        }
    };
    if uuid == self_uuid || uuid.is_empty() {
        return;
    }
    let name = info
        .get_property_val_str("name")
        .unwrap_or("Verenu device")
        .to_string();
    let port = info.get_port();
    let addresses: Vec<String> = info
        .get_addresses()
        .iter()
        .map(|scoped| SocketAddr::new(scoped.to_ip_addr(), port).to_string())
        .collect();

    let changed = {
        let mut map = match inner.discovered.lock() {
            Ok(map) => map,
            Err(_) => return,
        };
        match map.get(&uuid) {
            Some(existing) => {
                let changed = existing.name != name
                    || existing.port != port
                    || existing.addresses != addresses;
                let entry = map.get_mut(&uuid).expect("checked above");
                entry.name = name.clone();
                entry.port = port;
                entry.addresses = addresses.clone();
                entry.last_seen_ms = now_ms_u64();
                changed
            }
            None => {
                map.insert(
                    uuid.clone(),
                    DiscoveredDevice {
                        uuid: uuid.clone(),
                        name: name.clone(),
                        addresses: addresses.clone(),
                        port,
                        last_seen_ms: now_ms_u64(),
                    },
                );
                true
            }
        }
    };
    if changed {
        inner.emit_devices_changed();
    }

    // Auto-sync on appearance, respecting backoff and one-session-per-peer.
    let paired = paired_set(conn_peers(&inner.db));
    if !paired.contains(&uuid) || !should_auto_initiate(&self_uuid, &uuid) {
        return;
    }
    let due = inner
        .backoff
        .lock()
        .map(|backoff| match backoff.get(&uuid) {
            Some(entry) => Instant::now() >= entry.next_attempt,
            None => true,
        })
        .unwrap_or(false);
    if !due {
        return;
    }
    let inner = inner.clone();
    let uuid = uuid.clone();
    tauri::async_runtime::spawn(async move {
        let _ = SyncManager { inner }.sync_to_peer(&uuid).await;
    });
}

// ---- incoming connection handling ----

async fn handle_connection(
    inner: Arc<Inner>,
    mut tls: TlsStream<tokio::net::TcpStream>,
    permit: tokio::sync::OwnedSemaphorePermit,
) {
    let peer_fp = match tls
        .get_ref()
        .1
        .peer_certificates()
        .ok_or_else(|| anyhow!("no certificate"))
        .and_then(|certs| transport::peer_fingerprint(certs))
    {
        Ok(fp) => fp,
        Err(err) => {
            log::debug!("sync: incoming connection without certificate: {err}");
            return;
        }
    };
    let first = match tokio::time::timeout(CONNECT_TIMEOUT, read_message(&mut tls)).await {
        Ok(Ok(message)) => message,
        _ => return,
    };
    match first {
        Message::PairRequest {
            device_uuid,
            device_name,
            protocol,
            spake_msg,
        } => {
            handle_incoming_pairing(
                inner,
                tls,
                device_uuid,
                device_name,
                protocol,
                spake_msg,
                permit,
            )
            .await;
        }
        Message::Hello(hello) => {
            handle_sync_hello(inner, tls, hello, peer_fp).await;
        }
        Message::Unpair { device_uuid } => {
            // Only honor unpair requests from the pinned certificate of the
            // device being removed - anything else is an impostor.
            let matches = (|| {
                let conn = inner.db.lock().ok()?;
                sync_store::get_peer(&conn, &device_uuid)
                    .ok()
                    .flatten()
                    .map(|peer| peer.cert_fp == peer_fp)
            })()
            .unwrap_or(false);
            if matches {
                if let Ok(conn) = inner.db.lock() {
                    let _ = sync_store::remove_peer(&conn, &device_uuid);
                }
                inner.emit_devices_changed();
                log::info!("sync: removed by peer {device_uuid}");
            }
        }
        _ => {
            let _ = send_message(
                &mut tls,
                &Message::Error {
                    message: "unexpected first message".to_string(),
                },
            )
            .await;
        }
    }
}

async fn handle_incoming_pairing(
    inner: Arc<Inner>,
    mut tls: TlsStream<tokio::net::TcpStream>,
    peer_uuid: String,
    peer_name: String,
    protocol: u32,
    spake_msg: Vec<u8>,
    permit: tokio::sync::OwnedSemaphorePermit,
) {
    if protocol != PROTOCOL_VERSION {
        let _ = send_message(
            &mut tls,
            &Message::PairReject {
                reason: format!("protocol version {protocol} not supported"),
            },
        )
        .await;
        return;
    }
    let generation;
    {
        let mut pending = inner.pending.lock().await;
        if inner.pairing_in_progress.load(Ordering::Acquire) || pending.is_some() {
            let _ = send_message(&mut tls, &Message::PairBusy).await;
            return;
        }
        generation = inner.pairing_generation.fetch_add(1, Ordering::Relaxed) + 1;
        *pending = Some(PendingPairing::Incoming {
            peer_uuid: peer_uuid.clone(),
            peer_name: peer_name.clone(),
            spake_msg,
            stream: Box::new(tls),
            permit,
            created: Instant::now(),
            generation,
        });
    }
    inner.set_pairing_status(PairingStatus {
        kind: "incoming",
        phase: "awaiting_code",
        peer_uuid: peer_uuid.clone(),
        peer_name: peer_name.clone(),
        code: None,
        error: None,
        generation,
    });
    log::info!("sync: pairing request received from {peer_name}");
    let _ = inner.app.emit(
        "verenu:sync-pair-request",
        serde_json::json!({ "uuid": peer_uuid, "name": peer_name }),
    );
    inner.emit_devices_changed();

    // Prompt watchdog: if nobody responds before the lifetime, drop the
    // request (closing the held stream) so the next pairing isn't blocked.
    let watch_inner = inner.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(PAIRING_PROMPT_LIFETIME).await;
        let mut pending = watch_inner.pending.lock().await;
        if let Some(PendingPairing::Incoming { created, .. }) = pending.as_ref() {
            if created.elapsed() >= PAIRING_PROMPT_LIFETIME - Duration::from_secs(2)
                && watch_inner.pairing_generation.load(Ordering::Relaxed) == generation
            {
                *pending = None;
                watch_inner.clear_pairing_status(generation);
                watch_inner.emit_devices_changed();
            }
        }
    });
}

async fn handle_sync_hello(
    inner: Arc<Inner>,
    mut tls: TlsStream<tokio::net::TcpStream>,
    hello: Hello,
    peer_fp: String,
) {
    // Authenticate before anything else: the Hello's uuid must map to a paired
    // device whose pinned fingerprint matches the presented certificate.
    let peer = (|| {
        let conn = inner.db.lock().ok()?;
        sync_store::get_peer(&conn, &hello.device_uuid)
            .ok()
            .flatten()
    })();
    let Some(peer) = peer else {
        let _ = send_message(
            &mut tls,
            &Message::Error {
                message: "not paired with this device".to_string(),
            },
        )
        .await;
        return;
    };
    if peer.cert_fp != peer_fp {
        log::warn!(
            "sync: fingerprint mismatch for {} - rejecting connection",
            hello.device_uuid
        );
        let _ = send_message(
            &mut tls,
            &Message::Error {
                message: "certificate does not match the paired device".to_string(),
            },
        )
        .await;
        return;
    }
    if hello.protocol != PROTOCOL_VERSION {
        let _ = send_message(
            &mut tls,
            &Message::Error {
                message: format!(
                    "peer speaks sync protocol v{}, this device speaks v{PROTOCOL_VERSION}",
                    hello.protocol
                ),
            },
        )
        .await;
        return;
    }
    // Reserve the session before acknowledging it. Otherwise a duplicate
    // connection can receive HelloAck and then fail at the first session read.
    let session_available = {
        let mut sessions = inner.sessions.lock().expect("sessions lock");
        sessions.insert(hello.device_uuid.clone())
    };
    if !session_available {
        let _ = send_message(
            &mut tls,
            &Message::Error {
                message: "a sync session with this device is already running".to_string(),
            },
        )
        .await;
        return;
    }
    let _session_guard = SessionGuard(inner.clone(), hello.device_uuid.clone());
    let manager = SyncManager {
        inner: inner.clone(),
    };
    manager.set_status(&peer.device_uuid, PeerState::Syncing, None);
    let host = ManagerHost::new(&inner);
    // A stalled peer must not hold the session slot forever.
    let result = match tokio::time::timeout(
        Duration::from_secs(600),
        engine::run_session_after_hello(&inner.db, &host, &mut tls, false, &peer, Some(hello)),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(anyhow!("sync session with {} timed out", peer.name)),
    };
    match result {
        Ok(summary) => {
            if let Ok(conn) = inner.db.lock() {
                let _ = sync_store::mark_peer_synced(&conn, &peer.device_uuid, 0);
                let _ = sync_store::compact_log(&conn);
            }
            manager.reset_backoff(&peer.device_uuid);
            manager.set_status(&peer.device_uuid, PeerState::Synced, None);
            if summary.applied.applied > 0 || summary.settings_applied > 0 {
                let _ = inner.app.emit(
                    "verenu:sync-data-changed",
                    serde_json::json!({ "tables": summary.applied.touched_tables() }),
                );
            }
        }
        Err(err) => {
            log::warn!("sync: incoming session with {} failed: {err:#}", peer.name);
            if let Ok(conn) = inner.db.lock() {
                let _ = sync_store::mark_peer_error(&conn, &peer.device_uuid, &format!("{err:#}"));
            }
            manager.set_status(
                &peer.device_uuid,
                PeerState::Error,
                Some(&format!("{err:#}")),
            );
        }
    }
}

// ---- SyncHost implementation over the real app ----

pub(crate) struct ManagerHost {
    db: DbHandle,
    settings: SettingsHandle,
    app: AppHandle,
    uuid: String,
    /// App enumeration is relatively expensive on both platforms.  Most sync
    /// sessions contain no context-target changes, so defer it until the
    /// engine actually needs to resolve a target.
    installed_apps: OnceLock<Vec<crate::system::apps::InstalledApp>>,
}

impl ManagerHost {
    pub fn new(inner: &Arc<Inner>) -> Self {
        let uuid = inner
            .identity
            .read()
            .ok()
            .and_then(|identity| identity.as_ref().map(|identity| identity.uuid.clone()))
            .unwrap_or_default();
        Self {
            db: inner.db.clone(),
            settings: store::settings_handle(&inner.app).unwrap_or_else(|_| {
                // SettingsHandle::open failing here is practically impossible
                // (the same file opened fine at startup); fall back to a fresh
                // handle so sync continues with in-memory settings.
                store::SettingsHandle::open(&inner.app).expect("settings handle")
            }),
            app: inner.app.clone(),
            uuid,
            installed_apps: OnceLock::new(),
        }
    }
}

impl SyncHost for ManagerHost {
    fn device_uuid(&self) -> String {
        self.uuid.clone()
    }

    fn device_name(&self) -> String {
        // Read through the manager's identity when reachable; fall back to the
        // DB row (kept in sync by set_device_name).
        self.app
            .try_state::<SyncManager>()
            .map(|manager| manager.device_info().name)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "This device".to_string())
    }

    fn app_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    fn settings_payload(&self) -> anyhow::Result<Vec<super::protocol::SettingRecord>> {
        let stamps: HashMap<String, (i64, String)> = (|| {
            let conn = self.db.lock().ok()?;
            Some(
                sync_store::list_setting_stamps(&conn)
                    .ok()?
                    .into_iter()
                    .map(|(key, stamp)| (key, (stamp.ts_ms, stamp.origin)))
                    .collect(),
            )
        })()
        .unwrap_or_default();
        let mut records = Vec::new();
        for key in engine::SYNCABLE_SETTINGS {
            if let Some(value) = self.settings.get(key) {
                let (ts_ms, origin) = stamps.get(*key).cloned().unwrap_or((0, String::new()));
                records.push(super::protocol::SettingRecord {
                    key: key.to_string(),
                    value,
                    ts_ms,
                    origin,
                });
            }
        }
        Ok(records)
    }

    fn apply_remote_setting(&self, key: &str, value: &serde_json::Value) -> Result<(), String> {
        self.apply_remote_settings(&[(key.to_string(), value.clone())])
    }

    fn apply_remote_settings(
        &self,
        settings: &[(String, serde_json::Value)],
    ) -> Result<(), String> {
        // Values come from a trusted peer, but they still go through the same
        // validation the local save path uses.
        for (key, value) in settings {
            validate_setting(key, value)?;
        }

        let mut values = Vec::with_capacity(settings.len() * 3);
        for (key, value) in settings {
            if key == store::CONTEXTUAL_FORMATTING {
                values.push((store::CONTEXTUAL_FORMATTING, value.clone()));
                values.push((store::CONTEXTUAL_CAPS, value.clone()));
                values.push((store::AUTO_SPACING, value.clone()));
            } else {
                values.push((key.as_str(), value.clone()));
            }
        }
        // Persist first. SettingsHandle only publishes the new in-memory map
        // after the atomic document write succeeds, so a failed save cannot
        // leave a value visible without its sync stamp.
        self.settings.save_values_if_changed(values)?;

        // Side effects that keep the running app consistent with the new values.
        for (key, value) in settings {
            if crate::app_tray::setting_updates_runtime_icons(key) {
                crate::app_tray::apply_runtime_icons(&self.app, None);
            }
            if key == store::SOUND_EFFECTS_VOLUME {
                if let Some(volume) = value.as_f64() {
                    crate::media::sound::set_volume((volume as f32) / 100.0);
                }
            }
        }
        Ok(())
    }

    fn resolve_app_target(&self, source: &str) -> Option<String> {
        closest_installed_app(
            source,
            self.installed_apps
                .get_or_init(crate::system::apps::list_installed_apps),
        )
        .map(|app| app.exe.clone())
    }

    fn resolve_app_target_with_metadata(
        &self,
        source: &str,
        app_name: Option<&str>,
        developer: Option<&str>,
    ) -> Option<(String, Option<String>, Option<String>)> {
        crate::system::apps::closest_installed_app(
            source,
            app_name,
            developer,
            self.installed_apps
                .get_or_init(crate::system::apps::list_installed_apps),
        )
        .map(|app| {
            (
                app.exe.clone(),
                Some(app.name.clone()),
                app.developer.clone(),
            )
        })
    }
}

pub(crate) fn closest_installed_app<'a>(
    source: &str,
    apps: &'a [crate::system::apps::InstalledApp],
) -> Option<&'a crate::system::apps::InstalledApp> {
    crate::system::apps::closest_installed_app(source, None, None, apps)
}
