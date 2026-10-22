use ksni::{Tray, TrayMethods, menu::StandardItem};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
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
}

impl LinuxTray {
    fn apply_icon_update(&mut self, revision: u64, rgba: Vec<u8>, size: u32) {
        if self.icon_revision.load(Ordering::SeqCst) == revision {
            self.icon = icon(rgba, size);
        }
    }
}

impl Tray for LinuxTray {
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
        (self.dispatch)(Action::Open);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let item = |label: &str, action| {
            StandardItem {
                label: label.into(),
                activate: Box::new(move |tray: &mut Self| (tray.dispatch)(action)),
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

pub(super) fn setup(app: &mut tauri::App) -> tauri::Result<()> {
    let owner = app.handle().clone();
    let theme = super::resolve_icon_theme(&owner, None);
    let image = super::runtime_tray_icon_image(theme, super::runtime_icon_foreground(theme), 32);
    let icon_revision = Arc::new(AtomicU64::new(0));
    app.manage(IconRevision(icon_revision.clone()));
    let tray = LinuxTray {
        icon: icon(image.rgba().to_vec(), 32),
        icon_revision,
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
    };
    // Login startup can precede the shell's watcher; reconnect when it appears.
    let tray_owner = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        match tray.assume_sni_available(true).spawn().await {
            Ok(handle) => {
                tray_owner.manage(handle);
            }
            Err(error) => log::warn!("Failed to register Linux tray: {error}"),
        }
    });
    Ok(())
}

#[derive(Clone)]
struct IconRevision(Arc<AtomicU64>);

pub(super) fn update_icon(app: &AppHandle, rgba: Vec<u8>, size: u32) {
    if let (Some(handle), Some(revision)) = (
        app.try_state::<ksni::Handle<LinuxTray>>(),
        app.try_state::<IconRevision>(),
    ) {
        let handle = handle.inner().clone();
        let revision = revision.0.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
        tauri::async_runtime::spawn(async move {
            handle
                .update(move |tray| tray.apply_icon_update(revision, rgba, size))
                .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn identity_click_and_menu_contract() {
        let actions = Arc::new(Mutex::new(Vec::new()));
        let recorded = actions.clone();
        let mut tray = LinuxTray {
            icon: icon(vec![10, 20, 30, 40], 1),
            icon_revision: Arc::new(AtomicU64::new(0)),
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
            dispatch: Box::new(|_| {}),
        };

        tray.apply_icon_update(1, vec![50, 60, 70, 80], 1);
        assert_eq!(tray.icon.data, [40, 10, 20, 30]);

        tray.apply_icon_update(2, vec![50, 60, 70, 80], 1);
        assert_eq!(tray.icon.data, [80, 50, 60, 70]);
    }
}
