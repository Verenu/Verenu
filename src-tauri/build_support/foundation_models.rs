use std::{env, fs, path::PathBuf, process::Command};

fn output(args: &[&str]) -> String {
    let result = Command::new("xcrun")
        .args(args)
        .output()
        .expect("run Xcode tool");
    assert!(
        result.status.success(),
        "Xcode 26 or newer is required to build Apple Intelligence cleanup"
    );
    String::from_utf8(result.stdout)
        .expect("Xcode output")
        .trim()
        .to_owned()
}

pub fn build() {
    println!("cargo:rerun-if-changed=native/macos/FoundationModels.swift");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let sdk = output(&["--sdk", "macosx", "--show-sdk-path"]);
    let swift = output(&["--find", "swiftc"]);
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        _ => panic!("unsupported macOS architecture"),
    };
    let deployment = env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "11.0".into());
    let target = format!("{arch}-apple-macosx{deployment}");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let object = out.join("foundation_models.o");
    let status = Command::new(&swift)
        .args([
            "-emit-object",
            "-parse-as-library",
            "-swift-version",
            "5",
            "-O",
            "-sdk",
            &sdk,
            "-target",
            &target,
            "-Xfrontend",
            "-disable-autolink-framework",
            "-Xfrontend",
            "FoundationModels",
        ])
        .arg("native/macos/FoundationModels.swift")
        .arg("-o")
        .arg(&object)
        .status()
        .expect("compile FoundationModels bridge");
    assert!(
        status.success(),
        "FoundationModels bridge requires the macOS 26 SDK"
    );
    let status = Command::new("xcrun")
        .arg("ar")
        .arg("crs")
        .arg(out.join("libverenu_foundation_models.a"))
        .arg(&object)
        .status()
        .expect("archive FoundationModels bridge");
    assert!(status.success(), "archive FoundationModels bridge");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=verenu_foundation_models");
    // Swift Core ships with macOS 11, but Swift concurrency does not. Task
    // metadata can reference its runtime before an availability guard runs.
    let toolchain = PathBuf::from(swift)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    println!(
        "cargo:rustc-link-search=native={}",
        toolchain.join("lib/swift/macosx").display()
    );
    println!("cargo:rustc-link-search=native={sdk}/usr/lib/swift");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/swift-runtime");
    let concurrency = toolchain.join("lib/swift-5.5/macosx/libswift_Concurrency.dylib");
    assert!(
        concurrency.is_file(),
        "Xcode Swift concurrency back-deployment runtime is missing"
    );
    // Tauri's macOS frameworks list copies this dylib into Contents/Frameworks
    // and signs nested code before the app, for every bundling entry point.
    let stage = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"))
        .join("native/macos/swift-runtime");
    println!("cargo:rerun-if-changed={}", concurrency.display());
    println!(
        "cargo:rerun-if-changed={}",
        stage.join("libswift_Concurrency.dylib").display()
    );
    fs::create_dir_all(&stage).expect("create Swift runtime staging directory");
    fs::copy(&concurrency, stage.join("libswift_Concurrency.dylib"))
        .expect("stage Swift concurrency runtime for bundling");
    // Unbundled cargo/dev executables and test executables also need the
    // runtime on Big Sur. Keep these copies in this task's Cargo output only.
    let profile = out
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .expect("Cargo profile directory above build OUT_DIR");
    for directory in [profile.to_path_buf(), profile.join("deps")] {
        let runtime = directory.join("swift-runtime");
        fs::create_dir_all(&runtime).expect("create dev Swift runtime directory");
        fs::copy(&concurrency, runtime.join("libswift_Concurrency.dylib"))
            .expect("copy dev Swift concurrency runtime");
    }
    // Mach-O LC_LINKER_OPTION records carry Swift runtime autolinks. Darwin's
    // linker consumes them directly; swift-autolink-extract is an ELF tool.
    println!("cargo:rustc-link-arg=-Wl,-weak_framework,FoundationModels");
}
