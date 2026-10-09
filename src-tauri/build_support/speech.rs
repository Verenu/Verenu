#[path = "speech_signing.rs"]
mod speech_signing;

pub fn build() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    println!("cargo:rerun-if-changed=native/macos/Speech.m");
    cc::Build::new()
        .file("native/macos/Speech.m")
        .flag("-fobjc-arc")
        .flag("-mmacosx-version-min=11.0")
        .compile("verenu_speech");
    for framework in ["Speech", "AVFoundation", "Foundation"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("aarch64") {
        build_fluid_helper();
    }
}

fn build_fluid_helper() {
    use std::{path::PathBuf, process::Command};
    println!("cargo:rerun-if-changed=native/macos/FluidSpeech/Package.swift");
    println!("cargo:rerun-if-changed=native/macos/FluidSpeech/Package.resolved");
    println!("cargo:rerun-if-changed=native/macos/FluidSpeech/Sources");
    println!("cargo:rerun-if-changed=native/macos/FluidSpeech/NOTICE");
    println!("cargo:rerun-if-changed=native/macos/FluidSpeech/LICENSE-FluidAudio");
    println!("cargo:rerun-if-env-changed=APPLE_SIGNING_IDENTITY");
    println!("cargo:rerun-if-changed=build_support/speech_signing.rs");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output"));
    let scratch = output.join("fluid-speech");
    let package = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("native/macos/FluidSpeech");
    let status = Command::new("swift")
        .args(["build", "--package-path"])
        .arg(&package)
        .arg("--scratch-path")
        .arg(&scratch)
        .args([
            "--disable-keychain",
            "--disable-netrc",
            "--disable-automatic-resolution",
            "-c",
            "release",
            "-j",
            "2",
        ])
        .status()
        .expect("Swift 6.2+ is required to build FluidAudio");
    assert!(status.success(), "FluidAudio native helper failed to build");
    let helper = scratch.join("release/VerenuFluidSpeech");
    // Sign before embedding so library validation and hardened runtime retain
    // the same team identity as the host. The helper has no eager host linkage.
    let configured_identity = std::env::var("APPLE_SIGNING_IDENTITY").ok();
    let identity = speech_signing::identity(configured_identity.as_deref());
    let signed = Command::new("codesign")
        .args(["--force", "--sign", identity, "--options", "runtime"])
        .arg(&helper)
        .status()
        .expect("sign FluidAudio helper");
    assert!(signed.success(), "FluidAudio helper signing failed");
    println!("cargo:rustc-env=VERENU_FLUID_HELPER={}", helper.display());
    let products = scratch.join("release");
    let resources = output.join("fluid-resources.tar");
    let bundles = std::fs::read_dir(&products)
        .expect("Swift products")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "bundle")
        })
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();
    assert!(
        !bundles.is_empty(),
        "FluidAudio SwiftPM resources must be packaged"
    );
    let packed = Command::new("tar")
        .arg("-cf")
        .arg(&resources)
        .arg("-C")
        .arg(&products)
        .args(bundles)
        .status()
        .expect("package FluidAudio resources");
    assert!(packed.success(), "FluidAudio resources failed to package");
    let dependency = scratch.join("checkouts/FluidAudio");
    let notices = Command::new("tar")
        .arg("-rf")
        .arg(&resources)
        .arg("-C")
        .arg(&dependency)
        .args(["LICENSE", "ThirdPartyLicenses"])
        .arg("-C")
        .arg(&package)
        .args(["NOTICE", "LICENSE-FluidAudio"])
        .status()
        .expect("package FluidAudio license notices");
    assert!(
        notices.success(),
        "FluidAudio license notices must accompany the helper"
    );
    println!(
        "cargo:rustc-env=VERENU_FLUID_RESOURCES={}",
        resources.display()
    );
}
