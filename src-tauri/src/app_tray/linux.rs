use ksni::{menu::StandardItem, Tray, TrayMethods};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Open,
    Settings,
    Relaunch,
    Quit,
}

struct LinuxTray {
    icon: ksni::Icon,
    icon_revision: Arc<AtomicU64>,
    dispatch: Box<dyn Fn(Action) + Send + Sync>,
    active: bool,
}

impl LinuxTray {
    fn apply_icon_update(&mut self, revision: u64, rgba: Vec<u8>, size: u32) {
        if self.icon_revision.load(Ordering::SeqCst) == revision {
            self.icon = icon(rgba, size);
        }
    }
}

impl Tray for LinuxTray {
    fn status(&self) -> ksni::Status {
        if self.active {
            ksni::Status::Active
        } else {
            ksni::Status::Passive
        }
    }
    fn id(&self) -> String {
        "Verenu".into()
    }

    fn title(&self) -> String {
        "Verenu".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.title(),
            ..Default::default()
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![self.icon.clone()]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        if self.active {
            (self.dispatch)(Action::Open);
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let item = |label: &str, action| {
            StandardItem {
                label: label.into(),
                activate: Box::new(move |tray: &mut Self| {
                    if tray.active {
                        (tray.dispatch)(action);
                    }
                }),
                ..Default::default()
            }
            .into()
        };
        vec![
            item("Open Verenu", Action::Open),
            item("Settings...", Action::Settings),
            ksni::MenuItem::Separator,
            item("Relaunch", Action::Relaunch),
            item("Quit", Action::Quit),
        ]
    }
}

fn icon(mut rgba: Vec<u8>, size: u32) -> ksni::Icon {
    // StatusNotifier pixmaps use network-order ARGB, not Tauri's RGBA.
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.rotate_right(1);
    }
    ksni::Icon {
        width: size as i32,
        height: size as i32,
        data: rgba,
    }
}

pub(super) fn setup(app: &AppHandle) -> tauri::Result<()> {
    setup_with_visibility(app, true)
}

pub(super) fn setup_with_visibility(app: &AppHandle, active: bool) -> tauri::Result<()> {
    app.manage(TrayRuntime {
        icon_revision: Arc::new(AtomicU64::new(0)),
        service: tokio::sync::Mutex::new(TrayService::Stopped),
        desired: AtomicBool::new(true),
        active: AtomicBool::new(active),
    });
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = reconcile(&app).await {
            log::warn!("Failed to register Linux tray: {error}");
        }
    });
    Ok(())
}

struct TrayRuntime {
    icon_revision: Arc<AtomicU64>,
    service: tokio::sync::Mutex<TrayService>,
    desired: AtomicBool,
    active: AtomicBool,
}

enum TrayService {
    Stopped,
    Running(ksni::Handle<LinuxTray>),
    Stopping(ksni::Handle<LinuxTray>),
}

const TRAY_OPERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[allow(dead_code)]
pub(super) async fn start(app: &AppHandle) -> Result<(), String> {
    let runtime = app
        .try_state::<TrayRuntime>()
        .ok_or("Linux tray is unavailable")?;
    runtime.desired.store(true, Ordering::SeqCst);
    reconcile(app).await?;
    if !runtime.desired.load(Ordering::SeqCst) {
        return Err("Linux tray start was superseded".into());
    }
    Ok(())
}

async fn reconcile(app: &AppHandle) -> Result<(), String> {
    let runtime = app
        .try_state::<TrayRuntime>()
        .ok_or("Linux tray is unavailable")?;
    let mut service = runtime.service.lock().await;
    loop {
        let desired = runtime.desired.load(Ordering::SeqCst);
        match &*service {
            TrayService::Running(handle) if desired && !handle.is_closed() => return Ok(()),
            TrayService::Running(handle) if !desired => {
                *service = TrayService::Stopping(handle.clone());
                continue;
            }
            TrayService::Stopping(handle) => {
                tokio::time::timeout(TRAY_OPERATION_TIMEOUT, handle.shutdown())
                    .await
                    .map_err(|_| "Linux tray shutdown timed out".to_string())?;
                *service = TrayService::Stopped;
                continue;
            }
            TrayService::Stopped if !desired => return Ok(()),
            _ => *service = TrayService::Stopped,
        }
        let tray = make_tray(app, runtime.icon_revision.clone());
        // Login startup can precede the shell's watcher; reconnect when it appears.
        let require_watcher = !runtime.active.load(Ordering::SeqCst);
        let handle = tokio::time::timeout(
            TRAY_OPERATION_TIMEOUT,
            tray.assume_sni_available(!require_watcher).spawn(),
        )
        .await
        .map_err(|_| "Linux tray startup timed out".to_string())?
        .map_err(|error| error.to_string())?;
        *service = TrayService::Running(handle);
    }
}

