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

    match env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => {
            windows_resources::embed();
            return;
        }
        Ok("macos") => {}
        _ => return,
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
    for framework in [
        "AppKit",
        "AVFoundation",
        "QuartzCore",
        "CoreGraphics",
        "IOKit",
        "UniformTypeIdentifiers",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}

/// Windows executable identity: the Auralis icon and a VERSIONINFO block, compiled with the
/// toolchain's own resource compiler (`rc.exe` for MSVC, `windres` for GNU) and passed to the
/// linker. No build dependency is needed; see THIRD_PARTY_LICENSES.md for the evaluation.
mod windows_resources {
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    pub fn embed() {
        println!("cargo:rerun-if-changed=assets/auralis.ico");
        println!("cargo:rerun-if-env-changed=RC");
        println!("cargo:rerun-if-env-changed=WINDRES");

        let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets it"));
        let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
        let icon = manifest_dir.join("assets").join("auralis.ico");
        let script = out_dir.join("auralis.rc");
        fs::write(&script, resource_script(&icon)).expect("could not write auralis.rc");

        let result = if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            compile_msvc(&script, &out_dir.join("auralis.res"))
        } else {
            compile_gnu(&script, &out_dir.join("auralis_resources.o"))
        };
        match result {
            Ok(object) => println!("cargo:rustc-link-arg-bins={}", object.display()),
            // A missing resource compiler must not block development builds. CI checks the
            // release executable's version information, so a release cannot silently lack it.
            Err(reason) => println!(
                "cargo:warning=auralis.exe will have no icon or version information: {reason}"
            ),
        }
    }

    fn resource_script(icon: &Path) -> String {
        let version = env::var("CARGO_PKG_VERSION").expect("Cargo sets CARGO_PKG_VERSION");
        let build = env::var("AURALIS_BUILD_NUMBER")
            .ok()
            .map(|b| b.trim().to_string())
            .filter(|b| !b.is_empty() && b.bytes().all(|c| c.is_ascii_digit()));
        let numeric = numeric_version(&version, build.as_deref());
        let display_version = match &build {
            Some(number) => format!("{version}+build.{number}"),
            None => version.clone(),
        };
        let authors = env::var("CARGO_PKG_AUTHORS").unwrap_or_default();
        let company = authors
            .split(':')
            .map(|author| author.split('<').next().unwrap_or("").trim())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        let license = env::var("CARGO_PKG_LICENSE").unwrap_or_default();
        let copyright = format!("Copyright (c) {company}. Licensed under {license}.");
        let numeric_list = numeric.map(|part| part.to_string()).join(",");
        let icon = rc_escape(&icon.display().to_string());
        let company = rc_escape(&company);
        let display_version = rc_escape(&display_version);
        let copyright = rc_escape(&copyright);

        // Numeric flags avoid needing winver.h: VS_FFI_FILEFLAGSMASK, VOS_NT_WINDOWS32, VFT_APP.
        [
            format!("1 ICON \"{icon}\""),
            "1 VERSIONINFO".into(),
            format!("FILEVERSION {numeric_list}"),
            format!("PRODUCTVERSION {numeric_list}"),
            "FILEFLAGSMASK 0x3F".into(),
            "FILEFLAGS 0x0".into(),
            "FILEOS 0x40004".into(),
            "FILETYPE 0x1".into(),
            "FILESUBTYPE 0x0".into(),
            "BEGIN".into(),
            "  BLOCK \"StringFileInfo\"".into(),
            "  BEGIN".into(),
            "    BLOCK \"040904B0\"".into(),
            "    BEGIN".into(),
            format!("      VALUE \"CompanyName\", \"{company}\""),
            "      VALUE \"FileDescription\", \"Auralis live wallpaper\"".into(),
            format!("      VALUE \"FileVersion\", \"{display_version}\""),
            "      VALUE \"InternalName\", \"auralis\"".into(),
            format!("      VALUE \"LegalCopyright\", \"{copyright}\""),
            "      VALUE \"OriginalFilename\", \"auralis.exe\"".into(),
            "      VALUE \"ProductName\", \"Auralis\"".into(),
            format!("      VALUE \"ProductVersion\", \"{display_version}\""),
            "    END".into(),
            "  END".into(),
            "  BLOCK \"VarFileInfo\"".into(),
            "  BEGIN".into(),
            "    VALUE \"Translation\", 0x409, 1200".into(),
            "  END".into(),
            "END".into(),
            String::new(),
        ]
        .join("\n")
    }

    /// `MAJOR.MINOR.PATCH` from the Cargo version (prerelease and metadata dropped) plus the CI
    /// build number as the fourth field, each clamped to the 16-bit VERSIONINFO range.
    fn numeric_version(version: &str, build: Option<&str>) -> [u16; 4] {
        let clamp = |n: u64| u16::try_from(n).unwrap_or(u16::MAX);
        let core = version.split(['-', '+']).next().unwrap_or("");
        let mut parts = core
            .split('.')
            .map(|part| part.parse::<u64>().map_or(0, clamp));
        let mut numeric = [0u16; 4];
        for slot in numeric.iter_mut().take(3) {
            *slot = parts.next().unwrap_or(0);
        }
        numeric[3] = build.and_then(|b| b.parse::<u64>().ok()).map_or(0, clamp);
        numeric
    }

    /// Escape a value for an RC string literal (backslashes are escapes; quotes double up).
    fn rc_escape(value: &str) -> String {
        value.replace('\\', "\\\\").replace('"', "\"\"")
    }

    fn compile_msvc(script: &Path, output: &Path) -> Result<PathBuf, String> {
        let rc = find_rc().ok_or("rc.exe was not found (install the Windows SDK or set RC)")?;
        let status = Command::new(&rc)
            .args(["/nologo", "/fo"])
            .arg(output)
            .arg(script)
            .status()
            .map_err(|error| format!("could not run {}: {error}", rc.display()))?;
        if status.success() {
            Ok(output.to_path_buf())
        } else {
            Err(format!("{} failed with {status}", rc.display()))
        }
    }

    /// `RC`, then `rc.exe` on PATH (a Developer Command Prompt), then the newest Windows 10/11
    /// SDK under `Windows Kits\10\bin`.
    fn find_rc() -> Option<PathBuf> {
        if let Some(rc) = env::var_os("RC").filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(rc));
        }
        if Command::new("rc.exe").arg("/?").output().is_ok() {
            return Some(PathBuf::from("rc.exe"));
        }
        let host_arch = match env::var("HOST").unwrap_or_default().split('-').next() {
            Some("x86_64") => "x64",
            Some("aarch64") => "arm64",
            _ => "x86",
        };
        let program_files = env::var_os("ProgramFiles(x86)")
            .or_else(|| env::var_os("ProgramFiles"))
            .map(PathBuf::from)?;
        let bin = program_files.join("Windows Kits").join("10").join("bin");
        let mut candidates: Vec<(Vec<u32>, PathBuf)> = fs::read_dir(&bin)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let version = name
                    .split('.')
                    .map(|part| part.parse::<u32>().ok())
                    .collect::<Option<Vec<_>>>()?;
                let rc = entry.path().join(host_arch).join("rc.exe");
                rc.is_file().then_some((version, rc))
            })
            .collect();
        candidates.sort();
        candidates.pop().map(|(_, rc)| rc)
    }

    fn compile_gnu(script: &Path, output: &Path) -> Result<PathBuf, String> {
        let (prefix, bfd_target) = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
            Ok("x86_64") => ("x86_64-w64-mingw32-", "pe-x86-64"),
            Ok("aarch64") => ("aarch64-w64-mingw32-", "pe-aarch64-little"),
            _ => ("i686-w64-mingw32-", "pe-i386"),
        };
        let candidates: Vec<PathBuf> = match env::var_os("WINDRES").filter(|v| !v.is_empty()) {
            Some(windres) => vec![PathBuf::from(windres)],
            None => vec![
                PathBuf::from(format!("{prefix}windres")),
                PathBuf::from("windres"),
            ],
        };
        let mut failures = Vec::new();
        for windres in candidates {
            let result = Command::new(&windres)
                .arg("--input")
                .arg(script)
                .arg("--output")
                .arg(output)
                .args(["--input-format=rc", "--output-format=coff"])
                .arg(format!("--target={bfd_target}"))
                .output();
            match result {
                Ok(out) if out.status.success() => return Ok(output.to_path_buf()),
                // Typically a windres built without the target's object format (32-bit only).
                Ok(out) => failures.push(format!(
                    "{} failed: {}",
                    windres.display(),
                    String::from_utf8_lossy(&out.stderr).trim()
                )),
                Err(error) => failures.push(format!("{}: {error}", windres.display())),
            }
        }
        Err(format!(
            "no windres that can produce {bfd_target} objects ({}); set WINDRES",
            failures.join("; ")
        ))
    }
}
