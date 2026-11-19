use std::env;
use std::path::PathBuf;
use std::process::Command;

fn run(command: &mut Command, description: &str) {
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("could not run {description}: {error}"));
    assert!(status.success(), "{description} failed with {status}");
}

fn main() {
    println!("cargo:rerun-if-changed=src/platform/macos_native.m");
    // CI stamps release builds with a build number (see .github/workflows/package.yml).
    println!("cargo:rerun-if-env-changed=AURALIS_BUILD_NUMBER");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let object = out_dir.join("auralis_macos_native.o");
    let archive = out_dir.join("libauralis_macos_native.a");

    // Compile the bridge for the Rust target (not the host) so universal builds can link
    // separate arm64 and x86_64 slices.
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        Ok(other) => panic!("unsupported macOS target architecture: {other}"),
        Err(error) => panic!("Cargo did not set CARGO_CFG_TARGET_ARCH: {error}"),
    };
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    let min_version = env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "13.0".into());

    let mut clang = Command::new("xcrun");
    clang
        .args(["clang", "-fobjc-arc", "-fmodules", "-arch", arch])
        .arg(format!("-mmacosx-version-min={min_version}"))
        .arg("-c")
        .arg("src/platform/macos_native.m")
        .arg("-o")
        .arg(&object);
    run(&mut clang, "Objective-C macOS bridge compilation");

    let mut libtool = Command::new("xcrun");
    libtool
        .args(["libtool", "-static", "-o"])
        .arg(&archive)
        .arg(&object);
    run(&mut libtool, "Objective-C macOS bridge archive creation");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=auralis_macos_native");
    for framework in ["AppKit", "AVFoundation", "QuartzCore", "CoreGraphics"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