fn make_tray(app: &AppHandle, icon_revision: Arc<AtomicU64>) -> LinuxTray {
    let owner = app.clone();
    let theme = super::resolve_icon_theme(&owner, None);
    let image = super::runtime_tray_icon_image(theme, super::runtime_icon_foreground(theme), 32);
    LinuxTray {
        icon: icon(image.rgba().to_vec(), 32),
        icon_revision,
        active: app.state::<TrayRuntime>().active.load(Ordering::SeqCst),
        dispatch: Box::new(move |action| {
            let app = owner.clone();
            if let Err(error) = owner.run_on_main_thread(move || match action {
                Action::Open => crate::show_main_window(&app),
                Action::Settings => {
                    crate::show_main_window(&app);
                    let _ = app.emit("open-flow:open-settings-section", "general");
                }
                Action::Relaunch => super::relaunch_app(&app),
                Action::Quit => app.exit(0),
            }) {
                log::warn!("Failed to schedule tray action: {error}");
            }
        }),
    }
}

pub(super) async fn verify_probation(app: &AppHandle) -> Result<(), String> {
    let runtime = app.try_state::<TrayRuntime>().ok_or("Linux tray is unavailable")?;
    let service = runtime.service.lock().await;
    if !runtime.desired.load(Ordering::SeqCst) || runtime.active.load(Ordering::SeqCst) {
        return Err("Linux tray is not in restart preparation".into());
    }
    match &*service {
        TrayService::Running(handle) if !handle.is_closed() => Ok(()),
        _ => Err("Linux tray stopped during restart preparation".into()),
    }
}

#[allow(dead_code)]
pub(super) async fn activate_after_handoff(app: &AppHandle) -> Result<(), String> {
    let runtime = app
        .try_state::<TrayRuntime>()
        .ok_or("Linux tray is unavailable")?;
    let service = runtime.service.lock().await;
    let TrayService::Running(handle) = &*service else {
        return Err("Linux tray is not running".into());
    };
    handle
        .update(|tray| tray.active = true)
        .await
        .ok_or("Linux tray stopped during activation")?;
    runtime.active.store(true, Ordering::SeqCst);
    Ok(())
}

#[allow(dead_code)]
pub(super) async fn stop(app: &AppHandle) -> Result<(), String> {
    let runtime = app
        .try_state::<TrayRuntime>()
        .ok_or("Linux tray is unavailable")?;
    runtime.desired.store(false, Ordering::SeqCst);
    reconcile(app).await?;
    if runtime.desired.load(Ordering::SeqCst) {
        return Err("Linux tray stop was superseded".into());
    }
    Ok(())
}

