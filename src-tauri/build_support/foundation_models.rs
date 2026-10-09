use std::{env, path::PathBuf, process::Command};

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
    // Swift ABI libraries ship with macOS 11+. The bridge is part of the main
    // executable, so the existing app signing flow signs it, with no sidecar.
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
    // Mach-O LC_LINKER_OPTION records carry Swift runtime autolinks. Darwin's
    // linker consumes them directly; swift-autolink-extract is an ELF tool.
    println!("cargo:rustc-link-arg=-Wl,-weak_framework,FoundationModels");
}
