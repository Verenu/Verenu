use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};
use zbus::{blocking::Connection, interface};

// A launcher holds this lock until its callback is acknowledged. The original
// holds it exclusively across the final handoff and until process exit, so a
// call cannot be directed to an owner that is in the middle of exiting.
fn open_callback_lock() -> Result<std::fs::File, String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
    use sha2::{Digest, Sha256};
    let uid = unsafe { libc::geteuid() };
    let bus: String = Connection::session()
        .and_then(|connection| connection.call_method(
            Some("org.freedesktop.DBus"), "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"), "GetId", &(),
        ))
        .and_then(|message| message.body().deserialize())
        .map_err(|error| format!("Could not identify the shortcut session: {error}"))?;
    let digest = format!("{:x}", Sha256::digest(bus.as_bytes()));
    let directory = std::path::PathBuf::from(format!("/tmp/verenu-callbacks-{uid}-{}", &digest[..16]));
    match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(format!("Could not prepare shortcut routing: {error}")),
    }
    let metadata = std::fs::symlink_metadata(&directory)
        .map_err(|error| format!("Could not inspect shortcut routing: {error}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.uid() != uid
        || metadata.mode() & 0o077 != 0
    {
        return Err("Shortcut routing directory is not private".into());
    }
    let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(directory.join("lock"))
        .map_err(|error| format!("Could not open shortcut routing lock: {error}"))?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 || metadata.nlink() != 1 {
        return Err("Shortcut routing lock is not private".into());
    }
    Ok(file)
}

pub(crate) fn callback_lock(timeout: std::time::Duration) -> Result<std::fs::File, String> {
    let file = open_callback_lock()?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(file);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::WouldBlock && error.kind() != std::io::ErrorKind::Interrupted {
            return Err(format!("Could not lock shortcut routing: {error}"));
        }
        #[cfg(all(feature = "native-testing", debug_assertions))]
        if let Some(path) = std::env::var_os("VERENU_HANDOFF_TEST_LAUNCHER_WAIT") {
            let _ = std::fs::write(path, b"waiting");
        }
        if std::time::Instant::now() >= deadline {
            return Err("Timed out waiting for shortcut routing".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

// Tickets order launchers before they wait for the handoff lock. The callback
// queue still owns callbacks already accepted by the running instance.
struct LauncherTicket {
    path: std::path::PathBuf,
    _ticket: std::fs::File,
    coordinator: std::fs::File,
    lock: std::fs::File,
}

fn private_queue_file(path: &std::path::Path, create_new: bool) -> Result<std::fs::File, String> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new().read(true).write(true)
        .create(!create_new).create_new(create_new).truncate(false).mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path).map_err(|error| format!("Could not open shortcut queue: {error}"))?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0 || metadata.nlink() != 1
    {
        return Err("Shortcut queue file is not private".into());
    }
    Ok(file)
}

fn flock(file: &std::fs::File, flags: libc::c_int) -> Result<bool, String> {
    use std::os::fd::AsRawFd;
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), flags) } == 0 { return Ok(true); }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted { continue; }
        if error.kind() == std::io::ErrorKind::WouldBlock { return Ok(false); }
        return Err(format!("Could not lock shortcut queue: {error}"));
    }
}

fn unlock(file: &std::fs::File) {
    let _ = flock(file, libc::LOCK_UN);
}