pub(super) fn update_icon(app: &AppHandle, rgba: Vec<u8>, size: u32) {
    if let Some(runtime) = app.try_state::<TrayRuntime>() {
        let revision = runtime
            .icon_revision
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let Some(runtime) = app.try_state::<TrayRuntime>() else {
                return;
            };
            let handle = match &*runtime.service.lock().await {
                TrayService::Running(handle) => Some(handle.clone()),
                _ => None,
            };
            if let Some(handle) = handle {
                handle
                    .update(move |tray| tray.apply_icon_update(revision, rgba, size))
                    .await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[cfg(feature = "native-testing")]
    struct TestWatcher {
        registrations: Arc<Mutex<Vec<String>>>,
        blocked: Arc<(Mutex<bool>, std::sync::Condvar)>,
    }

    #[cfg(feature = "native-testing")]
    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl TestWatcher {
        #[zbus(property)]
        fn is_status_notifier_host_registered(&self) -> bool {
            true
        }

        fn register_status_notifier_item(&self, service: &str) {
            self.registrations.lock().unwrap().push(service.to_owned());
            let (lock, condition) = &*self.blocked;
            drop(
                condition
                    .wait_timeout_while(
                        lock.lock().unwrap(),
                        std::time::Duration::from_secs(30),
                        |blocked| *blocked,
                    )
                    .unwrap(),
            );
        }
    }

    #[cfg(feature = "native-testing")]
    #[test]
    #[ignore = "requires an isolated graphical and D-Bus session"]
    fn probation_tray_requires_a_desktop_watcher() {
        let app = tauri::Builder::default()
            .any_thread()
            .build(tauri::generate_context!())
            .unwrap();
        app.manage(crate::data::store::SettingsHandle::empty_for_test(
            std::env::temp_dir().join(format!("verenu-tray-test-{}", uuid::Uuid::new_v4())),
        ));
        super::setup_with_visibility(app.handle(), false).unwrap();
        tauri::async_runtime::block_on(async {
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(15),
                super::start(app.handle()),
            )
            .await
            .expect("tray start timed out");
            assert!(result.is_err(), "probation succeeded without a watcher");
            super::stop(app.handle()).await.unwrap();
        });
    }

    #[cfg(feature = "native-testing")]
    #[test]
    #[ignore = "requires an isolated graphical and D-Bus session"]
    fn tray_can_stop_and_resume_through_the_app_handle() {
        let registrations = Arc::new(Mutex::new(Vec::<String>::new()));
        let blocked = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let watcher = zbus::blocking::connection::Builder::session()
            .unwrap()
            .name("org.kde.StatusNotifierWatcher")
            .unwrap()
            .serve_at(
                "/StatusNotifierWatcher",
                TestWatcher {
                    registrations: registrations.clone(),
                    blocked: blocked.clone(),
                },
            )
            .unwrap()
            .build()
            .unwrap();
        async fn has_owner(watcher: &zbus::blocking::Connection, name: &str) -> bool {
            let watcher = watcher.clone();
            let name = name.to_owned();
            tokio::task::spawn_blocking(move || {
                watcher
                    .call_method(
                        Some("org.freedesktop.DBus"),
                        "/org/freedesktop/DBus",
                        Some("org.freedesktop.DBus"),
                        "NameHasOwner",
                        &(name,),
                    )
                    .unwrap()
                    .body()
                    .deserialize::<bool>()
                    .unwrap()
            })
            .await
            .unwrap()
        }
        let app = tauri::Builder::default()
            .any_thread()
            .build(tauri::generate_context!())
            .unwrap();
        let settings = crate::data::store::SettingsHandle::empty_for_test(
            std::env::temp_dir().join(format!("verenu-tray-test-{}", uuid::Uuid::new_v4())),
        );
        settings
            .set(
                crate::data::store::APPEARANCE_MODE,
                serde_json::json!("dark"),
            )
            .unwrap();
        app.manage(settings);
        super::setup_with_visibility(app.handle(), false).unwrap();
        tauri::async_runtime::block_on(async {
            let deadline = std::time::Duration::from_secs(15);
            tokio::time::timeout(deadline, super::start(app.handle()))
                .await
                .unwrap()
                .unwrap();
            let item = registrations.lock().unwrap().last().unwrap().clone();
            let read_status = |name: String| {
                let connection = watcher.clone();
                tokio::task::spawn_blocking(move || {
                    zbus::blocking::Proxy::new(
                        &connection,
                        name.as_str(),
                        "/StatusNotifierItem",
                        "org.kde.StatusNotifierItem",
                    )
                    .unwrap()
                    .get_property::<String>("Status")
                    .unwrap()
                })
            };
            assert_eq!(read_status(item.clone()).await.unwrap(), "Passive");
            super::verify_probation(app.handle()).await.unwrap();
            super::activate_after_handoff(app.handle()).await.unwrap();
            assert!(super::verify_probation(app.handle()).await.is_err());
            assert_eq!(read_status(item).await.unwrap(), "Active");
            tokio::time::timeout(deadline, super::stop(app.handle()))
                .await
                .unwrap()
                .unwrap();
            tokio::time::timeout(deadline, super::reconcile(app.handle()))
                .await
                .unwrap()
                .unwrap();
            let runtime = app.state::<super::TrayRuntime>();
            assert!(matches!(
                &*runtime.service.lock().await,
                super::TrayService::Stopped
            ));
            let stopped_names = registrations.lock().unwrap().clone();
            for name in stopped_names {
                assert!(
                    !has_owner(&watcher, &name).await,
                    "tray was recreated after stop"
                );
            }
            let before = registrations.lock().unwrap().len();
            *blocked.0.lock().unwrap() = true;
            let start_app = app.handle().clone();
            let starting = tokio::spawn(async move { super::start(&start_app).await });
            let wait = tokio::time::Instant::now() + deadline;
            while registrations.lock().unwrap().len() == before {
                assert!(
                    tokio::time::Instant::now() < wait,
                    "tray registration did not begin"
                );
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            let stop_app = app.handle().clone();
            let stopping = tokio::spawn(async move { super::stop(&stop_app).await });
            while runtime.desired.load(Ordering::SeqCst) {
                assert!(
                    tokio::time::Instant::now() < wait,
                    "tray stop did not begin"
                );
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            *blocked.0.lock().unwrap() = false;
            blocked.1.notify_all();
            assert!(tokio::time::timeout(deadline, starting)
                .await
                .unwrap()
                .unwrap()
                .is_err());
            tokio::time::timeout(deadline, stopping)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let delayed = registrations.lock().unwrap().last().unwrap().clone();
            assert!(!has_owner(&watcher, &delayed).await);
            tokio::time::timeout(deadline, super::start(app.handle()))
                .await
                .unwrap()
                .unwrap();
            assert!(
                matches!(&*runtime.service.lock().await, super::TrayService::Running(handle) if !handle.is_closed())
            );
            let first = registrations.lock().unwrap().last().unwrap().clone();
            assert!(has_owner(&watcher, &first).await);
            let previous = {
                let mut service = runtime.service.lock().await;
                let super::TrayService::Running(handle) = &*service else {
                    panic!("tray did not start")
                };
                let handle = handle.clone();
                *service = super::TrayService::Stopping(handle.clone());
                handle
            };
            drop(previous.shutdown());
            tokio::time::timeout(deadline, super::start(app.handle()))
                .await
                .unwrap()
                .unwrap();
            assert!(
                matches!(&*runtime.service.lock().await, super::TrayService::Running(handle) if !handle.is_closed())
            );
            assert!(!has_owner(&watcher, &first).await);
            let second = registrations.lock().unwrap().last().unwrap().clone();
            assert_ne!(first, second);
            assert!(has_owner(&watcher, &second).await);
            tokio::time::timeout(deadline, super::stop(app.handle()))
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(
                &*runtime.service.lock().await,
                super::TrayService::Stopped
            ));
            assert!(!has_owner(&watcher, &second).await);
            tokio::time::timeout(deadline, super::start(app.handle()))
                .await
                .unwrap()
                .unwrap();
            assert!(
                matches!(&*runtime.service.lock().await, super::TrayService::Running(handle) if !handle.is_closed())
            );
            let third = registrations.lock().unwrap().last().unwrap().clone();
            assert_ne!(second, third);
            assert!(has_owner(&watcher, &third).await);
            tokio::time::timeout(deadline, super::stop(app.handle()))
                .await
                .unwrap()
                .unwrap();
            assert!(!has_owner(&watcher, &third).await);

            let before = registrations.lock().unwrap().len();
            *blocked.0.lock().unwrap() = true;
            let start_app = app.handle().clone();
            let starting = tokio::spawn(async move { super::start(&start_app).await });
            let wait = tokio::time::Instant::now() + deadline;
            while registrations.lock().unwrap().len() == before {
                assert!(
                    tokio::time::Instant::now() < wait,
                    "tray registration did not begin"
                );
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            let result = tokio::time::timeout(deadline, starting).await;
            *blocked.0.lock().unwrap() = false;
            blocked.1.notify_all();
            assert!(result
                .unwrap()
                .unwrap()
                .unwrap_err()
                .contains("startup timed out"));
            let timed_out = registrations.lock().unwrap().last().unwrap().clone();
            let wait = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            while has_owner(&watcher, &timed_out).await {
                assert!(
                    tokio::time::Instant::now() < wait,
                    "timed-out tray still owns its name"
                );
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            tokio::time::timeout(deadline, super::start(app.handle()))
                .await
                .unwrap()
                .unwrap();
            let recovered = registrations.lock().unwrap().last().unwrap().clone();
            assert_ne!(timed_out, recovered);
            assert!(has_owner(&watcher, &recovered).await);
            tokio::time::timeout(deadline, super::stop(app.handle()))
                .await
                .unwrap()
                .unwrap();
            assert!(!has_owner(&watcher, &recovered).await);
        });
    }

    #[test]
    fn identity_click_and_menu_contract() {
        let actions = Arc::new(Mutex::new(Vec::new()));
        let recorded = actions.clone();
        let mut tray = LinuxTray {
            icon: icon(vec![10, 20, 30, 40], 1),
            icon_revision: Arc::new(AtomicU64::new(0)),
            active: true,
            dispatch: Box::new(move |action| recorded.lock().unwrap().push(action)),
        };
        assert_eq!(tray.id(), "Verenu");
        assert_eq!(tray.title(), "Verenu");
        assert_eq!(tray.tool_tip().title, "Verenu");
        assert!(!<LinuxTray as Tray>::MENU_ON_ACTIVATE);
        tray.activate(10, 20);
        assert_eq!(*actions.lock().unwrap(), [Action::Open]);

        let menu = tray.menu();
        let mut labels = Vec::new();
        for entry in menu {
            match entry {
                ksni::MenuItem::Standard(item) => {
                    labels.push(item.label);
                    (item.activate)(&mut tray);
                }
                ksni::MenuItem::Separator => {}
                _ => panic!("Unexpected tray menu entry"),
            }
        }
        assert_eq!(labels, ["Open Verenu", "Settings...", "Relaunch", "Quit"]);
        assert_eq!(
            *actions.lock().unwrap(),
            [
                Action::Open,
                Action::Open,
                Action::Settings,
                Action::Relaunch,
                Action::Quit
            ]
        );
    }

    #[test]
    fn passive_tray_ignores_actions_until_activated() {
        let actions = Arc::new(Mutex::new(Vec::new()));
        let recorded = actions.clone();
        let mut tray = LinuxTray {
            icon: icon(vec![10, 20, 30, 40], 1),
            icon_revision: Arc::new(AtomicU64::new(0)),
            active: false,
            dispatch: Box::new(move |action| recorded.lock().unwrap().push(action)),
        };
        assert!(matches!(tray.status(), ksni::Status::Passive));
        tray.activate(0, 0);
        for entry in tray.menu() {
            if let ksni::MenuItem::Standard(item) = entry {
                (item.activate)(&mut tray);
            }
        }
        assert!(actions.lock().unwrap().is_empty());
        tray.active = true;
        assert!(matches!(tray.status(), ksni::Status::Active));
        tray.activate(0, 0);
        assert_eq!(*actions.lock().unwrap(), [Action::Open]);
    }

    #[test]
    fn pixmap_preserves_color_and_alpha_in_argb_order() {
        let image = icon(
            vec![
                10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160,
            ],
            2,
        );
        assert_eq!(
            image.data,
            [
                40, 10, 20, 30, 80, 50, 60, 70, 120, 90, 100, 110, 160, 130, 140, 150
            ]
        );
        assert_eq!((image.width, image.height), (2, 2));
    }

    #[test]
    fn stale_icon_updates_do_not_replace_the_latest_icon() {
        let revision = Arc::new(AtomicU64::new(2));
        let mut tray = LinuxTray {
            icon: icon(vec![10, 20, 30, 40], 1),
            icon_revision: revision,
            active: true,
            dispatch: Box::new(|_| {}),
        };

        tray.apply_icon_update(1, vec![50, 60, 70, 80], 1);
        assert_eq!(tray.icon.data, [40, 10, 20, 30]);

        tray.apply_icon_update(2, vec![50, 60, 70, 80], 1);
        assert_eq!(tray.icon.data, [80, 50, 60, 70]);
    }
}
