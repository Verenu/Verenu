#[path = "build_support/foundation_models.rs"]
mod foundation_models;
#[path = "build_support/posthog.rs"]
mod posthog;
#[path = "build_support/speech.rs"]
mod speech;

fn main() {
    foundation_models::build();
    speech::build();
    println!("cargo:rerun-if-env-changed=VERENU_BUILD_FINGERPRINT");
    println!("cargo:rerun-if-env-changed=VERENU_BUILD_FINGERPRINT_FILE");
    if let Ok(fingerprint_file) = std::env::var("VERENU_BUILD_FINGERPRINT_FILE") {
        println!("cargo:rerun-if-changed={fingerprint_file}");
    }
    println!(
        "cargo:rustc-env=VERENU_BUILD_FINGERPRINT={}",
        std::env::var("VERENU_BUILD_FINGERPRINT").unwrap_or_default()
    );
    if (std::env::var_os("CARGO_FEATURE_NATIVE_TESTING").is_some()
        || std::env::var_os("CARGO_FEATURE_ANDROID_LOCAL_TESTING").is_some())
        && std::env::var("PROFILE").as_deref() == Ok("release")
    {
        panic!("native testing features are forbidden in release builds");
    }
    println!("cargo:rerun-if-env-changed=VERENU_REQUIRE_POSTHOG");
    println!("cargo:rerun-if-env-changed=POSTHOG_PROJECT_TOKEN");
    println!("cargo:rerun-if-env-changed=POSTHOG_HOST");
    let require_posthog = match std::env::var("VERENU_REQUIRE_POSTHOG") {
        Ok(value) if value == "1" => true,
        Ok(value) if value == "0" => false,
        Ok(_) => panic!("VERENU_REQUIRE_POSTHOG must be 0 or 1"),
        Err(_) => false,
    };
    let posthog_token = std::env::var("POSTHOG_PROJECT_TOKEN").ok();
    let posthog_host = std::env::var("POSTHOG_HOST").ok();
    if let Err(error) = posthog::validate_release_config(
        require_posthog,
        posthog_token.as_deref(),
        posthog_host.as_deref(),
    ) {
        panic!("{error}");
    }
    if let Some(token) = posthog_token.filter(|value| !value.trim().is_empty()) {
        println!("cargo:rustc-env=VERENU_POSTHOG_PROJECT_TOKEN={}", token);
    }
    if let Some(host) = posthog_host.filter(|value| !value.trim().is_empty()) {
        println!("cargo:rustc-env=VERENU_POSTHOG_HOST={host}");
    }
    // Build scripts themselves run for the host, so `cfg(target_os =
    // "windows")` describes this machine rather than Cargo's Android target.
    // Read CARGO_CFG_TARGET_OS for cross-compiles and keep desktop-only native
    // bridges out of mobile builds.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        #[cfg(target_os = "windows")]
        build_windows_titlebar();
    }

    if std::env::consts::OS == "macos" {
        println!("cargo:rerun-if-changed=src/system/macos_ax_text_marker.m");
        cc::Build::new()
            .file("src/system/macos_ax_text_marker.m")
            .flag("-fobjc-arc")
            .compile("verenu_macos_ax_text_marker");
    }

    if target_os == "android" {
        println!("cargo:rustc-link-arg=-lc++abi");
    }

    println!("cargo:rerun-if-changed=Info.plist");
    println!("cargo:rerun-if-changed=tauri.dev.conf.json");
    println!("cargo:rerun-if-changed=tauri.dev.windows.conf.json");
    println!("cargo:rerun-if-changed=permissions");
    println!("cargo:rerun-if-changed=icons/icon.png");
    println!("cargo:rerun-if-changed=icons/icon.icns");
    println!("cargo:rerun-if-changed=icons/verenu-mark.svg");
    println!("cargo:rerun-if-changed=src/generated_icon_geometry.rs");

    let mut attributes =
        tauri_build::Attributes::new().plugin("verenu-security", tauri_build::InlinedPlugin::new());
    if target_os == "windows" {
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        tauri_build::try_build(attributes).expect("failed to run Tauri build script");
        link_windows_manifest();
    } else {
        tauri_build::try_build(attributes).expect("failed to run Tauri build script");
    }
}

fn link_windows_manifest() {
    use std::{env, fs, path::PathBuf};

    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }

    const MANIFEST: &str = r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"
      />
    </dependentAssembly>
  </dependency>
</assembly>
"#;

    let manifest_path =
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR")).join("verenu.exe.manifest");
    fs::write(&manifest_path, MANIFEST).expect("write Windows application manifest");
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
        manifest_path.display()
    );
}

