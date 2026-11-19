//! Automatic updates from GitHub Releases.
//!
//! Auralis asks the GitHub releases API for newer builds, downloads the file for the way it
//! was installed, checks it against the SHA-256 checksum published beside it, and installs it:
//!
//! - Windows (installed with `setup.exe`): runs the new installer silently. The installer
//!   closes Auralis, upgrades it in place, and starts it again (`/relaunch=`).
//! - macOS (running from `Auralis.app`): copies the new `Auralis.app` out of the disk image,
//!   swaps it with the running bundle, and reopens it once this process has exited.
//! - Anything else (the Windows portable zip, `cargo run`): reports the update only.
//!
//! Networking uses the operating system's own `curl` (`%SystemRoot%\System32\curl.exe` on
//! Windows 10 1803 and later, `/usr/bin/curl` on macOS) over HTTPS only, so Auralis carries no
//! TLS stack of its own. Builds are not code-signed yet: the checksum proves the download is
//! the file GitHub published for that release, not who built it.

use serde::Deserialize;
use std::cmp::Ordering;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// GitHub repository that publishes Auralis builds.
pub const REPOSITORY: &str = "somyacodes07/Auralis";
/// Only files under this prefix are ever downloaded.
const DOWNLOAD_PREFIX: &str = "https://github.com/somyacodes07/Auralis/releases/download/";
/// Largest file the updater accepts (the installers are a few megabytes).
const MAX_DOWNLOAD_BYTES: u64 = 200 * 1024 * 1024;
/// How often automatic checks run.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// Delay after launch before the first automatic check, so startup stays quick.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------------------------
// Versions
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreIdent {
    Numeric(u64),
    Alpha(String),
}

impl PartialOrd for PreIdent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PreIdent {
    // SemVer 11.4: numeric identifiers sort below alphanumeric ones.
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Numeric(a), Self::Numeric(b)) => a.cmp(b),
            (Self::Numeric(_), Self::Alpha(_)) => Ordering::Less,
            (Self::Alpha(_), Self::Numeric(_)) => Ordering::Greater,
            (Self::Alpha(a), Self::Alpha(b)) => a.cmp(b),
        }
    }
}

/// A release version: SemVer plus the CI build number (`0.2.0-beta.1` build 42).
///
/// Builds of the same SemVer version are ordered by build number; a version without a build
/// number counts as build 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseVersion {
    core: [u64; 3],
    pre: Vec<PreIdent>,
    build: u64,
}

impl ReleaseVersion {
    /// Parse `0.2.0-beta.1`, a tag such as `v0.2.0-beta.1-build.4`, or the older
    /// `v0.2.0-beta.1+build.3` form.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let (semver, build) = split_build(text);
        let (core_text, pre_text) = match semver.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (semver, None),
        };
        let mut core = [0u64; 3];
        let mut parts = core_text.split('.');
        for slot in &mut core {
            *slot = parse_number(parts.next()?)?;
        }
        if parts.next().is_some() {
            return None;
        }
        let pre = match pre_text {
            Some(pre) => pre
                .split('.')
                .map(|ident| {
                    if ident.is_empty() || !ident.bytes().all(|b| b.is_ascii_alphanumeric()) {
                        None
                    } else if ident.bytes().all(|b| b.is_ascii_digit()) {
                        parse_number(ident).map(PreIdent::Numeric)
                    } else {
                        Some(PreIdent::Alpha(ident.to_string()))
                    }
                })
                .collect::<Option<Vec<_>>>()?,
            None => Vec::new(),
        };
        Some(Self { core, pre, build })
    }

    /// The version of this running binary.
    #[must_use]
    pub fn current() -> Self {
        let mut version = Self::parse(crate::VERSION).expect("Cargo.toml has a SemVer version");
        version.build = crate::BUILD_NUMBER
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or(0);
        version
    }

    /// Whether this is a prerelease (alpha, beta, or a numbered CI build).
    #[must_use]
    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty() || self.build > 0
    }

    /// Text shown to people, for example `0.2.0-beta.1 build 42`.
    #[must_use]
    pub fn label(&self) -> String {
        let mut text = format!("{}.{}.{}", self.core[0], self.core[1], self.core[2]);
        if !self.pre.is_empty() {
            let idents: Vec<String> = self
                .pre
                .iter()
                .map(|ident| match ident {
                    PreIdent::Numeric(n) => n.to_string(),
                    PreIdent::Alpha(a) => a.clone(),
                })
                .collect();
            text.push('-');
            text.push_str(&idents.join("."));
        }
        if self.build > 0 {
            text.push_str(&format!(" build {}", self.build));
        }
        text
    }
}

