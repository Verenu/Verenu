//! Linux installation ownership, verified staging, and native update operations.
use sha2::{Digest, Sha256};
use std::io::Read;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

const MAX_DOWNLOAD: u64 = 1024 * 1024 * 1024;
const PACMAN: &str = "/usr/bin/pacman";
const PKEXEC: &str = "/usr/bin/pkexec";

#[derive(Debug, PartialEq, Eq)]
pub enum Installation {
    Arch { executable: PathBuf },
    AppImage { image: PathBuf },
    Manual,
}

pub fn installation() -> Installation {
    let Ok(executable) = std::env::current_exe() else {
        return Installation::Manual;
    };
    detect_installation(
        &executable,
        std::env::var_os("APPIMAGE").as_deref().map(Path::new),
        package_owner,
    )
}

fn package_owner(path: &Path) -> Option<String> {
    let output = Command::new(PACMAN)
        .args(["-Qqo", "--"])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn detect_installation(
    executable: &Path,
    image: Option<&Path>,
    owner: impl Fn(&Path) -> Option<String>,
) -> Installation {
    if let Some(package) = owner(executable) {
        return if package == "verenu" {
            Installation::Arch {
                executable: executable.into(),
            }
        } else {
            Installation::Manual
        };
    }
    if let Some(image) = image
        .filter(|image| image.is_absolute())
        .and_then(|image| image.canonicalize().ok())
    {
        // Never replace a package-owned AppImage directly, including AUR packages.
        if let Some(package) = owner(&image) {
            return if package == "verenu" {
                Installation::Arch {
                    executable: executable.into(),
                }
            } else {
                Installation::Manual
            };
        }
        if validate_appimage(&image).is_ok() {
            return Installation::AppImage { image };
        }
    }
    Installation::Manual
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    phase: &'static str,
    downloaded: u64,
    total: Option<u64>,
}

fn progress(app: &AppHandle, phase: &'static str, downloaded: u64, total: Option<u64>) {
    let _ = app.emit(
        "verenu:update-progress",
        Progress {
            phase,
            downloaded,
            total,
        },
    );
}

/// Owned temporary files disappear after every error, cancellation, or success.
struct StagingDirectory(PathBuf);
impl StagingDirectory {
    fn new(parent: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let path = parent.join(format!("verenu-update-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|e| e.to_string())?;
        Ok(Self(path))
    }
}
impl Drop for StagingDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn download_file(
    url: &str,
    destination: &Path,
    expected: &str,
    mut report: impl FnMut(&'static str, u64, Option<u64>),
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(std::time::Duration::from_secs(60))
        .timeout(std::time::Duration::from_secs(30 * 60))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(url)
        .header("User-Agent", "verenu")
        .send()
        .await
        .map_err(|error| crate::api::updater::request_error_message(&error))?
        .error_for_status()
        .map_err(|error| crate::api::updater::request_error_message(&error))?;
    let total = response.content_length();
    if total.is_some_and(|size| size > MAX_DOWNLOAD) {
        return Err("The update exceeds the 1 GiB download limit.".into());
    }
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)
        .await
        .map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut downloaded = 0;
    let mut last_event = std::time::Instant::now();
    report("downloading", 0, total);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| crate::api::updater::request_error_message(&error))?
    {
        downloaded += chunk.len() as u64;
        if downloaded > MAX_DOWNLOAD {
            return Err("The update exceeds the 1 GiB download limit.".into());
        }
        hash.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Could not save the update. Check free disk space: {e}"))?;
        if last_event.elapsed() >= std::time::Duration::from_millis(250) {
            report("downloading", downloaded, total);
            last_event = std::time::Instant::now();
        }
    }
    report("verifying", downloaded, total);
    if downloaded == 0
        || total.is_some_and(|size| size != downloaded)
        || format!("{:x}", hash.finalize()) != expected
    {
        return Err("The downloaded update failed SHA256 verification. Your installation has not changed. Check for updates and try again.".into());
    }
    file.sync_all().await.map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn install(
    app: &AppHandle,
    url: &str,
    channel: crate::api::updater::UpdateChannel,
) -> Result<(), String> {
    progress(app, "resolving", 0, None);
    let mode = tokio::task::spawn_blocking(installation)
        .await
        .map_err(|e| e.to_string())?;
    if mode == Installation::Manual {
        use tauri_plugin_shell::ShellExt;
        #[allow(deprecated)]
        app.shell().open(url, None).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let (name, hash) = crate::api::updater::resolve_linux_download(url, channel)
        .await
        .map_err(|e| e.to_string())?;
    // Validate metadata before turning its name into a local path.
    if Path::new(&name).file_name().and_then(|name| name.to_str()) != Some(&name) {
        return Err("Invalid release installer filename.".into());
    }
    // Pacman's downloader may drop privileges to the alpm user. Release
    // packages must be reachable outside a private home directory after
    // verification; no app data or credentials are staged here.
    let cache = if matches!(mode, Installation::Arch { .. }) {
        PathBuf::from("/var/tmp")
    } else {
        app.path().app_cache_dir().map_err(|e| e.to_string())?
    };
    let staging = StagingDirectory::new(&cache)?;
    let installer = staging.0.join(&name);
    download_file(url, &installer, &hash, |phase, downloaded, total| {
        progress(app, phase, downloaded, total)
    })
    .await?;
    progress(app, "backing-up", 0, None);
    let db = app.state::<crate::DbHandle>().inner().clone();
    let app = app.clone();
    tokio::task::spawn_blocking(move || {
        // The backup is mandatory. A failed backup never proceeds to install.
        let conn = db
            .lock()
            .map_err(|_| "Could not lock the database for the update backup.".to_owned())?;
        crate::commands::backup_sqlite_database(
            &conn,
            &crate::app_db_path().with_extension("db.bak"),
        )
        .map_err(|e| format!("Could not back up Verenu data. Installation was stopped: {e}"))?;
        drop(conn);
        // Re-check ownership after the download, in case an external manager updated it.
        if installation() != mode {
            return Err(
                "Verenu's installation changed during the download. Check for updates again."
                    .into(),
            );
        }
        match mode {
            Installation::Arch { .. } => {
                validate_arch_package(&installer)?;
                std::fs::set_permissions(&installer, std::fs::Permissions::from_mode(0o644))
                    .map_err(|e| e.to_string())?;
                std::fs::set_permissions(&staging.0, std::fs::Permissions::from_mode(0o755))
                    .map_err(|e| e.to_string())?;
                progress(&app, "authorizing", 0, None);
                apply_arch_package(&installer)?;
            }
            Installation::AppImage { image } => {
                progress(&app, "installing", 0, None);
                replace_appimage(&image, &installer)?;
            }
            Installation::Manual => unreachable!(),
        }
        progress(&app, "complete", 0, None);
        drop(staging);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn validate_arch_package(path: &Path) -> Result<(), String> {
    let output = Command::new(PACMAN)
        .args(["-Qpq", "--"])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Could not inspect the Arch package: {e}"))?;
    if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim() != "verenu" {
        return Err(
            "The downloaded file is not a valid Verenu pacman package. Installation was blocked."
                .into(),
        );
    }
    Ok(())
}

fn apply_arch_package(path: &Path) -> Result<(), String> {
    // Absolute program paths and separate arguments avoid shell interpolation.
    // Do not disable package signatures, dependency checks, or database locks.
    // -U never refreshes sync databases or performs a partial system upgrade.
    let output = Command::new(PKEXEC)
        .args(pkexec_pacman_args(path))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Could not start administrator authorization: {e}. Install polkit and a desktop authentication agent, or update with your package manager."))?;
    package_result(
        output.status.code(),
        &String::from_utf8_lossy(&output.stderr),
    )
}

fn pkexec_pacman_args(path: &Path) -> Vec<std::ffi::OsString> {
    let mut args = [
        "--disable-internal-agent",
        "/usr/bin/env",
        "LC_ALL=C",
        PACMAN,
        "-U",
        "--noconfirm",
        "--",
    ]
    .into_iter()
    .map(std::ffi::OsString::from)
    .collect::<Vec<_>>();
    args.push(path.as_os_str().to_owned());
    args
}

fn package_result(code: Option<i32>, stderr: &str) -> Result<(), String> {
    match code {
        Some(0) => Ok(()),
        Some(126) => Err("Update authorization was cancelled. Verenu is still running; you can try again.".into()),
        Some(127) => Err("Administrator authorization was unavailable. Make sure a polkit authentication agent is running, or update Verenu with your package manager.".into()),
        _ if stderr.contains("unable to lock database") => Err("Pacman is busy with another installation. Wait for it to finish, then try again. Do not remove its lock file.".into()),
        _ if stderr.contains("signature") => Err("Pacman rejected the package signature. Update Verenu with your usual package manager; Verenu will not bypass signature policy.".into()),
        _ if stderr.contains("dependencies") || stderr.contains("could not satisfy") => Err("Pacman could not satisfy the update's dependencies. Complete a full system update with your package manager, then try again.".into()),
        _ => Err("Pacman could not install the update. Check disk space and update Verenu with your package manager. The running app has not been restarted.".into()),
    }
}

fn validate_appimage(path: &Path) -> Result<(), String> {
    let mut header = [0; 20];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|e| e.to_string())?;
    if &header[..4] != b"\x7fELF"
        || header[4] != 2
        || header[5] != 1
        || &header[8..11] != b"AI\x02"
        || header[18..20] != [62, 0]
    {
        return Err(
            "The downloaded file is not an x86_64 AppImage. Installation was blocked.".into(),
        );
    }
    Ok(())
}

fn replace_appimage(image: &Path, installer: &Path) -> Result<(), String> {
    replace_appimage_with_directory_sync(image, installer, |parent| {
        std::fs::File::open(parent).and_then(|file| file.sync_all())
    })
}

fn replace_appimage_with_directory_sync(
    image: &Path,
    installer: &Path,
    sync_directory: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<(), String> {
    validate_appimage(installer)?;
    let parent = image
        .parent()
        .ok_or("The AppImage has no parent directory.")?;
    let original = std::fs::symlink_metadata(image).map_err(|e| e.to_string())?;
    if !original.is_file() {
        return Err("The original AppImage is no longer a regular file.".into());
    }
    let staging = StagingDirectory::new(parent).map_err(|e| format!("The AppImage folder is not writable. Move Verenu to a folder you own or download the update manually: {e}"))?;
    let next = staging.0.join("next.AppImage");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&next)
        .map_err(|e| e.to_string())?;
    std::io::copy(
        &mut std::fs::File::open(installer).map_err(|e| e.to_string())?,
        &mut file,
    )
    .map_err(|e| format!("Could not stage the AppImage. Check disk space: {e}"))?;
    file.set_permissions(std::fs::Permissions::from_mode(
        original.mode() & 0o777 | 0o100,
    ))
    .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    let previous = staging.0.join("previous.AppImage");
    std::fs::hard_link(image, &previous)
        .map_err(|e| format!("Could not preserve the previous AppImage: {e}"))?;
    let current = std::fs::symlink_metadata(image).map_err(|e| e.to_string())?;
    if !current.is_file()
        || (
            current.dev(),
            current.ino(),
            current.len(),
            current.mtime(),
            current.mtime_nsec(),
        ) != (
            original.dev(),
            original.ino(),
            original.len(),
            original.mtime(),
            original.mtime_nsec(),
        )
    {
        return Err(
            "The AppImage changed during the update. Installation was stopped; try again.".into(),
        );
    }
    let mut backup_name = image.as_os_str().to_owned();
    backup_name.push(".previous");
    std::fs::rename(&previous, PathBuf::from(backup_name))
        .map_err(|e| format!("Could not retain the previous AppImage: {e}"))?;
    // Same-filesystem rename is atomic. Every earlier error leaves the old image intact.
    std::fs::rename(&next, image).map_err(|e| format!("Could not replace the AppImage: {e}"))?;
    if let Err(error) = sync_directory(parent) {
        log::warn!("AppImage replacement committed, but its directory sync failed: {error}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(parent: &Path, name: &str, suffix: &[u8]) -> PathBuf {
        let mut bytes = vec![0; 20];
        bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
        bytes[8..11].copy_from_slice(b"AI\x02");
        bytes[18] = 62;
        bytes.extend_from_slice(suffix);
        let path = parent.join(name);
        std::fs::write(&path, bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
    #[test]
    fn appimage_replacement_is_atomic_preserves_alias_and_backup() {
        let dir = StagingDirectory::new(&std::env::temp_dir()).unwrap();
        let image = fixture(&dir.0, "Verenu with spaces.AppImage", b"old");
        let downloaded = std::env::var_os("VERENU_UPDATE_TEST_APPIMAGE")
            .map(PathBuf::from)
            .unwrap_or_else(|| fixture(&dir.0, "download.AppImage", b"new"));
        let alias = dir.0.join("Verenu.AppImage");
        std::os::unix::fs::symlink(&image, &alias).unwrap();
        let original = std::fs::read(&image).unwrap();
        let running = std::fs::File::open(&image).unwrap();
        replace_appimage(&alias.canonicalize().unwrap(), &downloaded).unwrap();
        assert_eq!(
            std::fs::read(&alias).unwrap(),
            std::fs::read(&downloaded).unwrap()
        );
        assert_eq!(
            std::fs::read(dir.0.join("Verenu with spaces.AppImage.previous")).unwrap(),
            original
        );
        let mut still_running = Vec::new();
        std::io::BufReader::new(running)
            .read_to_end(&mut still_running)
            .unwrap();
        assert_eq!(still_running, original);
        assert!(std::fs::symlink_metadata(&alias).unwrap().is_symlink());
        assert_eq!(std::fs::metadata(&image).unwrap().mode() & 0o777, 0o755);
    }
    #[test]
    fn invalid_download_does_not_touch_original_or_create_backup() {
        let dir = StagingDirectory::new(&std::env::temp_dir()).unwrap();
        let image = fixture(&dir.0, "Verenu.AppImage", b"old");
        let bad = dir.0.join("bad.AppImage");
        std::fs::write(&bad, b"not an AppImage").unwrap();
        let original = std::fs::read(&image).unwrap();
        assert!(replace_appimage(&image, &bad).is_err());
        assert_eq!(std::fs::read(&image).unwrap(), original);
        assert!(!dir.0.join("Verenu.AppImage.previous").exists());
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 2);
    }
    #[test]
    fn package_ownership_takes_priority_over_appimage_environment() {
        let dir = StagingDirectory::new(&std::env::temp_dir()).unwrap();
        let image = fixture(&dir.0, "Verenu.AppImage", b"old");
        let exe = Path::new("/opt/verenu/usr/bin/verenu");
        assert!(matches!(
            detect_installation(exe, Some(&image), |_| Some("verenu".into())),
            Installation::Arch { .. }
        ));
        assert_eq!(
            detect_installation(exe, Some(&image), |_| Some("verenu-bin".into())),
            Installation::Manual
        );
        assert_eq!(
            detect_installation(exe, Some(&image), |path| (path == image)
                .then_some("verenu-bin".into())),
            Installation::Manual
        );
        assert_eq!(
            detect_installation(exe, Some(&image), |_| None),
            Installation::AppImage { image }
        );
        assert_eq!(
            detect_installation(exe, None, |_| None),
            Installation::Manual
        );
    }
    #[test]
    fn pacman_failures_are_actionable_and_never_successful() {
        assert!(package_result(Some(0), "").is_ok());
        for (code, stderr, expected) in [
            (126, "", "cancelled"),
            (127, "", "polkit"),
            (1, "unable to lock database", "busy"),
            (1, "invalid signature", "signature"),
            (1, "could not satisfy dependencies", "full system update"),
            (1, "disk full", "disk space"),
        ] {
            assert!(package_result(Some(code), stderr)
                .unwrap_err()
                .contains(expected));
        }
        assert!(package_result(None, "").is_err());
    }

    #[test]
    fn pacman_runs_with_c_locale_after_pkexec_clears_environment() {
        let args = pkexec_pacman_args(Path::new("/var/tmp/verenu package.pkg.tar.zst"));
        let args: Vec<_> = args
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "--disable-internal-agent",
                "/usr/bin/env",
                "LC_ALL=C",
                PACMAN,
                "-U",
                "--noconfirm",
                "--",
                "/var/tmp/verenu package.pkg.tar.zst",
            ]
        );
    }

    #[test]
    fn appimage_replacement_succeeds_if_directory_sync_is_unsupported() {
        let dir = StagingDirectory::new(&std::env::temp_dir()).unwrap();
        let image = fixture(&dir.0, "Verenu.AppImage", b"old");
        let downloaded = fixture(&dir.0, "download.AppImage", b"new");
        let original = std::fs::read(&image).unwrap();

        replace_appimage_with_directory_sync(&image, &downloaded, |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "directory sync unsupported",
            ))
        })
        .unwrap();

        assert_eq!(
            std::fs::read(&image).unwrap(),
            std::fs::read(&downloaded).unwrap()
        );
        assert_eq!(
            std::fs::read(dir.0.join("Verenu.AppImage.previous")).unwrap(),
            original
        );
    }

    #[test]
    #[cfg(feature = "native-testing")]
    fn pacman_inspects_real_archives_and_rejects_other_packages() {
        let dir = StagingDirectory::new(&std::env::temp_dir()).unwrap();
        for name in ["verenu", "unrelated"] {
            let package = dir.0.join(format!("{name}.pkg.tar"));
            let mut builder = tar::Builder::new(std::fs::File::create(&package).unwrap());
            let info = format!("pkgname = {name}\npkgver = 0.21.0-1\npkgdesc = Synthetic public fixture\nurl = https://verenu.com\nbuilddate = 1\npackager = Verenu fixture\nsize = 0\narch = x86_64\nlicense = MIT\n");
            let mut header = tar::Header::new_gnu();
            header.set_size(info.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, ".PKGINFO", info.as_bytes())
                .unwrap();
            builder.finish().unwrap();
            drop(builder);
            assert_eq!(validate_arch_package(&package).is_ok(), name == "verenu");
        }
        if let Some(package) = std::env::var_os("VERENU_UPDATE_TEST_PACKAGE") {
            validate_arch_package(Path::new(&package))
                .expect("downloaded official release package");
        }
    }

    fn http_fixture(response: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/installer", server.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).unwrap();
            std::io::Write::write_all(&mut socket, &response).unwrap();
        });
        (url, worker)
    }

    #[tokio::test]
    async fn download_streams_and_verifies_real_http_bytes() {
        let dir = StagingDirectory::new(&std::env::temp_dir()).unwrap();
        let body = b"synthetic public update bytes";
        let hash = format!("{:x}", Sha256::digest(body));
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        let (url, worker) = http_fixture(response.clone());
        let target = dir.0.join("verified");
        let mut phases = Vec::new();
        download_file(&url, &target, &hash, |phase, _, _| phases.push(phase))
            .await
            .unwrap();
        worker.join().unwrap();
        assert_eq!(std::fs::read(target).unwrap(), body);
        assert_eq!(phases, ["downloading", "verifying"]);
        let (url, worker) = http_fixture(response);
        assert!(
            download_file(&url, &dir.0.join("corrupt"), &"0".repeat(64), |_, _, _| {})
                .await
                .unwrap_err()
                .contains("SHA256")
        );
        worker.join().unwrap();
    }

    #[tokio::test]
    async fn interrupted_and_oversized_downloads_fail_before_installation() {
        let dir = StagingDirectory::new(&std::env::temp_dir()).unwrap();
        for (name, response) in [
            (
                "interrupted",
                b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort"
                    .to_vec(),
            ),
            (
                "oversized",
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    MAX_DOWNLOAD + 1
                )
                .into_bytes(),
            ),
            (
                "not-found",
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_vec(),
            ),
        ] {
            let (url, worker) = http_fixture(response);
            assert!(
                download_file(&url, &dir.0.join(name), &"0".repeat(64), |_, _, _| {})
                    .await
                    .is_err()
            );
            worker.join().unwrap();
        }
        drop(dir);
    }
}
