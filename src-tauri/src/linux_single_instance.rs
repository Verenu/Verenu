use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};
use zbus::{blocking::Connection, interface};

struct CallbackRoute {
    parent: Option<String>,
    local: tokio::sync::watch::Sender<bool>,
    order: Arc<CallbackOrder>,
    queue: Mutex<Option<CallbackQueueSlot>>,
}

struct CallbackQueueSlot {
    queue: Arc<crate::linux_handoff_callbacks::CallbackQueue>,
    delivered: Option<i64>,
    failed: bool,
}

#[derive(Default)]
struct CallbackOrder {
    state: Mutex<CallbackOrderState>,
    changed: tokio::sync::Notify,
}

#[derive(Default)]
struct CallbackOrderState {
    issued: u64,
    next: u64,
    completed: BTreeSet<u64>,
}

struct CallbackTicket {
    order: Arc<CallbackOrder>,
    number: u64,
}

impl CallbackOrder {
    fn ticket(self: &Arc<Self>) -> CallbackTicket {
        let mut state = self.state.lock().expect("callback order lock");
        let number = state.issued;
        state.issued = state.issued.checked_add(1).expect("callback counter exhausted");
        CallbackTicket { order: self.clone(), number }
    }
}

impl CallbackTicket {
    async fn wait_turn(&self) {
        loop {
            let notified = self.order.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.order.state.lock().expect("callback order lock").next == self.number {
                return;
            }
            notified.await;
        }
    }
}

impl Drop for CallbackTicket {
    fn drop(&mut self) {
        let mut state = self.order.state.lock().expect("callback order lock");
        state.completed.insert(self.number);
        while {
            let next = state.next;
            state.completed.remove(&next)
        } {
            state.next += 1;
        }
        self.order.changed.notify_waiters();
    }
}

impl CallbackRoute {
    fn new(parent: Option<String>) -> Self {
        let (local, _) = tokio::sync::watch::channel(parent.is_none());
        Self { parent, local, order: Arc::new(CallbackOrder::default()), queue: Mutex::new(None) }
    }

    fn enqueue_if_active(&self, args: &[String]) -> Option<zbus::fdo::Result<()>> {
        let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        queue.as_mut().map(|slot| {
            let callback = crate::linux_handoff_callbacks::Callback::from_args(args);
            let result = if self.parent.is_some() {
                slot.queue.push_replacement(callback)
            } else {
                slot.queue.push(callback)
            };
            result
                .map_err(|error| {
                    slot.failed = true;
                    zbus::fdo::Error::Failed(format!("Could not queue restart callback: {error}"))
                })
        })
    }

    fn activate(&self, _parent_exited: &crate::linux_handoff_transport::ParentExited) {
        self.local.send_replace(true);
    }

    async fn wait_for_local(&self) -> Result<(), zbus::fdo::Error> {
        let mut local = self.local.subscribe();
        while !*local.borrow_and_update() {
            local.changed().await.map_err(|_| {
                zbus::fdo::Error::Failed("Restart callback routing stopped".into())
            })?;
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "native-testing"))]
static TEST_CALLBACK_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(all(test, feature = "native-testing"))]
static TEST_CALLBACK_STARTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(all(test, feature = "native-testing"))]
static TEST_FORWARD_FAILED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

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

    async fn execute_callback(&self, argv: Vec<String>, cwd: String) -> zbus::fdo::Result<()> {
        if self.route.parent.is_some() {
            if let Some(result) = self.route.enqueue_if_active(&argv) {
                return result;
            }
            // A disconnected caller must not release its ticket while the
            // forwarded D-Bus method can still be running in the parent.
            let ticket = self.route.order.ticket();
            let instance = SingleInstance {
                app: self.app.clone(),
                route: self.route.clone(),
            };
            return tauri::async_runtime::spawn(async move {
                instance.dispatch_callback(argv, cwd, Some(ticket)).await
            })
            .await
            .map_err(|_| zbus::fdo::Error::Failed("Restart callback stopped".into()))?;
        }
        self.dispatch_callback(argv, cwd, None).await
    }
}