fn split_build(text: &str) -> (&str, u64) {
    for marker in ["-build.", "+build."] {
        if let Some(index) = text.rfind(marker) {
            if let Some(number) = parse_number(&text[index + marker.len()..]) {
                return (&text[..index], number);
            }
        }
    }
    (text, 0)
}

fn parse_number(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

impl PartialOrd for ReleaseVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ReleaseVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            })
            .then_with(|| self.build.cmp(&other.build))
    }
}

// ---------------------------------------------------------------------------------------------
// Releases
// ---------------------------------------------------------------------------------------------

/// How this copy of Auralis was installed, which decides the file an update needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// Installed by `auralis-…-windows-x86_64-setup.exe`.
    WindowsInstaller,
    /// Unzipped from `auralis-…-windows-x86_64.zip`.
    WindowsPortable,
    /// Running from an `Auralis.app` bundle.
    MacApp,
    /// A development build or an unsupported platform.
    Unmanaged,
}

impl InstallKind {
    /// Detect how the running executable was installed.
    #[must_use]
    pub fn detect() -> Self {
        let Ok(exe) = std::env::current_exe() else {
            return Self::Unmanaged;
        };
        Self::for_executable(&exe)
    }

    fn for_executable(exe: &Path) -> Self {
        if cfg!(target_os = "windows") {
            let dir = exe.parent();
            if dir.is_some_and(|d| d.join("unins000.exe").is_file()) {
                Self::WindowsInstaller
            } else if crate::BUILD_NUMBER.is_some() {
                Self::WindowsPortable
            } else {
                Self::Unmanaged
            }
        } else if cfg!(target_os = "macos") && app_bundle_for(exe).is_some() {
            Self::MacApp
        } else {
            Self::Unmanaged
        }
    }

    /// File name suffix of the release asset this install updates from.
    #[must_use]
    pub const fn asset_suffix(self) -> Option<&'static str> {
        match self {
            Self::WindowsInstaller => Some("-windows-x86_64-setup.exe"),
            Self::WindowsPortable => Some("-windows-x86_64.zip"),
            Self::MacApp => Some("-macos-universal.dmg"),
            Self::Unmanaged => None,
        }
    }

    /// Whether Auralis can install the update itself.
    #[must_use]
    pub const fn can_self_install(self) -> bool {
        matches!(self, Self::WindowsInstaller | Self::MacApp)
    }
}

/// `…/Auralis.app` for `…/Auralis.app/Contents/MacOS/Auralis`.
fn app_bundle_for(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then(|| bundle.to_path_buf())
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

/// A newer build that this install can update to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateOffer {
    /// Version of the newer build.
    pub version: ReleaseVersion,
    /// Release page on GitHub.
    pub release_url: String,
    /// File name of the download.
    pub file_name: String,
    /// HTTPS download URL.
    pub download_url: String,
    /// URL of the published `<file>.sha256`.
    pub checksum_url: String,
    /// Download size in bytes as published.
    pub size: u64,
    /// How this copy was installed.
    pub kind: InstallKind,
}

/// Only plain file names (`auralis-v0.2.0-build.4-macos-universal.dmg`) are accepted.
fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name.starts_with("auralis-")
        && !name.contains("..")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// Pick the newest release that is newer than `current` and has a checksummed file for