impl LauncherTicket {
    fn acquire(timeout: Option<std::time::Duration>) -> Result<Self, String> {
        use std::os::fd::AsRawFd;
        let lock = open_callback_lock()?;
        let lock_path = std::fs::read_link(format!("/proc/self/fd/{}", lock.as_raw_fd()))
            .map_err(|error| format!("Could not locate shortcut queue: {error}"))?;
        let directory = lock_path.parent().ok_or("Shortcut queue has no directory")?;
        let coordinator = private_queue_file(&directory.join("queue"), false)?;
        flock(&coordinator, libc::LOCK_EX)?;
        let result = (|| -> Result<(std::path::PathBuf, std::fs::File), String> {
            let mut next = 0u64;
            for entry in std::fs::read_dir(directory).map_err(|error| error.to_string())? {
                let entry = entry.map_err(|error| error.to_string())?;
                let name = entry.file_name();
                let Some(number) = name.to_str().and_then(|s| s.strip_prefix("ticket-"))
                    .and_then(|s| s.parse::<u64>().ok()) else { continue };
                next = next.max(number.checked_add(1).ok_or("Shortcut queue is full")?);
            }
            let path = directory.join(format!("ticket-{next:020}"));
            let ticket = private_queue_file(&path, true)?;
            if !flock(&ticket, libc::LOCK_EX | libc::LOCK_NB)? {
                return Err("Could not reserve shortcut queue ticket".into());
            }
            Ok((path, ticket))
        })();
        unlock(&coordinator);
        let (path, ticket) = result?;
        let result = Self { path, _ticket: ticket, coordinator, lock };
        #[cfg(all(feature = "native-testing", debug_assertions))]
        if let Some(path) = std::env::var_os("VERENU_HANDOFF_TEST_LAUNCHER_WAIT") {
            let _ = std::fs::write(path, b"waiting");
        }
        let deadline = timeout.map(|duration| std::time::Instant::now() + duration);
        loop {
            if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
                return Err("Timed out waiting for shortcut routing".into());
            }
            if result.is_first()? && flock(&result.lock, libc::LOCK_EX | libc::LOCK_NB)? {
                return Ok(result);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn is_first(&self) -> Result<bool, String> {
        flock(&self.coordinator, libc::LOCK_EX)?;
        let result = (|| -> Result<bool, String> {
            let directory = self.path.parent().ok_or("Shortcut queue has no directory")?;
            let mut first = true;
            for entry in std::fs::read_dir(directory).map_err(|error| error.to_string())? {
                let path = entry.map_err(|error| error.to_string())?.path();
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else { continue };
                if !name.strip_prefix("ticket-").is_some_and(|number| number.len() == 20 && number.parse::<u64>().is_ok())
                    || path == self.path { continue; }
                let ticket = private_queue_file(&path, false)?;
                if flock(&ticket, libc::LOCK_EX | libc::LOCK_NB)? {
                    std::fs::remove_file(path).map_err(|error| error.to_string())?;
                } else if path < self.path {
                    first = false;
                }
            }
            Ok(first)
        })();
        unlock(&self.coordinator);
        result
    }
}

impl Drop for LauncherTicket {
    fn drop(&mut self) {
        if flock(&self.coordinator, libc::LOCK_EX).is_ok() {
            let _ = std::fs::remove_file(&self.path);
            unlock(&self.coordinator);
        }
    }
}

fn validate_callback_lock(fd: &zbus::zvariant::OwnedFd, expected_file: &std::fs::File) -> zbus::fdo::Result<()> {
    use std::os::{fd::{AsFd, AsRawFd}, unix::fs::MetadataExt};
    let received = std::fs::File::from(
        fd.as_fd().try_clone_to_owned().map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?,
    );
    let actual = received.metadata().map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
    let expected = expected_file.metadata().map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
    if actual.dev() != expected.dev() || actual.ino() != expected.ino()
        || unsafe { libc::flock(fd.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0
    {
        return Err(zbus::fdo::Error::AccessDenied("Shortcut callback requires the session routing lock".into()));
    }
    Ok(())
}

#[cfg(all(test, feature = "native-testing"))]
pub(crate) fn test_callback(connection: &Connection, destination: &str, path: &str, argv: Vec<String>) -> Result<(), String> {
    let lock = callback_lock(std::time::Duration::from_secs(10))?;
    connection.call_method(
        Some(destination), path, Some("org.SingleInstance.DBus"), "ExecuteCallbackGuarded",
        &(argv, String::new(), zbus::zvariant::Fd::from(&lock)),
    ).map(|_| ()).map_err(|error| error.to_string())
}

struct CallbackRoute {
    replacement: std::sync::atomic::AtomicBool,
    local_ready: std::sync::atomic::AtomicBool,
    queue: Mutex<Option<CallbackQueueSlot>>,
    lock_file: Option<std::fs::File>,
}

struct CallbackQueueSlot {
    queue: Arc<crate::linux_handoff_callbacks::CallbackQueue>,
    delivered: Option<i64>,
    failed: bool,
}

impl CallbackRoute {
    fn new(replacement: bool) -> Self {
        let lock_file = match open_callback_lock() {
            Ok(file) => Some(file),
            Err(error) => {
                log::error!("Shortcut routing is unavailable: {error}");
                None
            }
        };
        Self { replacement: std::sync::atomic::AtomicBool::new(replacement), local_ready: std::sync::atomic::AtomicBool::new(!replacement), queue: Mutex::new(None), lock_file }
    }

    fn enqueue_if_active(&self, args: &[String]) -> Option<zbus::fdo::Result<()>> {
        let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        queue.as_mut().map(|slot| {
            let callback = crate::linux_handoff_callbacks::Callback::from_args(args);
            slot.queue.push(callback)
                .map_err(|error| {
                    slot.failed = true;
                    zbus::fdo::Error::Failed(format!("Could not queue restart callback: {error}"))
                })
        })
    }

}

#[cfg(all(test, feature = "native-testing"))]
static TEST_CALLBACK_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(all(test, feature = "native-testing"))]
static TEST_CALLBACKS: Mutex<Vec<crate::linux_handoff_callbacks::Callback>> = Mutex::new(Vec::new());

#[cfg(all(test, feature = "native-testing"))]
static TEST_CALLBACK_STARTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(all(test, feature = "native-testing"))]
pub(crate) fn test_callback_count() -> usize {
    TEST_CALLBACK_COUNT.load(std::sync::atomic::Ordering::SeqCst)
}

#[cfg(all(test, feature = "native-testing"))]
pub(crate) fn test_callback_started() -> usize {
    TEST_CALLBACK_STARTED.load(std::sync::atomic::Ordering::SeqCst)
}

struct SingleInstance {
    app: AppHandle,
    route: Arc<CallbackRoute>,
}

#[interface(name = "org.SingleInstance.DBus", spawn = false)]
impl SingleInstance {
    fn handoff_barrier(&self) -> zbus::fdo::Result<()> {
        let queue = self.route.queue.lock().unwrap_or_else(|error| error.into_inner());
        match queue.as_ref() {
            Some(slot) if !slot.failed => Ok(()),
            Some(_) => Err(zbus::fdo::Error::Failed("A restart callback could not be queued".into())),
            None => Err(zbus::fdo::Error::Failed("Restart callback queue is unavailable".into())),
        }
    }

    fn execute_callback(&self, _argv: Vec<String>, _cwd: String) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported("Legacy shortcut callbacks are unsupported; launch the installed Verenu application instead".into()))
    }