impl SingleInstance {
    async fn dispatch_callback(
        &self,
        argv: Vec<String>,
        cwd: String,
        ticket: Option<CallbackTicket>,
    ) -> zbus::fdo::Result<()> {
        if let Some(ticket) = ticket.as_ref() {
            ticket.wait_turn().await;
        }
        let local = *self.route.local.borrow();
        if let Some(parent) = self.route.parent.as_ref().filter(|_| !local) {
            let forwarded = async {
                let connection = zbus::Connection::session().await?;
                let (_, path) = address(&self.app.config().identifier);
                connection
                    .call_method(
                        Some(parent.as_str()),
                        path.as_str(),
                        Some("org.SingleInstance.DBus"),
                        "ExecuteCallback",
                        &(argv.clone(), cwd),
                    )
                    .await?;
                Ok::<(), zbus::Error>(())
            }
            .await;
            if forwarded.is_ok() {
                return Ok(());
            }
            #[cfg(all(test, feature = "native-testing"))]
            TEST_FORWARD_FAILED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // During probation the parent is paused. A disconnected forward
            // can run here only after its authenticated exit is confirmed.
            self.route.wait_for_local().await?;
        }
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
    TEST_CALLBACK_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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

#[allow(dead_code)]
pub(crate) enum OwnershipMode {
    Transferable,
    Exclusive,
}

impl InstanceOwner {
    #[allow(dead_code)]
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

    #[allow(dead_code)]
    pub(crate) fn callback_barrier(&self) -> Result<(), String> {
        let unique = self.unique_name()?;
        let path = format!("/{}", self.name.replace('.', "/").replace('-', "_"));
        Connection::session()
            .and_then(|connection| connection.call_method(
                Some(unique.as_str()), path.as_str(), Some("org.SingleInstance.DBus"),
                "HandoffBarrier", &(),
            ))
            .map(|_| ())
            .map_err(|error| format!("Could not finish pending restart callbacks: {error}"))
    }

    #[allow(dead_code)]
    pub(crate) fn drain_callback_queue(&self, app: &AppHandle) -> Result<(), String> {
        let mut slot = self.route.queue.lock().unwrap_or_else(|error| error.into_inner());
        let Some(active) = slot.as_mut() else { return Ok(()); };
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
        *slot = None;
        Ok(())
    }
    #[allow(dead_code)]
    pub(crate) fn activate_callbacks(
        &self,
        app: &AppHandle,
        parent_exited: &crate::linux_handoff_transport::ParentExited,
    ) -> Result<(), String> {
        self.drain_callback_queue(app)?;
        self.route.activate(parent_exited);
        Ok(())
    }
    #[allow(dead_code)]
    pub(crate) fn unique_name(&self) -> Result<String, String> {
        self.connection
            .unique_name()
            .map(ToString::to_string)
            .ok_or_else(|| "Single-instance connection has no unique name".into())
    }
    #[allow(dead_code)]
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

    #[allow(dead_code)]
    pub(crate) fn set_ownership_mode(&self, mode: OwnershipMode) -> Result<(), String> {
        let flags = match mode {
            OwnershipMode::Transferable => zbus::fdo::RequestNameFlags::AllowReplacement as u32,
            OwnershipMode::Exclusive => zbus::fdo::RequestNameFlags::DoNotQueue as u32,
        };
        let reply = self
            .connection
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "RequestName",
                &(self.name.as_str(), flags),
            )
            .and_then(|message| message.body().deserialize::<u32>())
            .map_err(|error| format!("Could not change instance ownership: {error}"))?;
        if reply != 4 {
            return Err("Verenu no longer owns its single-instance name.".into());
        }
        Ok(())
    }
}

#[allow(dead_code)]
pub(crate) fn replacement_plugin(
    parent: String,
    queue: Arc<crate::linux_handoff_callbacks::CallbackQueue>,
) -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("single-instance-replacement")
        .setup(move |app, _api| {
            zbus::names::UniqueName::try_from(parent.as_str())?;
            let (name, path) = address(&app.config().identifier);
            let route = Arc::new(CallbackRoute::new(Some(parent.clone())));
            *route.queue.lock().unwrap_or_else(|error| error.into_inner()) =
                Some(CallbackQueueSlot { queue: queue.clone(), delivered: None, failed: false });
            let connection = zbus::blocking::connection::Builder::session()?
                .name(name.as_str())?
                .replace_existing_names(true)
                .allow_name_replacements(false)
                .serve_at(
                    path.as_str(),
                    SingleInstance {
                        app: app.clone(),
                        route: route.clone(),
                    },
                )?
                .build()?;
            app.manage(InstanceOwner { connection, name, route });
            Ok(())
        })
        .on_event(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(owner) = app.try_state::<InstanceOwner>() {
                    let _ = owner.connection.release_name(owner.name.as_str());
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
            let route = Arc::new(CallbackRoute::new(None));
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
                    if let Ok(connection) = Connection::session() {
                        let _ = connection.call_method(
                            Some(name.as_str()),
                            path.as_str(),
                            Some("org.SingleInstance.DBus"),
                            "ExecuteCallback",
                            &(
                                std::env::args().collect::<Vec<_>>(),
                                std::env::current_dir()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .to_string(),
                            ),
                        );
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
                    let _ = owner.connection.release_name(owner.name.as_str());
                }
            }
        })
        .build()
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "native-testing")]
    struct BlockingForward {
        arrived: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        first_arrived: std::sync::Arc<tokio::sync::Notify>,
        release: std::sync::Arc<tokio::sync::Semaphore>,
    }

    #[cfg(feature = "native-testing")]
    #[zbus::interface(name = "org.SingleInstance.DBus")]
    impl BlockingForward {
        async fn execute_callback(&self, _argv: Vec<String>, _cwd: String) {
            if self.arrived.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                self.first_arrived.notify_one();
                self.release.acquire().await.unwrap().forget();
            }
        }
    }

    #[cfg(feature = "native-testing")]
    #[test]
    #[ignore = "requires an isolated graphical and D-Bus session"]
    fn cancelled_forward_keeps_the_next_callback_waiting() {
        use std::sync::{atomic::{AtomicUsize, Ordering}, Arc};
        use std::time::Duration;

        let app = tauri::Builder::default()
            .any_thread()
            .build(tauri::generate_context!())
            .unwrap();
        tauri::async_runtime::block_on(async {
            let arrived = Arc::new(AtomicUsize::new(0));
            let first_arrived = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Semaphore::new(0));
            let (_, path) = super::address(&app.config().identifier);
            let parent = zbus::connection::Builder::session().unwrap()
                .serve_at(path.as_str(), BlockingForward {
                    arrived: arrived.clone(),
                    first_arrived: first_arrived.clone(),
                    release: release.clone(),
                }).unwrap()
                .build().await.unwrap();
            let instance = Arc::new(super::SingleInstance {
                app: app.handle().clone(),
                route: Arc::new(super::CallbackRoute::new(Some(parent.unique_name().unwrap().to_string()))),
            });
            let first_instance = instance.clone();
            let first = tokio::spawn(async move {
                first_instance.execute_callback(vec!["first".into()], String::new()).await
            });
            tokio::time::timeout(Duration::from_secs(5), first_arrived.notified()).await.unwrap();
            first.abort();
            let _ = first.await;
            let second = tokio::spawn(async move {
                instance.execute_callback(vec!["second".into()], String::new()).await
            });
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert_eq!(arrived.load(Ordering::SeqCst), 1);
            release.add_permits(1);
            tokio::time::timeout(Duration::from_secs(5), second).await.unwrap().unwrap().unwrap();
            assert_eq!(arrived.load(Ordering::SeqCst), 2);
        });
    }

    #[tokio::test]
    async fn callback_replay_keeps_order_and_skips_cancelled_calls() {
        use std::sync::Arc;
        use std::time::Duration;

        let order = Arc::new(super::CallbackOrder::default());
        let first = order.ticket();
        let cancelled = order.ticket();
        let last = order.ticket();
        let waiting = tokio::spawn(async move { last.wait_turn().await });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(cancelled);
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(first);
        tokio::time::timeout(Duration::from_secs(2), waiting).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn disconnected_forward_waits_for_confirmed_parent_exit() {
        use std::sync::Arc;
        use std::time::Duration;

        let route = Arc::new(super::CallbackRoute::new(Some(":1.42".into())));
        let waiting_route = route.clone();
        let waiter = tokio::spawn(async move { waiting_route.wait_for_local().await });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        let parent_exited = crate::linux_handoff_transport::parent_exited_for_test();
        route.activate(&parent_exited);
        tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        route.wait_for_local().await.unwrap();
    }

    #[cfg(feature = "native-testing")]
    #[test]
    #[ignore = "requires an isolated graphical and D-Bus session"]
    fn disconnected_dbus_forward_dispatches_after_confirmed_parent_exit() {
        use std::sync::Arc;
        use std::time::Duration;

        let app = tauri::Builder::default()
            .any_thread()
            .build(tauri::generate_context!())
            .unwrap();
        let route = Arc::new(super::CallbackRoute::new(Some(":1.999999".into())));
        let instance = super::SingleInstance {
            app: app.handle().clone(),
            route: route.clone(),
        };
        let before = super::test_callback_count();
        let failures = super::TEST_FORWARD_FAILED.load(std::sync::atomic::Ordering::SeqCst);
        tauri::async_runtime::block_on(async {
            let call = tokio::spawn(async move {
                instance.execute_callback(vec!["--verenu-hotkey-release".into()], String::new()).await
            });
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while super::TEST_FORWARD_FAILED.load(std::sync::atomic::Ordering::SeqCst) == failures {
                assert!(tokio::time::Instant::now() < deadline, "forward did not fail");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(!call.is_finished());
            assert_eq!(super::test_callback_count(), before);
            let parent_exited = crate::linux_handoff_transport::parent_exited_for_test();
            route.activate(&parent_exited);
            tokio::time::timeout(Duration::from_secs(5), call).await.unwrap().unwrap().unwrap();
            assert_eq!(super::test_callback_count(), before + 1);
        });
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
            let result = connection.call_method(
                Some("com.verenu.app.SingleInstance"),
                "/com/verenu/app/SingleInstance",
                Some("org.SingleInstance.DBus"),
                "ExecuteCallback",
                &(vec!["--verenu-hotkey-release".to_string()], String::new()),
            );
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
        connection.call_method(
            Some(owner.name.as_str()),
            object_path.as_str(),
            Some("org.SingleInstance.DBus"),
            "ExecuteCallback",
            &(vec!["--verenu-hotkey-release".to_string()], String::new()),
        ).unwrap();
        assert_eq!(super::test_callback_count(), count);
        assert_eq!(queue.first().unwrap().unwrap().1, crate::linux_handoff_callbacks::Callback::Release);
        owner.callback_barrier().unwrap();
        let filler = rusqlite::Connection::open(&path).unwrap();
        filler.execute_batch(
            "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x + 1 FROM n WHERE x < 4095) INSERT INTO callbacks(kind) SELECT 3 FROM n;"
        ).unwrap();
        assert!(connection.call_method(
            Some(owner.name.as_str()), object_path.as_str(), Some("org.SingleInstance.DBus"),
            "ExecuteCallback", &(vec!["--verenu-hotkey-release".to_string()], String::new()),
        ).is_err());
        assert!(owner.callback_barrier().is_err());
        filler.execute("DELETE FROM callbacks WHERE kind = 3", []).unwrap();
        drop(filler);
        owner.drain_callback_queue(app.handle()).unwrap();
        assert_eq!(super::test_callback_count(), count + 1);
        assert_eq!(queue.first().unwrap(), None);
        assert!(gate.try_enter().is_none());
        drop(pause);
        drop(owner);
        drop(app);
        drop(queue);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[cfg(feature = "native-testing")]
    #[test]
    #[ignore = "requires an isolated graphical and D-Bus session"]
    fn tauri_instance_regains_ownership_after_replacement_dies() {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        use tauri::Manager;

        let app = tauri::Builder::default()
            .any_thread()
            .plugin(super::plugin())
            .build(tauri::generate_context!())
            .unwrap();
        let owner = app.state::<super::InstanceOwner>();
        owner
            .set_ownership_mode(super::OwnershipMode::Transferable)
            .unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "linux_single_instance::tests::bus_restores_owner_after_replacement_process_dies",
                "--nocapture",
            ])
            .env("VERENU_TEST_TRANSFER_NAME", &owner.name)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stderr = child.stderr.take().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if line == "replacement-acquired" {
                    let _ = sender.send(());
                    break;
                }
            }
        });
        let ready = receiver.recv_timeout(Duration::from_secs(10));
        let _ = child.kill();
        let _ = child.wait();
        ready.unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let current = owner
                .connection
                .call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "GetNameOwner",
                    &(owner.name.as_str(),),
                )
                .and_then(|message| message.body().deserialize::<String>())
                .unwrap();
            if current == owner.connection.unique_name().unwrap().as_str() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Tauri instance did not regain ownership"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        owner
            .set_ownership_mode(super::OwnershipMode::Exclusive)
            .unwrap();
        let connection = zbus::blocking::Connection::session().unwrap();
        let (_, path) = super::address(&app.config().identifier);
        connection
            .call_method(
                Some(owner.name.as_str()),
                path.as_str(),
                Some("org.SingleInstance.DBus"),
                "ExecuteCallback",
                &(Vec::<String>::new(), String::new()),
            )
            .unwrap();
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
            route: std::sync::Arc::new(super::CallbackRoute::new(None)),
        };
        original
            .set_ownership_mode(super::OwnershipMode::Transferable)
            .unwrap();
        let replacement = zbus::blocking::Connection::session().unwrap();
        assert_eq!(
            replacement
                .request_name_with_flags(
                    name.as_str(),
                    RequestNameFlags::ReplaceExisting | RequestNameFlags::DoNotQueue,
                )
                .unwrap(),
            RequestNameReply::PrimaryOwner
        );
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
        original
            .set_ownership_mode(super::OwnershipMode::Exclusive)
            .unwrap();
        assert!(replacement
            .request_name_with_flags(
                name.as_str(),
                RequestNameFlags::ReplaceExisting | RequestNameFlags::DoNotQueue,
            )
            .is_err());
    }

    #[cfg(feature = "native-testing")]
    #[test]
    fn bus_restores_owner_after_replacement_process_dies() {
        use std::io::{BufRead, BufReader, Read};
        use std::process::{Child, Command, Stdio};
        use std::time::{Duration, Instant};
        use zbus::fdo::{RequestNameFlags, RequestNameReply};

        if let Ok(name) = std::env::var("VERENU_TEST_TRANSFER_NAME") {
            if std::env::var_os("VERENU_TEST_TRANSFER_WAIT").is_some() {
                std::io::stdin().read_exact(&mut [0]).unwrap();
            }
            let connection = zbus::blocking::Connection::session().unwrap();
            assert_eq!(
                connection
                    .request_name_with_flags(
                        name.as_str(),
                        RequestNameFlags::ReplaceExisting | RequestNameFlags::DoNotQueue,
                    )
                    .unwrap(),
                RequestNameReply::PrimaryOwner
            );
            if let Some(path) = std::env::var_os("VERENU_TEST_TRANSFER_QUEUE") {
                crate::linux_handoff_callbacks::CallbackQueue::open(std::path::Path::new(&path))
                    .unwrap()
                    .push_replacement(crate::linux_handoff_callbacks::Callback::Handsfree)
                    .unwrap();
            }
            eprintln!("replacement-acquired");
            let _ = std::io::stdin().read(&mut [0]);
            return;
        }

        struct OwnedChild(Child);
        impl Drop for OwnedChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let name = format!("com.verenu.TransferTest.x{}", uuid::Uuid::new_v4().simple());
        let connection = zbus::blocking::connection::Builder::session()
            .unwrap()
            .name(name.as_str())
            .unwrap()
            .replace_existing_names(false)
            .allow_name_replacements(false)
            .build()
            .unwrap();
        let original = super::InstanceOwner {
            connection,
            name,
            route: std::sync::Arc::new(super::CallbackRoute::new(None)),
        };
        original
            .set_ownership_mode(super::OwnershipMode::Transferable)
            .unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "linux_single_instance::tests::bus_restores_owner_after_replacement_process_dies",
                "--nocapture",
            ])
            .env("VERENU_TEST_TRANSFER_NAME", &original.name)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut child = OwnedChild(child);
        let stderr = child.0.stderr.take().unwrap();
        let (ready, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if line == "replacement-acquired" {
                    let _ = ready.send(());
                    break;
                }
            }
        });
        receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let current = original
                .connection
                .call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "GetNameOwner",
                    &(original.name.as_str(),),
                )
                .and_then(|message| message.body().deserialize::<String>())
                .unwrap();
            if current == original.connection.unique_name().unwrap().as_str() {
                break;
            }
            assert!(Instant::now() < deadline, "original owner was not restored");
            std::thread::sleep(Duration::from_millis(20));
        }
        original
            .set_ownership_mode(super::OwnershipMode::Exclusive)
            .unwrap();
    }
}