/// `kind`. Prereleases are offered only to prerelease builds.
fn choose_update(
    releases: &[GitHubRelease],
    current: &ReleaseVersion,
    kind: InstallKind,
) -> Option<UpdateOffer> {
    let suffix = kind.asset_suffix()?;
    let allow_prerelease = current.is_prerelease();
    releases
        .iter()
        .filter(|release| !release.draft && (allow_prerelease || !release.prerelease))
        .filter_map(|release| {
            let version = ReleaseVersion::parse(&release.tag_name)?;
            if version <= *current {
                return None;
            }
            let asset = release.assets.iter().find(|asset| {
                asset.name.ends_with(suffix)
                    && safe_file_name(&asset.name)
                    && asset.browser_download_url.starts_with(DOWNLOAD_PREFIX)
                    && asset.size <= MAX_DOWNLOAD_BYTES
            })?;
            let checksum_name = format!("{}.sha256", asset.name);
            let checksum = release.assets.iter().find(|a| {
                a.name == checksum_name && a.browser_download_url.starts_with(DOWNLOAD_PREFIX)
            })?;
            Some(UpdateOffer {
                version,
                release_url: release.html_url.clone(),
                file_name: asset.name.clone(),
                download_url: asset.browser_download_url.clone(),
                checksum_url: checksum.browser_download_url.clone(),
                size: asset.size,
                kind,
            })
        })
        .max_by(|a, b| a.version.cmp(&b.version))
}

/// Read the digest for `file_name` from a `sha256sum`-style line (`<hex>  <name>`).
fn parse_checksum(text: &str, file_name: &str) -> Option<[u8; 32]> {
    text.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let hex = fields.next()?;
        let name = fields.next().map(|n| n.trim_start_matches('*'));
        if name.is_some_and(|n| n != file_name) {
            return None;
        }
        decode_hex_digest(hex)
    })
}

fn decode_hex_digest(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------------
// SHA-256 (FIPS 180-4)
// ---------------------------------------------------------------------------------------------

const SHA256_K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

/// Streaming SHA-256, used to check downloads against their published checksums.
struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffered: usize,
    length: u64,
}

impl Sha256 {
    const fn new() -> Self {
        Self {
            state: [
                0x6a09_e667,
                0xbb67_ae85,
                0x3c6e_f372,
                0xa54f_f53a,
                0x510e_527f,
                0x9b05_688c,
                0x1f83_d9ab,
                0x5be0_cd19,
            ],
            buffer: [0; 64],
            buffered: 0,
            length: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.length = self.length.wrapping_add(data.len() as u64);
        if self.buffered > 0 {
            let take = (64 - self.buffered).min(data.len());
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&data[..take]);
            self.buffered += take;
            data = &data[take..];
            if self.buffered < 64 {
                return;
            }
            let block = self.buffer;
            self.compress(&block);
            self.buffered = 0;
        }
        while data.len() >= 64 {
            let (block, rest) = data.split_at(64);
            self.compress(block.try_into().expect("64-byte block"));
            data = rest;
        }
        self.buffer[..data.len()].copy_from_slice(data);
        self.buffered = data.len();
    }

    fn finish(mut self) -> [u8; 32] {
        let bit_length = self.length.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buffered != 56 {
            self.update(&[0]);
        }
        self.update(&bit_length.to_be_bytes());
        let mut out = [0u8; 32];
        for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (index, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[index] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
}

fn sha256_file(path: &Path) -> std::io::Result<[u8; 32]> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finish())
}

// ---------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------

/// Why an update check, download, or install did not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    /// GitHub could not be reached or answered with an error.
    Network(String),
    /// GitHub's answer could not be read.
    BadResponse(String),
    /// The download did not match its published checksum.
    ChecksumMismatch,
    /// The update could not be written or installed.
    Install(String),
    /// This install cannot update itself.
    NotSupported,
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(detail) => write!(f, "GitHub could not be reached ({detail})"),
            Self::BadResponse(detail) => write!(f, "GitHub sent an unexpected answer ({detail})"),
            Self::ChecksumMismatch => write!(
                f,
                "The download did not match its published checksum, so it was deleted"
            ),
            Self::Install(detail) => write!(f, "The update could not be installed: {detail}"),
            Self::NotSupported => write!(
                f,
                "This copy of Auralis cannot update itself; download the new version from the website"
            ),
        }
    }
}

impl std::error::Error for UpdateError {}

// ---------------------------------------------------------------------------------------------
// Network (system curl)
// ---------------------------------------------------------------------------------------------

fn curl_path() -> PathBuf {
    if cfg!(target_os = "windows") {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        PathBuf::from(root).join("System32").join("curl.exe")
    } else {
        PathBuf::from("/usr/bin/curl")
    }
}