    async fn execute_callback_guarded(
        &self,
        argv: Vec<String>,
        _cwd: String,
        lock: zbus::zvariant::OwnedFd,
    ) -> zbus::fdo::Result<()> {
        let expected = self.route.lock_file.as_ref().ok_or_else(|| zbus::fdo::Error::Failed("Shortcut routing lock is unavailable".into()))?;
        validate_callback_lock(&lock, expected)?;
        if self.route.replacement.load(std::sync::atomic::Ordering::Acquire) {
            if let Some(result) = self.route.enqueue_if_active(&argv) {
                return result;
            }
            if !self.route.local_ready.load(std::sync::atomic::Ordering::Acquire) {
                return Err(zbus::fdo::Error::Failed("Restart callback routing is not active".into()));
            }
            return self.dispatch_callback(argv).await;
        }
        self.dispatch_callback(argv).await
    }
}

impl SingleInstance {
    async fn dispatch_callback(
        &self,
        argv: Vec<String>,
    ) -> zbus::fdo::Result<()> {
        // The hotkey callbacks change gesture state before dispatching work.
        // Keep them out of a paused parent so a failed handoff can resume
        // and handle the original event with its state intact.
        #[cfg(all(test, feature = "native-testing"))]
        TEST_CALLBACK_STARTED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let gate = self.app.try_state::<Arc<crate::linux_activity::ActivityGate>>();
        let _activity = if let Some(gate) = gate {
            match gate.try_enter() {
                Some(activity) => Some(Arc::new(activity)),
                None => {
                    if let Some(result) = self.route.enqueue_if_active(&argv) {
                        return result;
                    }
                    Some(Arc::new(gate.enter().await))
                }
            }
        } else {
            None
        };
        dispatch_callback(&self.app, crate::linux_handoff_callbacks::Callback::from_args(&argv));
        Ok(())
    }
}

fn dispatch_callback(app: &AppHandle, callback: crate::linux_handoff_callbacks::Callback) {
    #[cfg(all(test, feature = "native-testing"))]
    {
        TEST_CALLBACK_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        TEST_CALLBACKS.lock().unwrap().push(callback);
    }
    match callback {
        crate::linux_handoff_callbacks::Callback::Release => crate::core::hotkey::notify_release(),
        crate::linux_handoff_callbacks::Callback::Handsfree => crate::core::hotkey::notify_handless(),
        crate::linux_handoff_callbacks::Callback::CaptureSubApp => crate::core::hotkey::notify_capture_sub_app(),
        crate::linux_handoff_callbacks::Callback::Show => crate::show_main_window(app),
    }
}

pub(crate) struct InstanceOwner {
    connection: Connection,
    name: String,
    route: Arc<CallbackRoute>,
}

pub(crate) struct PreparedCallbacks<'a> {
    route: &'a CallbackRoute,
    slot: std::sync::MutexGuard<'a, Option<CallbackQueueSlot>>,
    reader: crate::linux_handoff_callbacks::CallbackReader,
}

impl PreparedCallbacks<'_> {
    pub(crate) fn activate(mut self, app: &AppHandle, _parent_exited: &crate::linux_handoff_transport::ParentExited) {
        if let Err(error) = self.reader.drain(|callback| dispatch_callback(app, callback)) {
            use tauri::Emitter;
            log::error!("Could not read restart callbacks: {error}");
            let _ = app.emit("verenu:error", "Some shortcuts could not be restored after restarting.");
        }
        self.route.local_ready.store(true, std::sync::atomic::Ordering::Release);
        self.route.replacement.store(false, std::sync::atomic::Ordering::Release);
        *self.slot = None;
    }
}

