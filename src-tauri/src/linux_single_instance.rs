use tauri::{AppHandle, Manager};
use zbus::{blocking::Connection, interface};

struct SingleInstance {
    app: AppHandle,
}

#[interface(name = "org.SingleInstance.DBus")]
impl SingleInstance {
    fn execute_callback(&self, argv: Vec<String>, _cwd: String) {
        if argv.iter().any(|arg| arg == "--verenu-hotkey-release") {
            crate::core::hotkey::notify_release();
        } else if argv.iter().any(|arg| arg == "--verenu-hotkey-handsfree") {
            crate::core::hotkey::notify_handless();
        } else if argv.iter().any(|arg| arg == "--verenu-capture-sub-app") {
            crate::core::hotkey::notify_capture_sub_app();
        } else {
            crate::show_main_window(&self.app);
        }
    }
}

pub(crate) struct InstanceOwner {
    connection: Connection,
    name: String,
}

#[cfg(test)]
enum OwnershipMode {
    Transferable,
    Exclusive,
}

#[cfg(test)]
impl InstanceOwner {
    fn set_ownership_mode(&self, mode: OwnershipMode) -> Result<(), String> {
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

fn address(identifier: &str) -> (String, String) {
    let name = format!("{identifier}.SingleInstance");
    let path = format!("/{}", name.replace('.', "/").replace('-', "_"));
    (name, path)
}

pub(crate) fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("single-instance")
        .setup(|app, _api| {
            let (name, path) = address(&app.config().identifier);
            let result = zbus::blocking::connection::Builder::session()?
                .name(name.as_str())?
                .replace_existing_names(false)
                .allow_name_replacements(false)
                .serve_at(path.as_str(), SingleInstance { app: app.clone() })?
                .build();
            match result {
                Ok(connection) => {
                    app.manage(InstanceOwner { connection, name });
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
        let original = super::InstanceOwner { connection, name };
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