fn curl() -> Command {
    let mut command = Command::new(curl_path());
    command.args([
        "--silent",
        "--show-error",
        "--fail",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "20",
        "--retry",
        "2",
        "--user-agent",
    ]);
    command.arg(format!("Auralis-Updater/{}", crate::VERSION));
    hide_console(&mut command);
    command
}

#[cfg(target_os = "windows")]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
const fn hide_console(_command: &mut Command) {}

fn run_curl(mut command: Command) -> Result<Vec<u8>, UpdateError> {
    let output = command
        .output()
        .map_err(|error| UpdateError::Network(format!("curl could not start: {error}")))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(UpdateError::Network(if detail.is_empty() {
            format!("curl exited with {}", output.status)
        } else {
            detail
        }))
    }
}

fn fetch_releases() -> Result<Vec<GitHubRelease>, UpdateError> {
    let mut command = curl();
    command
        .args(["--max-time", "30", "--max-filesize", "5000000", "--header"])
        .arg("Accept: application/vnd.github+json")
        .arg(format!(
            "https://api.github.com/repos/{REPOSITORY}/releases?per_page=20"
        ));
    let body = run_curl(command)?;
    serde_json::from_slice(&body).map_err(|error| UpdateError::BadResponse(error.to_string()))
}

/// Ask GitHub whether a newer build exists for this install.
///
/// # Errors
/// Returns an error when GitHub cannot be reached or its answer cannot be read.
pub fn check_for_update() -> Result<Option<UpdateOffer>, UpdateError> {
    let releases = fetch_releases()?;
    Ok(choose_update(
        &releases,
        &ReleaseVersion::current(),
        InstallKind::detect(),
    ))
}

/// Where downloaded updates are kept until installed.
#[must_use]
pub fn download_directory() -> PathBuf {
    if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA")
            .map_or_else(std::env::temp_dir, PathBuf::from)
            .join("Auralis")
            .join("updates")
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map_or_else(std::env::temp_dir, PathBuf::from)
            .join("Library/Caches/Auralis/updates")
    } else {
        std::env::temp_dir().join("auralis-updates")
    }
}

/// Download `offer` into [`download_directory`] and verify its checksum. Earlier downloads are
/// removed first. A file that does not match is deleted.
///
/// # Errors
/// Returns an error when the download fails or does not match its checksum.
pub fn download(offer: &UpdateOffer) -> Result<PathBuf, UpdateError> {
    if !safe_file_name(&offer.file_name) || !offer.download_url.starts_with(DOWNLOAD_PREFIX) {
        return Err(UpdateError::BadResponse(
            "unexpected download location".into(),
        ));
    }
    let dir = download_directory();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)
        .map_err(|error| UpdateError::Install(format!("{}: {error}", dir.display())))?;

    let mut command = curl();
    command.args(["--max-time", "60", "--max-filesize", "4096"]);
    command.arg(&offer.checksum_url);
    let checksum_text = String::from_utf8_lossy(&run_curl(command)?).into_owned();
    let expected = parse_checksum(&checksum_text, &offer.file_name)
        .ok_or_else(|| UpdateError::BadResponse("the checksum file is not valid".into()))?;

    let target = dir.join(&offer.file_name);
    let mut command = curl();
    command
        .args(["--max-time", "900", "--max-filesize"])
        .arg(MAX_DOWNLOAD_BYTES.to_string())
        .arg("--output")
        .arg(&target)
        .arg(&offer.download_url);
    run_curl(command)?;

    let actual = sha256_file(&target).map_err(|error| UpdateError::Install(error.to_string()))?;
    if actual != expected {
        let _ = std::fs::remove_file(&target);
        return Err(UpdateError::ChecksumMismatch);
    }
    Ok(target)
}

/// Install a downloaded, verified update and arrange for Auralis to start again.
///
/// On Windows this starts the installer, which closes the running Auralis itself. On macOS the
/// app bundle is replaced and reopened after this process exits. In both cases the caller
/// should quit promptly afterwards (restoring the desktop as on a normal Quit).
///
/// # Errors
/// Returns an error when the update cannot be installed; the current version is left in place.
pub fn install(offer: &UpdateOffer, file: &Path, relaunch: Relaunch) -> Result<(), UpdateError> {
    match offer.kind {
        #[cfg(target_os = "windows")]
        InstallKind::WindowsInstaller => install_windows(file, relaunch),
        #[cfg(target_os = "macos")]
        InstallKind::MacApp => install_macos(file, relaunch),
        _ => {
            let _ = (file, relaunch);
            Err(UpdateError::NotSupported)
        }
    }
}