impl InstanceOwner {
    pub(crate) fn prepare_callbacks<'a>(
        &'a self,
        reader: crate::linux_handoff_callbacks::CallbackReader,
        pause: &crate::linux_activity::Pause,
        gate: &Arc<crate::linux_activity::ActivityGate>,
    ) -> Result<PreparedCallbacks<'a>, String> {
        if !pause.belongs_to(gate) || !self.route.replacement.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Restart callback gate does not match".into());
        }
        let slot = self.route.queue.lock().map_err(|_| "Restart callback queue is unavailable")?;
        Ok(PreparedCallbacks { route: &self.route, slot, reader })
    }

    pub(crate) fn install_callback_queue(
        &self,
        queue: Arc<crate::linux_handoff_callbacks::CallbackQueue>,
    ) -> Result<(), &'static str> {
        let mut slot = self.route.queue.lock().unwrap_or_else(|error| error.into_inner());
        if slot.is_some() {
            return Err("A restart callback queue is already active");
        }
        *slot = Some(CallbackQueueSlot { queue, delivered: None, failed: false });
        Ok(())
    }

    pub(crate) fn callback_barrier(&self) -> Result<(), String> {
        let unique = self.unique_name()?;
        self.callback_barrier_at(&unique)
    }

    fn callback_barrier_at(&self, unique: &str) -> Result<(), String> {
        let path = format!("/{}", self.name.replace('.', "/").replace('-', "_"));
        Connection::session()
            .and_then(|connection| connection.call_method(
                Some(unique), path.as_str(), Some("org.SingleInstance.DBus"),
                "HandoffBarrier", &(),
            ))
            .map(|_| ())
            .map_err(|error| format!("Could not finish pending restart callbacks: {error}"))
    }

    pub(crate) fn drain_callback_queue(&self, app: &AppHandle) -> Result<(), String> {
        if self.route.replacement.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Replacement callbacks require confirmed parent exit".into());
        }
        let mut slot = self.route.queue.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(active) = slot.as_mut() {
            while let Some((id, callback)) = active.queue.first().map_err(|error| error.to_string())? {
                if active.delivered != Some(id) {
                    dispatch_callback(app, callback);
                    active.delivered = Some(id);
                }
                if !active.queue.acknowledge(id).map_err(|error| error.to_string())? {
                    return Err("Restart callback queue changed unexpectedly".into());
                }
                active.delivered = None;
            }
        }
        *slot = None;
        Ok(())
    }
    pub(crate) fn unique_name(&self) -> Result<String, String> {
        self.connection
            .unique_name()
            .map(ToString::to_string)
            .ok_or_else(|| "Single-instance connection has no unique name".into())
    }
    pub(crate) fn owns_name(&self) -> Result<bool, String> {
        let current: String = self
            .connection
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetNameOwner",
                &(self.name.as_str(),),
            )
            .and_then(|message| message.body().deserialize())
            .map_err(|error| format!("Could not inspect instance ownership: {error}"))?;
        Ok(self
            .connection
            .unique_name()
            .is_some_and(|name| name.as_str() == current))
    }

    pub(crate) fn verify_queued_replacement(&self, group: u32) -> Result<(), String> {
        let owners: Vec<String> = self.connection.call_method(
            Some("org.freedesktop.DBus"), "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"), "ListQueuedOwners", &(self.name.as_str(),),
        ).and_then(|message| message.body().deserialize())
            .map_err(|error| format!("Could not inspect the instance queue: {error}"))?;
        if owners.first() != Some(&self.unique_name()?) {
            return Err("Verenu no longer owns its single-instance name".into());
        }
        let replacement = owners.get(1).ok_or("The replacement is not queued for instance ownership")?;
        let pid: u32 = self.connection.call_method(
            Some("org.freedesktop.DBus"), "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"), "GetConnectionUnixProcessID", &(replacement.as_str(),),
        ).and_then(|message| message.body().deserialize())
            .map_err(|error| format!("Could not identify replacement owner: {error}"))?;
        if !crate::linux_handoff_transport::process_in_replacement_group(pid, group) {
            return Err("A different process is queued before the replacement".into());
        }
        Ok(())
    }

}

pub(crate) fn replacement_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("single-instance-replacement")
        .setup(move |app, _api| {
            let (name, path) = address(&app.config().identifier);
            let route = Arc::new(CallbackRoute::new(true));
            let connection = zbus::blocking::connection::Builder::session()?
                .serve_at(
                    path.as_str(),
                    SingleInstance {
                        app: app.clone(),
                        route: route.clone(),
                    },
                )?
                .build()?;
            let reply = connection.request_name_with_flags(name.as_str(), enumflags2::BitFlags::empty())?;
            if reply != zbus::fdo::RequestNameReply::InQueue {
                return Err(std::io::Error::other("Original instance is not holding its name").into());
            }
            app.manage(InstanceOwner { connection, name, route });
            Ok(())
        })
        .on_event(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(owner) = app.try_state::<InstanceOwner>() {
                    std::mem::forget(owner.connection.clone());
                }
            }
        })
        .build()
}