#[cfg(target_os = "windows")]
fn build_windows_titlebar() {
    use std::{
        env, fs,
        path::{Path, PathBuf},
        process::Command,
    };

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let native = manifest.join("native/windows");
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("aarch64") => "ARM64",
        other => panic!("unsupported Windows architecture for title bar bridge: {other:?}"),
    };
    let configuration = if env::var_os("DEBUG").as_deref() == Some(std::ffi::OsStr::new("true")) {
        "Debug"
    } else {
        "Release"
    };
    let stage = native.join("runtime").join(arch.to_ascii_lowercase());
    let cargo_out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let native_out = cargo_out.join("windows-titlebar");
    let native_obj = cargo_out.join("windows-titlebar-obj");
    fs::create_dir_all(&stage).expect("create Windows title bar runtime directory");

    let vswhere =
        Path::new(r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe");
    let msbuild = if vswhere.exists() {
        let output = Command::new(vswhere)
            .args([
                "-latest",
                "-products",
                "*",
                "-requires",
                "Microsoft.Component.MSBuild",
                "-find",
                r"MSBuild\**\Bin\MSBuild.exe",
            ])
            .output()
            .expect("run vswhere");
        let output = String::from_utf8(output.stdout).expect("vswhere output is UTF-8");
        PathBuf::from(
            output
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .expect("vswhere found no MSBuild"),
        )
    } else {
        PathBuf::from("MSBuild.exe")
    };
    let status = Command::new(msbuild)
        .arg(native.join("Verenu.WindowsChrome.vcxproj"))
        .args(["/restore", "/m", "/v:minimal"])
        .arg(format!("/p:Configuration={configuration}"))
        .arg(format!("/p:Platform={arch}"))
        .arg(format!("/p:OutDir={}\\", native_out.display()))
        .arg(format!("/p:IntDir={}\\", native_obj.display()))
        .status()
        .expect("run MSBuild for Windows title bar bridge");
    assert!(status.success(), "Windows title bar bridge failed to build");

    fn copy_changed(source: &Path, destination: &Path) {
        let unchanged = fs::read(source)
            .ok()
            .zip(fs::read(destination).ok())
            .is_some_and(|(a, b)| a == b);
        if !unchanged {
            fs::copy(source, destination).unwrap_or_else(|error| {
                panic!(
                    "copy {} to {}: {error}",
                    source.display(),
                    destination.display()
                )
            });
        }
    }

    for name in [
        "Verenu.WindowsChrome.dll",
        "Verenu.WindowsChrome.pri",
        "Microsoft.WindowsAppRuntime.Bootstrap.dll",
    ] {
        let source = native_out.join(name);
        if source.exists() {
            copy_changed(&source, &stage.join(name));
        }
    }

    let packages = env::var_os("NUGET_PACKAGES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("USERPROFILE").expect("USERPROFILE is set"))
                .join(".nuget/packages")
        });
    for (package, version) in [
        ("microsoft.windowsappsdk.foundation", "1.8.260505001"),
        (
            "microsoft.windowsappsdk.interactiveexperiences",
            "1.8.260708001",
        ),
    ] {
        let payload = packages
            .join(package)
            .join(version)
            .join("runtimes-framework")
            .join(format!("win-{}", arch.to_ascii_lowercase()))
            .join("native");
        for entry in fs::read_dir(&payload).unwrap_or_else(|error| {
            panic!(
                "read Windows App SDK payload {}: {error}",
                payload.display()
            )
        }) {
            let entry = entry.expect("read Windows App SDK payload entry");
            if entry.file_type().expect("read payload type").is_file() {
                copy_changed(&entry.path(), &stage.join(entry.file_name()));
            }
        }
    }

    let profile = env::var("PROFILE").expect("Cargo profile");
    let target_profile = cargo_out
        .ancestors()
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name == profile.as_str())
        })
        .expect("Cargo profile directory in OUT_DIR")
        .to_path_buf();
    fs::create_dir_all(&target_profile).expect("create Cargo profile directory");
    for entry in fs::read_dir(&stage).expect("read staged title bar runtime") {
        let entry = entry.expect("read staged runtime entry");
        if entry
            .file_type()
            .expect("read staged runtime type")
            .is_file()
        {
            copy_changed(&entry.path(), &target_profile.join(entry.file_name()));
        }
    }

    println!("cargo:rerun-if-changed=native/windows/extended_titlebar.cpp");
    println!("cargo:rerun-if-changed=native/windows/extended_titlebar.h");
    println!("cargo:rerun-if-changed=native/windows/Verenu.WindowsChrome.vcxproj");
}