/// How Auralis should start again after an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relaunch {
    /// Do not start Auralis (the command line updater).
    No,
    /// Start in the background, as after an automatic update.
    Minimized,
    /// Start and open the Studio, as after "Check for Updates".
    Studio,
}

#[cfg(target_os = "windows")]
fn install_windows(setup: &Path, relaunch: Relaunch) -> Result<(), UpdateError> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    let mut command = Command::new(setup);
    command.args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]);
    match relaunch {
        Relaunch::No => {}
        Relaunch::Minimized => {
            command.arg("/relaunch=minimized");
        }
        Relaunch::Studio => {
            command.arg("/relaunch=studio");
        }
    }
    command.creation_flags(DETACHED_PROCESS);
    command
        .spawn()
        .map(drop)
        .map_err(|error| UpdateError::Install(format!("the installer could not start: {error}")))
}

#[cfg(target_os = "macos")]
fn install_macos(dmg: &Path, relaunch: Relaunch) -> Result<(), UpdateError> {
    let exe = std::env::current_exe().map_err(|error| UpdateError::Install(error.to_string()))?;
    let bundle = app_bundle_for(&exe).ok_or(UpdateError::NotSupported)?;
    let parent = bundle
        .parent()
        .ok_or_else(|| UpdateError::Install("Auralis.app has no parent folder".into()))?;

    let mount = download_directory().join("mount");
    let _ = std::fs::create_dir_all(&mount);
    let status = Command::new("/usr/bin/hdiutil")
        .args([
            "attach",
            "-nobrowse",
            "-readonly",
            "-noautoopen",
            "-mountpoint",
        ])
        .arg(&mount)
        .arg(dmg)
        .output()
        .map_err(|error| UpdateError::Install(format!("hdiutil could not start: {error}")))?;
    if !status.status.success() {
        return Err(UpdateError::Install(format!(
            "the disk image could not be opened: {}",
            String::from_utf8_lossy(&status.stderr).trim()
        )));
    }

    let staged = parent.join(".Auralis-update.app");
    let copied = copy_app_from_image(&mount, &staged, parent);
    let _ = Command::new("/usr/bin/hdiutil")
        .args(["detach", "-force"])
        .arg(&mount)
        .output();
    if let Err(error) = copied {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(error);
    }

    // Swap the bundles. A running process keeps its open files, so this is safe while
    // Auralis runs; if the second rename fails the original bundle is put back.
    let previous = parent.join(".Auralis-previous.app");
    let _ = std::fs::remove_dir_all(&previous);
    std::fs::rename(&bundle, &previous)
        .map_err(|error| UpdateError::Install(format!("{}: {error}", bundle.display())))?;
    if let Err(error) = std::fs::rename(&staged, &bundle) {
        let _ = std::fs::rename(&previous, &bundle);
        let _ = std::fs::remove_dir_all(&staged);
        return Err(UpdateError::Install(format!(
            "{}: {error}",
            bundle.display()
        )));
    }
    let _ = std::fs::remove_dir_all(&previous);
    let _ = std::fs::remove_file(dmg);

    if relaunch != Relaunch::No {
        // Wait for this process to exit, then open the new bundle.
        let mode = if relaunch == Relaunch::Minimized {
            "--minimized"
        } else {
            "--studio"
        };
        Command::new("/bin/sh")
            .arg("-c")
            .arg(
                "while /bin/kill -0 \"$1\" 2>/dev/null; do /bin/sleep 0.2; done; \
                 if [ \"$3\" = --minimized ]; then exec /usr/bin/open \"$2\" --args gui --minimized; \
                 else exec /usr/bin/open \"$2\" --args gui; fi",
            )
            .arg("auralis-relaunch")
            .arg(std::process::id().to_string())
            .arg(&bundle)
            .arg(mode)
            .spawn()
            .map_err(|error| UpdateError::Install(format!("could not schedule relaunch: {error}")))?;
    }
    Ok(())
}