fn address(identifier: &str) -> (String, String) {
    let name = format!("{identifier}.SingleInstance");
    let path = format!("/{}", name.replace('.', "/").replace('-', "_"));
    (name, path)
}

pub(crate) fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("single-instance")
        .setup(|app, _api| {
            let (name, path) = address(&app.config().identifier);
            let route = Arc::new(CallbackRoute::new(false));
            let result = zbus::blocking::connection::Builder::session()?
                .name(name.as_str())?
                .replace_existing_names(false)
                .allow_name_replacements(false)
                .serve_at(
                    path.as_str(),
                    SingleInstance {
                        app: app.clone(),
                        route: route.clone(),
                    },
                )?
                .build();
            match result {
                Ok(connection) => {
                    app.manage(InstanceOwner { connection, name, route });
                }
                Err(zbus::Error::NameTaken) => {
                    let result = (|| -> Result<(), String> {
                        let lock = LauncherTicket::acquire(None)?;
                        let connection = Connection::session().map_err(|error| error.to_string())?;
                        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                        while connection.call_method(
                            Some("org.freedesktop.DBus"), "/org/freedesktop/DBus",
                            Some("org.freedesktop.DBus"), "GetNameOwner", &(name.as_str(),),
                        ).is_err() {
                            if std::time::Instant::now() >= deadline {
                                return Err("Verenu's shortcut service is unavailable".into());
                            }
                            std::thread::sleep(std::time::Duration::from_millis(20));
                        }
                        connection.call_method(
                            Some(name.as_str()),
                            path.as_str(),
                            Some("org.SingleInstance.DBus"),
                            "ExecuteCallbackGuarded",
                            &(
                                std::env::args().collect::<Vec<_>>(),
                                std::env::current_dir()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .to_string(),
                                zbus::zvariant::Fd::from(&lock.lock),
                            ),
                        ).map_err(|error| format!("Could not deliver shortcut: {error}"))?;
                        Ok(())
                    })();
                    if let Err(error) = result {
                        log::error!("{error}");
                        app.cleanup_before_exit();
                        std::process::exit(1);
                    }
                    app.cleanup_before_exit();
                    std::process::exit(0);
                }
                Err(error) => log::warn!("Linux single-instance service unavailable: {error}"),
            }
            Ok(())
        })
        .on_event(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(owner) = app.try_state::<InstanceOwner>() {
                    std::mem::forget(owner.connection.clone());
                }
            }
        })
        .build()
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires an isolated D-Bus session"]
    fn launchers_wait_in_registration_order_and_cancel_cleanly() {
        use std::{os::fd::AsRawFd, sync::mpsc, time::{Duration, Instant}};
        let held = super::callback_lock(Duration::from_secs(1)).unwrap();
        let lock_path = std::fs::read_link(format!("/proc/self/fd/{}", held.as_raw_fd())).unwrap();
        let directory = lock_path.parent().unwrap();
        let (send, receive) = mpsc::channel();
        let mut workers = Vec::new();
        for index in 0..8 {
            let send = send.clone();
            workers.push(std::thread::spawn(move || {
                let _ticket = super::LauncherTicket::acquire(Some(Duration::from_secs(10))).unwrap();
                send.send(index).unwrap();
            }));
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let count = std::fs::read_dir(directory).unwrap().filter(|entry| {
                    entry.as_ref().unwrap().file_name().to_string_lossy().starts_with("ticket-")
                }).count();
                if count == index + 1 { break; }
                assert!(Instant::now() < deadline, "launcher did not register");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let canceled = std::thread::spawn(|| super::LauncherTicket::acquire(Some(Duration::from_millis(100))).is_err());
        assert!(canceled.join().unwrap());
        drop(held);
        assert_eq!((0..8).map(|_| receive.recv_timeout(Duration::from_secs(5)).unwrap()).collect::<Vec<_>>(), (0..8).collect::<Vec<_>>());
        for worker in workers { worker.join().unwrap(); }
        let stale = directory.join("ticket-00000000000000000000");
        drop(super::private_queue_file(&stale, true).unwrap());
        let _ticket = super::LauncherTicket::acquire(Some(Duration::from_secs(1))).unwrap();
        assert!(!stale.exists());
    }

    #[test]
    #[ignore = "requires an isolated D-Bus session"]
    fn shortcut_lock_waits_for_inflight_launchers() {
        use std::time::Duration;
        let inflight = super::callback_lock(Duration::from_secs(1)).unwrap();
        let direct = zbus::zvariant::OwnedFd::from(std::os::fd::OwnedFd::from(
            super::open_callback_lock().unwrap(),
        ));
        assert!(super::validate_callback_lock(&direct, &inflight).is_err());
        let blocked = std::thread::spawn(|| super::callback_lock(Duration::from_millis(100)));
        assert!(blocked.join().unwrap().is_err());
        drop(inflight);
        let expected = super::open_callback_lock().unwrap();
        super::validate_callback_lock(&direct, &expected).unwrap();
        drop(direct);
        let next = super::callback_lock(Duration::from_secs(1)).unwrap();
        drop(next);
    }

    #[cfg(feature = "native-testing")]
    #[test]
    #[ignore = "requires an isolated graphical and D-Bus session"]
    fn replacement_routes_callbacks_locally_after_queue_activation() {
        use std::sync::Arc;
        use tauri::Manager;

        let (queue, path) = crate::linux_handoff_callbacks::CallbackQueue::create_private().unwrap();
        let reader = crate::linux_handoff_callbacks::CallbackReader::open(&path).unwrap();
        let queue = Arc::new(queue);
        let gate = Arc::new(crate::linux_activity::ActivityGate::default());
        let app = tauri::Builder::default()
            .any_thread()
            .manage(gate.clone())
            .build(tauri::generate_context!())
            .unwrap();
        let (_, path_on_bus) = super::address(&app.config().identifier);
        let route = Arc::new(super::CallbackRoute::new(false));
        *route.queue.lock().unwrap() = Some(super::CallbackQueueSlot {
            queue: queue.clone(), delivered: None, failed: false,
        });
        let original = zbus::blocking::connection::Builder::session().unwrap()
            .name("com.verenu.app.SingleInstance").unwrap()
            .allow_name_replacements(false)
            .serve_at(path_on_bus.as_str(), super::SingleInstance { app: app.handle().clone(), route: route.clone() }).unwrap()
            .build().unwrap();
        app.handle().plugin(super::replacement_plugin()).unwrap();
        let uninitialized = super::SingleInstance {
            app: app.handle().clone(),
            route: Arc::new(super::CallbackRoute::new(true)),
        };
        assert!(uninitialized.execute_callback(vec!["--verenu-hotkey-release".into()], String::new()).is_err());
        let owner = app.state::<super::InstanceOwner>();
        assert!(owner.drain_callback_queue(app.handle()).is_err());
        let connection = zbus::blocking::Connection::session().unwrap();
        assert!(super::test_callback(&connection, &owner.unique_name().unwrap(), &path_on_bus,
            vec!["--verenu-hotkey-release".into()]).is_err());
        let send = |flag: &str| super::test_callback(&connection, &owner.name, &path_on_bus,
            vec![flag.to_owned()]).unwrap();
        let before = super::test_callback_count();
        let recorded = super::TEST_CALLBACKS.lock().unwrap().len();
        let pause = tauri::async_runtime::block_on(gate.pause()).unwrap();
        let held = route.queue.lock().unwrap();
        let started = super::test_callback_started();
        let old_name = original.unique_name().unwrap().to_string();
        let first_path = path_on_bus.clone();
        let first = std::thread::spawn(move || {
            super::test_callback(&zbus::blocking::Connection::session().unwrap(), &old_name,
                &first_path, vec!["--verenu-hotkey-release".into()]).unwrap();
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while super::test_callback_started() == started {
            assert!(std::time::Instant::now() < deadline, "first callback did not arrive");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let (done, wait_done) = std::sync::mpsc::channel();
        let second = std::thread::spawn(move || {
            super::test_callback(&zbus::blocking::Connection::session().unwrap(),
                "com.verenu.app.SingleInstance", "/com/verenu/app/SingleInstance",
                vec!["--verenu-hotkey-handsfree".into()]).unwrap();
            done.send(()).unwrap();
        });
        assert!(wait_done.recv_timeout(std::time::Duration::from_millis(100)).is_err());
        drop(held);
        first.join().unwrap();
        wait_done.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        second.join().unwrap();
        send("--verenu-capture-sub-app");
        assert_eq!(super::test_callback_count(), before);
        assert!(!owner.owns_name().unwrap());
        let callbacks = owner.prepare_callbacks(reader, &pause, &gate).unwrap();
        original.release_name("com.verenu.app.SingleInstance").unwrap();
        drop(original);
        drop(route);
        drop(queue);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !owner.owns_name().unwrap_or(false) {
            assert!(std::time::Instant::now() < deadline, "replacement did not acquire the name");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let parent_exited = crate::linux_handoff_transport::parent_exited_for_test();
        callbacks.activate(app.handle(), &parent_exited);
        drop(pause);
        assert_eq!(super::test_callback_count(), before + 3);
        assert_eq!(&super::TEST_CALLBACKS.lock().unwrap()[recorded..], &[
            crate::linux_handoff_callbacks::Callback::Release,
            crate::linux_handoff_callbacks::Callback::Handsfree,
            crate::linux_handoff_callbacks::Callback::CaptureSubApp,
        ]);
        send("--verenu-hotkey-handsfree");
        assert_eq!(super::test_callback_count(), before + 4);
        let (next_queue, next_path) = crate::linux_handoff_callbacks::CallbackQueue::create_private().unwrap();
        let next_queue = Arc::new(next_queue);
        owner.install_callback_queue(next_queue.clone()).unwrap();
        next_queue.push(crate::linux_handoff_callbacks::Callback::Release).unwrap();
        owner.drain_callback_queue(app.handle()).unwrap();
        assert_eq!(super::test_callback_count(), before + 5);
        drop(next_queue);
        std::fs::remove_dir_all(next_path.parent().unwrap()).unwrap();
        drop(owner);
        drop(app);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[cfg(feature = "native-testing")]
    #[test]
    #[ignore = "requires an isolated graphical and D-Bus session"]
    fn paused_instance_handles_forwarded_hotkey_after_resuming() {
        use std::sync::Arc;
        use std::time::Duration;

        let gate = Arc::new(crate::linux_activity::ActivityGate::default());
        let app = tauri::Builder::default()
            .any_thread()
            .manage(gate.clone())
            .plugin(super::plugin())
            .build(tauri::generate_context!())
            .unwrap();
        let pause = tauri::async_runtime::block_on(gate.pause()).unwrap();
        let count = super::test_callback_count();
        let started = super::TEST_CALLBACK_STARTED.load(std::sync::atomic::Ordering::SeqCst);
        let (sent, received) = std::sync::mpsc::channel();
        let sender = std::thread::spawn(move || {
            let connection = zbus::blocking::Connection::session().unwrap();
            let result = super::test_callback(&connection, "com.verenu.app.SingleInstance",
                "/com/verenu/app/SingleInstance", vec!["--verenu-hotkey-release".into()]);
            sent.send(result.is_ok()).unwrap();
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while super::TEST_CALLBACK_STARTED.load(std::sync::atomic::Ordering::SeqCst) == started {
            assert!(std::time::Instant::now() < deadline, "callback did not arrive");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(received.recv_timeout(Duration::from_millis(200)).is_err());
        assert_eq!(super::test_callback_count(), count);
        let (barrier_sent, barrier_received) = std::sync::mpsc::channel();
        let barrier = std::thread::spawn(move || {
            let connection = zbus::blocking::Connection::session().unwrap();
            let result = connection.call_method(
                Some("com.verenu.app.SingleInstance"), "/com/verenu/app/SingleInstance",
                Some("org.SingleInstance.DBus"), "HandoffBarrier", &(),
            );
            barrier_sent.send(result.is_err()).unwrap();
        });
        assert!(barrier_received.recv_timeout(Duration::from_millis(200)).is_err());
        drop(pause);
        assert!(received.recv_timeout(Duration::from_secs(5)).unwrap());
        assert!(barrier_received.recv_timeout(Duration::from_secs(5)).unwrap());
        sender.join().unwrap();
        barrier.join().unwrap();
        assert_eq!(super::test_callback_count(), count + 1);
        drop(app);
    }

    #[cfg(feature = "native-testing")]
    #[test]
    #[ignore = "requires an isolated graphical and D-Bus session"]
    fn paused_owner_records_and_drains_callbacks() {
        use std::sync::Arc;
        use tauri::Manager;

        let gate = Arc::new(crate::linux_activity::ActivityGate::default());
        let app = tauri::Builder::default()
            .any_thread()
            .manage(gate.clone())
            .plugin(super::plugin())
            .build(tauri::generate_context!())
            .unwrap();
        let (queue, path) = crate::linux_handoff_callbacks::CallbackQueue::create_private().unwrap();
        let queue = Arc::new(queue);
        let owner = app.state::<super::InstanceOwner>();
        let pause = gate.begin_pause_with(|| owner.install_callback_queue(queue.clone())).unwrap();
        tauri::async_runtime::block_on(pause.wait_drained());
        let count = super::test_callback_count();
        let connection = zbus::blocking::Connection::session().unwrap();
        let (_, object_path) = super::address(&app.config().identifier);
        let legacy = connection.call_method(Some(owner.name.as_str()), object_path.as_str(),
            Some("org.SingleInstance.DBus"), "ExecuteCallback", &(Vec::<String>::new(), String::new()));
        assert!(legacy.unwrap_err().to_string().contains("Legacy shortcut callbacks are unsupported"));
        let wrong = std::fs::File::open("/dev/null").unwrap();
        let forged = connection.call_method(Some(owner.name.as_str()), object_path.as_str(),
            Some("org.SingleInstance.DBus"), "ExecuteCallbackGuarded",
            &(Vec::<String>::new(), String::new(), zbus::zvariant::Fd::from(&wrong)));
        assert!(forged.is_err());
        super::test_callback(&connection, &owner.name, &object_path,
            vec!["--verenu-hotkey-release".into()]).unwrap();
        assert_eq!(super::test_callback_count(), count);
        assert_eq!(queue.first().unwrap().unwrap().1, crate::linux_handoff_callbacks::Callback::Release);
        owner.callback_barrier().unwrap();
        for _ in 0..4095 { queue.push(crate::linux_handoff_callbacks::Callback::Show).unwrap(); }
        assert!(super::test_callback(&connection, &owner.name, &object_path,
            vec!["--verenu-hotkey-release".into()]).is_err());
        assert!(owner.callback_barrier().is_err());
        assert_eq!(queue.first().unwrap().unwrap().1, crate::linux_handoff_callbacks::Callback::Release);
        *owner.route.queue.lock().unwrap() = None;
        assert!(gate.try_enter().is_none());
        drop(pause);
        drop(owner);
        drop(app);
        drop(queue);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }


    #[test]
    fn address_matches_existing_single_instance_protocol() {
        assert_eq!(
            super::address("com.verenu.app"),
            (
                "com.verenu.app.SingleInstance".into(),
                "/com/verenu/app/SingleInstance".into()
            )
        );
        assert_eq!(
            super::address("com.verenu.my-app").1,
            "/com/verenu/my_app/SingleInstance"
        );
    }

    #[cfg(feature = "native-testing")]
    #[test]
    fn bus_restores_previous_owner_when_replacement_disconnects() {
        use std::os::unix::process::CommandExt;
        use zbus::fdo::{RequestNameFlags, RequestNameReply};
        let name = format!("com.verenu.TransferTest.x{}", uuid::Uuid::new_v4().simple());
        let original = zbus::blocking::connection::Builder::session()
            .unwrap()
            .name(name.as_str())
            .unwrap()
            .replace_existing_names(false)
            .allow_name_replacements(false)
            .build()
            .unwrap();
        let original = super::InstanceOwner {
            connection: original,
            name: name.clone(),
            route: std::sync::Arc::new(super::CallbackRoute::new(false)),
        };
        let replacement = zbus::blocking::Connection::session().unwrap();
        assert_eq!(
            replacement
                .request_name_with_flags(
                    name.as_str(),
                    enumflags2::BitFlags::empty(),
                )
                .unwrap(),
            RequestNameReply::InQueue
        );
        let mut expected = std::process::Command::new("/bin/sleep")
            .arg("5").process_group(0).spawn().unwrap();
        assert!(original.verify_queued_replacement(expected.id()).is_err());
        expected.kill().unwrap();
        expected.wait().unwrap();
        replacement.release_name(name.as_str()).unwrap();
        let actual = original
            .connection
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetNameOwner",
                &(name.as_str(),),
            )
            .unwrap()
            .body()
            .deserialize::<String>()
            .unwrap();
        assert_eq!(actual, original.connection.unique_name().unwrap().as_str());
        assert!(replacement
            .request_name_with_flags(
                name.as_str(),
                RequestNameFlags::ReplaceExisting | RequestNameFlags::DoNotQueue,
            )
            .is_err());
        assert_eq!(replacement.request_name_with_flags(name.as_str(), enumflags2::BitFlags::empty()).unwrap(), RequestNameReply::InQueue);
        original.connection.release_name(name.as_str()).unwrap();
        let current: String = replacement.call_method(
            Some("org.freedesktop.DBus"), "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"), "GetNameOwner", &(name.as_str(),),
        ).unwrap().body().deserialize().unwrap();
        assert_eq!(current, replacement.unique_name().unwrap().as_str());
    }

    #[cfg(feature = "native-testing")]
    #[test]
    #[ignore = "subprocess fixture"]
    fn replacement_queue_fixture() {
        use std::io::Read;
        let Ok(name) = std::env::var("VERENU_TEST_TRANSFER_NAME") else { return };
        if std::env::var_os("VERENU_TEST_TRANSFER_WAIT").is_some() {
            std::io::stdin().read_exact(&mut [0]).unwrap();
        }
        let connection = zbus::blocking::Connection::session().unwrap();
        assert_eq!(
            connection.request_name_with_flags(name.as_str(), enumflags2::BitFlags::empty()).unwrap(),
            zbus::fdo::RequestNameReply::InQueue
        );
        if std::env::var_os("VERENU_TEST_TRANSFER_QUEUE").is_some() {
            let path = format!("/{}", name.replace('.', "/").replace('-', "_"));
            super::test_callback(&connection, &name, &path,
                vec!["--verenu-hotkey-handsfree".into()]).unwrap();
        }
        eprintln!("replacement-acquired");
        let _ = std::io::stdin().read(&mut [0]);
    }
}