/// Copy `Auralis.app` from the mounted disk image to `staged`.
#[cfg(target_os = "macos")]
fn copy_app_from_image(mount: &Path, staged: &Path, parent: &Path) -> Result<(), UpdateError> {
    let source = mount.join("Auralis.app");
    if !source.join("Contents/MacOS/Auralis").is_file() {
        return Err(UpdateError::Install(
            "the disk image does not contain Auralis.app".into(),
        ));
    }
    let _ = std::fs::remove_dir_all(staged);
    let output = Command::new("/usr/bin/ditto")
        .arg(&source)
        .arg(staged)
        .output()
        .map_err(|error| UpdateError::Install(format!("ditto could not start: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(UpdateError::Install(format!(
            "Auralis.app could not be copied into {} ({}). Move Auralis to your Applications folder and try again",
            parent.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}
// ---------------------------------------------------------------------------------------------
// Background agent used by the desktop apps
// ---------------------------------------------------------------------------------------------

/// Progress reported by [`UpdateAgent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateEvent {
    /// This is the newest build.
    UpToDate,
    /// A newer build exists but this install cannot update itself.
    Available(UpdateOffer),
    /// A newer build was downloaded and verified and is ready to install.
    Ready(UpdateOffer, PathBuf),
    /// The check or download failed.
    Failed(UpdateError),
}

/// Whether a check was started by the person or by the daily schedule. Automatic checks stay
/// silent unless they find something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckOrigin {
    /// "Check for Updates" in the tray or menu bar.
    Manual,
    /// The daily background check.
    Automatic,
}

/// Runs update checks and downloads on a background thread so the UI never blocks.
#[derive(Debug)]
pub struct UpdateAgent {
    running: Option<(CheckOrigin, Receiver<UpdateEvent>)>,
    started_at: SystemTime,
}

impl Default for UpdateAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateAgent {
    /// Create an idle agent. Automatic checks are measured from this moment.
    #[must_use]
    pub fn new() -> Self {
        Self {
            running: None,
            started_at: SystemTime::now(),
        }
    }

    /// Whether a check or download is in progress.
    #[must_use]
    pub const fn is_busy(&self) -> bool {
        self.running.is_some()
    }

    /// Start a check (and download, when the update can be installed) unless one is running.
    pub fn start(&mut self, origin: CheckOrigin) {
        if self.running.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("auralis-update".into())
            .spawn(move || {
                let event = match check_for_update() {
                    Ok(None) => UpdateEvent::UpToDate,
                    Ok(Some(offer)) if offer.kind.can_self_install() => match download(&offer) {
                        Ok(path) => UpdateEvent::Ready(offer, path),
                        Err(error) => UpdateEvent::Failed(error),
                    },
                    Ok(Some(offer)) => UpdateEvent::Available(offer),
                    Err(error) => UpdateEvent::Failed(error),
                };
                let _ = sender.send(event);
            });
        if spawned.is_ok() {
            self.running = Some((origin, receiver));
        }
    }

    /// Start the automatic check when it is due. Call this from the UI tick.
    pub fn tick(&mut self, enabled: bool, last_check: Option<u64>) {
        if enabled
            && !self.is_busy()
            && automatic_check_due(
                unix_seconds(SystemTime::now()),
                unix_seconds(self.started_at),
                last_check,
            )
        {
            self.start(CheckOrigin::Automatic);
        }
    }

    /// Collect the result of a finished check.
    pub fn poll(&mut self) -> Option<(CheckOrigin, UpdateEvent)> {
        let (origin, receiver) = self.running.as_ref()?;
        let origin = *origin;
        let event = match receiver.try_recv() {
            Ok(event) => event,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => UpdateEvent::Failed(UpdateError::Install(
                "the update check stopped unexpectedly".into(),
            )),
        };
        self.running = None;
        Some((origin, event))
    }
}

/// Whether automatic updates apply to this build. Local development builds (no CI build
/// number) never update themselves automatically.
#[must_use]
pub fn automatic_updates_supported() -> bool {
    crate::BUILD_NUMBER.is_some() && InstallKind::detect() != InstallKind::Unmanaged
}

/// Seconds since the Unix epoch.
#[must_use]
pub fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn automatic_check_due(now: u64, started: u64, last_check: Option<u64>) -> bool {
    if now < started + FIRST_CHECK_DELAY.as_secs() {
        return false;
    }
    match last_check {
        // A clock moved backwards also counts as due.
        Some(last) => now < last || now - last >= CHECK_INTERVAL.as_secs(),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> ReleaseVersion {
        ReleaseVersion::parse(text).unwrap()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn sha(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hex(&hasher.finish())
    }

    #[test]
    fn parses_tags_and_build_numbers() {
        assert_eq!(v("v0.2.0-beta.1-build.4").build, 4);
        assert_eq!(v("v0.2.0-beta.1+build.3").build, 3);
        assert_eq!(v("0.2.0").build, 0);
        assert_eq!(v("v0.2.0-beta.1-build.4").label(), "0.2.0-beta.1 build 4");
        assert_eq!(v("v1.0.0").label(), "1.0.0");
        assert!(ReleaseVersion::parse("v0.2").is_none());
        assert!(ReleaseVersion::parse("v0.2.0.1").is_none());
        assert!(ReleaseVersion::parse("latest").is_none());
        assert!(ReleaseVersion::parse("v0.2.0-").is_none());
    }

    #[test]
    fn orders_versions_like_semver_then_build() {
        assert!(v("0.2.0-beta.1-build.5") > v("0.2.0-beta.1-build.4"));
        assert!(v("0.2.0-beta.1-build.10") > v("0.2.0-beta.1-build.9"));
        assert!(v("0.2.0-beta.2") > v("0.2.0-beta.1-build.99"));
        assert!(v("0.2.0") > v("0.2.0-beta.2"));
        assert!(v("0.2.0-beta.1") > v("0.2.0-alpha.9"));
        assert!(v("0.2.0-beta.11") > v("0.2.0-beta.2"));
        assert!(v("0.3.0-alpha.1") > v("0.2.9"));
        assert_eq!(v("v0.2.0-beta.1+build.3"), v("0.2.0-beta.1-build.3"));
    }

    #[test]
    fn current_version_matches_cargo() {
        let current = ReleaseVersion::current();
        assert!(current.label().starts_with(crate::VERSION));
    }

    fn release(tag: &str, prerelease: bool, files: &[&str]) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.into(),
            draft: false,
            prerelease,
            html_url: format!("https://github.com/{REPOSITORY}/releases/tag/{tag}"),
            assets: files
                .iter()
                .map(|file| GitHubAsset {
                    name: (*file).into(),
                    browser_download_url: format!("{DOWNLOAD_PREFIX}{tag}/{file}"),
                    size: 1000,
                })
                .collect(),
        }
    }

    fn build(tag: &str) -> GitHubRelease {
        let setup = format!("auralis-{tag}-windows-x86_64-setup.exe");
        let dmg = format!("auralis-{tag}-macos-universal.dmg");
        release(
            tag,
            true,
            &[
                &setup,
                &format!("{setup}.sha256"),
                &dmg,
                &format!("{dmg}.sha256"),
            ],
        )
    }

    #[test]
    fn chooses_newest_newer_build_for_install_kind() {
        let releases = vec![
            build("v0.2.0-beta.1-build.6"),
            build("v0.2.0-beta.1-build.7"),
            build("v0.2.0-beta.1-build.3"),
        ];
        let current = v("0.2.0-beta.1-build.5");
        let offer = choose_update(&releases, &current, InstallKind::WindowsInstaller).unwrap();
        assert_eq!(offer.version, v("0.2.0-beta.1-build.7"));
        assert_eq!(
            offer.file_name,
            "auralis-v0.2.0-beta.1-build.7-windows-x86_64-setup.exe"
        );
        assert!(offer.checksum_url.ends_with("setup.exe.sha256"));

        let mac = choose_update(&releases, &current, InstallKind::MacApp).unwrap();
        assert!(mac.file_name.ends_with("-macos-universal.dmg"));

        assert!(
            choose_update(&releases, &v("0.2.0-beta.1-build.7"), InstallKind::MacApp).is_none()
        );
        assert!(choose_update(&releases, &current, InstallKind::Unmanaged).is_none());
        // No portable zip in these releases.
        assert!(choose_update(&releases, &current, InstallKind::WindowsPortable).is_none());
    }

    #[test]
    fn skips_releases_without_checksum_or_from_elsewhere() {
        let tag = "v0.2.0-beta.1-build.9";
        let unsummed = release(
            tag,
            true,
            &["auralis-v0.2.0-beta.1-build.9-macos-universal.dmg"],
        );
        let current = v("0.2.0-beta.1-build.1");
        assert!(choose_update(&[unsummed], &current, InstallKind::MacApp).is_none());

        let mut foreign = build(tag);
        for asset in &mut foreign.assets {
            asset.browser_download_url = format!("https://example.com/{}", asset.name);
        }
        assert!(choose_update(&[foreign], &current, InstallKind::MacApp).is_none());

        let mut draft = build(tag);
        draft.draft = true;
        assert!(choose_update(&[draft], &current, InstallKind::MacApp).is_none());
    }

    #[test]
    fn stable_builds_ignore_prereleases() {
        let releases = vec![build("v1.1.0-build.2"), {
            let mut stable = build("v1.0.1");
            stable.prerelease = false;
            stable
        }];
        let offer = choose_update(&releases, &v("1.0.0"), InstallKind::MacApp).unwrap();
        assert_eq!(offer.version, v("1.0.1"));
    }

    #[test]
    fn rejects_unsafe_file_names() {
        assert!(safe_file_name("auralis-v0.2.0-build.4-macos-universal.dmg"));
        assert!(!safe_file_name("../auralis.dmg"));
        assert!(!safe_file_name("auralis-../x.dmg"));
        assert!(!safe_file_name("auralis-a/b.dmg"));
        assert!(!safe_file_name("other.dmg"));
        assert!(!safe_file_name("auralis-a b.dmg"));
    }

    #[test]
    fn reads_published_checksum_lines() {
        let digest = "ab".repeat(32);
        let text = format!("{digest}  auralis-x.dmg\n");
        assert_eq!(parse_checksum(&text, "auralis-x.dmg"), Some([0xab; 32]));
        assert_eq!(parse_checksum(&text, "auralis-y.dmg"), None);
        assert_eq!(
            parse_checksum(&format!("{}\n", digest.to_uppercase()), "auralis-x.dmg"),
            Some([0xab; 32])
        );
        assert_eq!(parse_checksum("abc  auralis-x.dmg", "auralis-x.dmg"), None);
    }

    #[test]
    fn sha256_matches_fips_vectors() {
        assert_eq!(
            sha(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let million = vec![b'a'; 1_000_000];
        assert_eq!(
            sha(&million),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
        // Split updates hash the same as one update.
        let mut hasher = Sha256::new();
        for chunk in million.chunks(997) {
            hasher.update(chunk);
        }
        assert_eq!(hex(&hasher.finish()), sha(&million));
    }

    #[test]
    fn sha256_of_file_matches_memory() {
        let path = std::env::temp_dir().join(format!("auralis-sha-{}.bin", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            hex(&sha256_file(&path).unwrap()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn detects_mac_app_bundles() {
        assert_eq!(
            app_bundle_for(Path::new(
                "/Applications/Auralis.app/Contents/MacOS/Auralis"
            )),
            Some(PathBuf::from("/Applications/Auralis.app"))
        );
        assert_eq!(app_bundle_for(Path::new("/usr/local/bin/auralis")), None);
        assert_eq!(
            app_bundle_for(Path::new("/x/target/release/MacOS/Contents/auralis")),
            None
        );
    }

    /// Run with `cargo test -- --ignored`: needs network access to GitHub.
    #[test]
    #[ignore = "downloads a published build from GitHub"]
    fn downloads_and_verifies_a_published_build() {
        let releases = fetch_releases().unwrap();
        let offer =
            choose_update(&releases, &v("0.0.1-alpha.0"), InstallKind::WindowsPortable).unwrap();
        let path = download(&offer).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), offer.size);
        let _ = std::fs::remove_dir_all(download_directory());
    }

    #[test]
    fn schedules_daily_checks_after_startup_delay() {
        let start = 1_000_000;
        let day = CHECK_INTERVAL.as_secs();
        assert!(!automatic_check_due(start + 10, start, None));
        assert!(automatic_check_due(start + 61, start, None));
        assert!(!automatic_check_due(start + 61, start, Some(start - 100)));
        assert!(automatic_check_due(
            start + 61,
            start,
            Some(start + 61 - day)
        ));
        assert!(automatic_check_due(start + 61, start, Some(start + 5000)));
    }
}
