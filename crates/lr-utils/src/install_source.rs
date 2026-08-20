//! Working out which package manager, if any, owns this installation.
//!
//! LocalRouter ships a signed Tauri auto-updater that replaces the app bundle
//! in place. That is correct for someone who downloaded a DMG or an NSIS
//! installer, and actively harmful for someone who ran `brew install --cask`:
//! Homebrew records a version and a checksum for the bundle it placed, so an
//! in-place self-update leaves the cask permanently "outdated" and the next
//! `brew upgrade` happily overwrites whatever the app installed. The same
//! applies to `apt`, `dnf`, `flatpak`, and `snap`.
//!
//! So: whenever an external package manager owns the install, the self-updater
//! must stand down and the UI must tell the user which command to run instead.
//!
//! Detection precedence, highest first:
//!
//! 1. `LOCALROUTER_INSTALL_SOURCE` — an explicit packager override
//! 2. live runtime signals: `APPIMAGE`, then the sandbox we are confined by
//!    ([`crate::sandbox`])
//! 3. the `install-source` marker file we write from deb/rpm/AUR packaging
//! 4. `/.dockerenv`
//! 5. executable-path heuristics (Scoop, Homebrew Caskroom, Linux `/usr`)
//!
//! Steps 2 and 3 are in that order deliberately. Tauri builds the AppImage
//! from the deb tree, and the Flatpak and Snap recipes repack that same deb,
//! so all three images carry a marker file that says `deb`. Only the runtime
//! signal knows how the process was actually launched.
//!
//! Anything unrecognised falls through to [`InstallSource::Direct`], which
//! preserves today's behaviour: the self-updater stays on.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::sandbox::{self, Sandbox};

/// How this copy of LocalRouter was installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallSource {
    /// A downloaded DMG / MSI / NSIS installer. The in-app updater owns updates.
    Direct,
    /// A downloaded AppImage. Tauri's updater can replace it in place.
    #[serde(rename = "appimage")]
    AppImage,
    Homebrew,
    Scoop,
    /// Windows Package Manager.
    #[serde(rename = "winget")]
    WinGet,
    /// Arch User Repository (`localrouter-bin`).
    Aur,
    /// A `.deb` installed through apt/dpkg.
    Debian,
    /// An `.rpm` installed through dnf/yum/zypper.
    Rpm,
    /// A Linux system package whose exact manager could not be determined.
    SystemPackage,
    Flatpak,
    Snap,
    /// Running inside the GHCR container image.
    Docker,
}

impl InstallSource {
    /// Whether the in-app Tauri updater may replace this installation.
    ///
    /// Only true when nothing else is tracking the installed files.
    pub fn is_self_updatable(self) -> bool {
        matches!(self, InstallSource::Direct | InstallSource::AppImage)
    }

    /// Human-readable name of the owning package manager, for the UI.
    pub fn label(self) -> &'static str {
        match self {
            InstallSource::Direct => "Direct download",
            InstallSource::AppImage => "AppImage",
            InstallSource::Homebrew => "Homebrew",
            InstallSource::Scoop => "Scoop",
            InstallSource::WinGet => "WinGet",
            InstallSource::Aur => "AUR",
            InstallSource::Debian => "APT",
            InstallSource::Rpm => "DNF",
            InstallSource::SystemPackage => "System package manager",
            InstallSource::Flatpak => "Flatpak",
            InstallSource::Snap => "Snap",
            InstallSource::Docker => "Docker",
        }
    }

    /// The command the user should run to upgrade, when one exists.
    ///
    /// `None` means either the app updates itself, or we know a package
    /// manager owns it but cannot name the command (see
    /// [`InstallSource::SystemPackage`]).
    pub fn upgrade_command(self) -> Option<&'static str> {
        match self {
            InstallSource::Direct | InstallSource::AppImage | InstallSource::SystemPackage => None,
            InstallSource::Homebrew => Some("brew upgrade --cask localrouter"),
            InstallSource::Scoop => Some("scoop update localrouter"),
            InstallSource::WinGet => Some("winget upgrade LocalRouter.LocalRouter"),
            InstallSource::Aur => Some("yay -S localrouter-bin"),
            InstallSource::Debian => {
                Some("sudo apt update && sudo apt install --only-upgrade localrouter")
            }
            InstallSource::Rpm => Some("sudo dnf upgrade localrouter"),
            InstallSource::Flatpak => Some("flatpak update ai.localrouter.app"),
            InstallSource::Snap => Some("sudo snap refresh localrouter"),
            InstallSource::Docker => Some("docker pull ghcr.io/localrouter/localrouter:latest"),
        }
    }

    /// Parse the value written by a packager into the override env var or the
    /// marker file. Unknown values are rejected rather than guessed at.
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "direct" => Some(InstallSource::Direct),
            "appimage" => Some(InstallSource::AppImage),
            "homebrew" | "brew" => Some(InstallSource::Homebrew),
            "scoop" => Some(InstallSource::Scoop),
            "winget" => Some(InstallSource::WinGet),
            "aur" | "pacman" => Some(InstallSource::Aur),
            "deb" | "debian" | "apt" => Some(InstallSource::Debian),
            "rpm" | "dnf" | "yum" => Some(InstallSource::Rpm),
            "flatpak" => Some(InstallSource::Flatpak),
            "snap" => Some(InstallSource::Snap),
            "docker" => Some(InstallSource::Docker),
            _ => None,
        }
    }
}

/// Where packaging scripts drop the `install-source` marker file.
///
/// On Linux the executable lands in `/usr/bin`, which must not be polluted
/// with data files, so the marker goes to the package's share directory.
/// Elsewhere the app is a self-contained bundle and the marker sits next to
/// the executable.
pub fn marker_path(exe_path: Option<&Path>) -> Option<PathBuf> {
    if cfg!(target_os = "linux") {
        return Some(PathBuf::from("/usr/share/localrouter/install-source"));
    }
    exe_path?.parent().map(|dir| dir.join("install-source"))
}

/// Detect how this process was installed, reading the real environment.
pub fn current() -> InstallSource {
    let exe_path = std::env::current_exe().ok();
    let marker =
        marker_path(exe_path.as_deref()).and_then(|path| std::fs::read_to_string(path).ok());

    detect_install_source(
        |key| std::env::var(key).ok(),
        exe_path.as_deref(),
        sandbox::current(),
        marker.as_deref(),
        |path| path.exists(),
    )
}

/// Pure install-source detection, parameterised over every input it reads.
///
/// `marker` is the raw contents of the marker file, if it exists.
pub(crate) fn detect_install_source<E, P>(
    env: E,
    exe_path: Option<&Path>,
    sandbox: Sandbox,
    marker: Option<&str>,
    path_exists: P,
) -> InstallSource
where
    E: Fn(&str) -> Option<String>,
    P: Fn(&Path) -> bool,
{
    // 1. Explicit override always wins, so a packager can correct anything
    //    below.
    if let Some(source) = env("LOCALROUTER_INSTALL_SOURCE").and_then(|v| InstallSource::parse(&v)) {
        return source;
    }

    // 2. Live runtime signals beat the marker file, and this ordering is
    //    load-bearing rather than arbitrary. Tauri builds the AppImage *from*
    //    the deb tree, and our Flatpak and Snap recipes unpack that same deb,
    //    so all three images contain the deb's
    //    `/usr/share/localrouter/install-source` saying "deb". Trusting the
    //    marker first would report an AppImage as an apt install and switch
    //    off its self-updater, which is the one Linux format that *can*
    //    self-update.
    if env("APPIMAGE").is_some_and(|v| !v.is_empty()) {
        return InstallSource::AppImage;
    }
    match sandbox {
        Sandbox::Flatpak => return InstallSource::Flatpak,
        Sandbox::Snap => return InstallSource::Snap,
        Sandbox::None => {}
    }

    // 3. Marker file written at package build time. This is what separates
    //    deb from rpm from AUR, which are otherwise identical at runtime.
    if let Some(source) = marker.and_then(InstallSource::parse) {
        return source;
    }

    if path_exists(Path::new("/.dockerenv")) {
        return InstallSource::Docker;
    }

    // 4. Path heuristics.
    if let Some(exe) = exe_path {
        if let Some(source) = detect_from_exe_path(exe, &path_exists) {
            return source;
        }
    }

    InstallSource::Direct
}

/// Heuristics based on where the executable ended up on disk.
fn detect_from_exe_path<P>(exe: &Path, path_exists: &P) -> Option<InstallSource>
where
    P: Fn(&Path) -> bool,
{
    let exe_str = exe.to_string_lossy().to_ascii_lowercase();

    // Scoop always installs under `<scoop root>\apps\<name>\<version>\`, and
    // the root is user-configurable, so match the stable middle segment.
    // Normalise separators first: the same manifest is testable from a host
    // that uses `/`.
    let normalised = exe_str.replace('\\', "/");
    if normalised.contains("/scoop/apps/") {
        return Some(InstallSource::Scoop);
    }

    // A Homebrew cask stages the bundle in the Caskroom and moves it to
    // /Applications, so the executable path itself looks like a direct
    // install. The Caskroom directory is what gives it away.
    if cfg!(target_os = "macos") {
        for prefix in ["/opt/homebrew", "/usr/local"] {
            if path_exists(&PathBuf::from(prefix).join("Caskroom/localrouter")) {
                return Some(InstallSource::Homebrew);
            }
        }
    }

    // A Linux system package is the only thing that puts our binary in
    // /usr/bin. deb, rpm and AUR are indistinguishable at this point — the
    // marker file above is what separates them, so this is the fallback for a
    // package built without one.
    if cfg!(target_os = "linux")
        && (normalised.starts_with("/usr/bin/") || normalised.starts_with("/usr/local/bin/"))
    {
        return Some(InstallSource::SystemPackage);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_from<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        let map: HashMap<&str, &str> = pairs.iter().copied().collect();
        move |key| map.get(key).map(|v| v.to_string())
    }

    fn no_paths(_: &Path) -> bool {
        false
    }

    #[test]
    fn defaults_to_direct_download() {
        assert_eq!(
            detect_install_source(env_from(&[]), None, Sandbox::None, None, no_paths),
            InstallSource::Direct
        );
    }

    #[test]
    fn env_override_wins_over_everything() {
        // Even though we are demonstrably inside Flatpak with a conflicting
        // marker, the explicit override is honoured.
        assert_eq!(
            detect_install_source(
                env_from(&[("LOCALROUTER_INSTALL_SOURCE", "homebrew")]),
                Some(Path::new("/usr/bin/localrouter")),
                Sandbox::Flatpak,
                Some("deb"),
                no_paths,
            ),
            InstallSource::Homebrew
        );
    }

    #[test]
    fn unrecognised_override_falls_through() {
        assert_eq!(
            detect_install_source(
                env_from(&[("LOCALROUTER_INSTALL_SOURCE", "nonsense")]),
                None,
                Sandbox::None,
                None,
                no_paths,
            ),
            InstallSource::Direct
        );
    }

    #[test]
    fn marker_file_wins_over_path_heuristics() {
        // /usr/bin alone would only yield the generic SystemPackage; the
        // marker is what identifies the actual package manager.
        assert_eq!(
            detect_install_source(
                env_from(&[]),
                Some(Path::new("/usr/bin/localrouter")),
                Sandbox::None,
                Some("rpm"),
                no_paths,
            ),
            InstallSource::Rpm
        );
    }

    #[test]
    fn appimage_beats_the_deb_marker_baked_into_its_image() {
        // Tauri builds the AppImage from the deb tree, so the squashfs really
        // does contain the deb's marker file. The APPIMAGE env var is the
        // truth about how this process was launched — getting this backwards
        // would disable self-updates for the one self-updatable Linux format.
        assert_eq!(
            detect_install_source(
                env_from(&[("APPIMAGE", "/home/u/LocalRouter.AppImage")]),
                Some(Path::new("/tmp/.mount_LocalXXXX/usr/bin/localrouter")),
                Sandbox::None,
                Some("deb"),
                no_paths,
            ),
            InstallSource::AppImage
        );
    }

    #[test]
    fn sandboxes_beat_the_deb_marker_unpacked_into_them() {
        // Both the Flatpak manifest and snapcraft.yaml repack the .deb, so the
        // same stale marker is present inside each image.
        for (sandbox, expected) in [
            (Sandbox::Flatpak, InstallSource::Flatpak),
            (Sandbox::Snap, InstallSource::Snap),
        ] {
            assert_eq!(
                detect_install_source(
                    env_from(&[]),
                    Some(Path::new("/usr/bin/localrouter")),
                    sandbox,
                    Some("deb"),
                    no_paths,
                ),
                expected
            );
        }
    }

    #[test]
    fn marker_file_is_trimmed_and_case_insensitive() {
        // Packaging scripts write it with `echo`, so it carries a newline.
        assert_eq!(
            detect_install_source(
                env_from(&[]),
                None,
                Sandbox::None,
                Some("  AUR \n"),
                no_paths
            ),
            InstallSource::Aur
        );
    }

    #[test]
    fn sandbox_identifies_flatpak_and_snap() {
        assert_eq!(
            detect_install_source(env_from(&[]), None, Sandbox::Flatpak, None, no_paths),
            InstallSource::Flatpak
        );
        assert_eq!(
            detect_install_source(env_from(&[]), None, Sandbox::Snap, None, no_paths),
            InstallSource::Snap
        );
    }

    #[test]
    fn appimage_env_detected() {
        assert_eq!(
            detect_install_source(
                env_from(&[("APPIMAGE", "/home/u/LocalRouter.AppImage")]),
                None,
                Sandbox::None,
                None,
                no_paths,
            ),
            InstallSource::AppImage
        );
    }

    #[test]
    fn docker_detected_from_dockerenv() {
        assert_eq!(
            detect_install_source(env_from(&[]), None, Sandbox::None, None, |p| p
                == Path::new("/.dockerenv")),
            InstallSource::Docker
        );
    }

    #[test]
    fn scoop_detected_from_exe_path() {
        assert_eq!(
            detect_install_source(
                env_from(&[]),
                Some(Path::new(
                    r"C:\Users\u\scoop\apps\localrouter\0.0.1\LocalRouter.exe"
                )),
                Sandbox::None,
                None,
                no_paths,
            ),
            InstallSource::Scoop
        );
    }

    #[test]
    fn scoop_detected_under_a_custom_root() {
        assert_eq!(
            detect_install_source(
                env_from(&[]),
                Some(Path::new(
                    r"D:\Tools\Scoop\apps\LocalRouter\current\LocalRouter.exe"
                )),
                Sandbox::None,
                None,
                no_paths,
            ),
            InstallSource::Scoop
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn homebrew_detected_from_caskroom() {
        assert_eq!(
            detect_install_source(
                env_from(&[]),
                Some(Path::new(
                    "/Applications/LocalRouter.app/Contents/MacOS/LocalRouter"
                )),
                Sandbox::None,
                None,
                |p| p == Path::new("/opt/homebrew/Caskroom/localrouter"),
            ),
            InstallSource::Homebrew
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn intel_homebrew_prefix_also_detected() {
        assert_eq!(
            detect_install_source(
                env_from(&[]),
                Some(Path::new(
                    "/Applications/LocalRouter.app/Contents/MacOS/LocalRouter"
                )),
                Sandbox::None,
                None,
                |p| p == Path::new("/usr/local/Caskroom/localrouter"),
            ),
            InstallSource::Homebrew
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn dmg_install_without_caskroom_is_direct() {
        assert_eq!(
            detect_install_source(
                env_from(&[]),
                Some(Path::new(
                    "/Applications/LocalRouter.app/Contents/MacOS/LocalRouter"
                )),
                Sandbox::None,
                None,
                no_paths,
            ),
            InstallSource::Direct
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn usr_bin_without_a_marker_is_a_generic_system_package() {
        assert_eq!(
            detect_install_source(
                env_from(&[]),
                Some(Path::new("/usr/bin/localrouter")),
                Sandbox::None,
                None,
                no_paths,
            ),
            InstallSource::SystemPackage
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_binary_run_from_a_download_dir_is_direct() {
        assert_eq!(
            detect_install_source(
                env_from(&[]),
                Some(Path::new("/home/u/Downloads/localrouter")),
                Sandbox::None,
                None,
                no_paths,
            ),
            InstallSource::Direct
        );
    }

    #[test]
    fn only_direct_and_appimage_self_update() {
        assert!(InstallSource::Direct.is_self_updatable());
        assert!(InstallSource::AppImage.is_self_updatable());
        for source in [
            InstallSource::Homebrew,
            InstallSource::Scoop,
            InstallSource::WinGet,
            InstallSource::Aur,
            InstallSource::Debian,
            InstallSource::Rpm,
            InstallSource::SystemPackage,
            InstallSource::Flatpak,
            InstallSource::Snap,
            InstallSource::Docker,
        ] {
            assert!(
                !source.is_self_updatable(),
                "{source:?} must not self-update"
            );
        }
    }

    #[test]
    fn every_managed_source_names_its_upgrade_command() {
        // SystemPackage is the deliberate exception: we know a package manager
        // owns it but not which one.
        for source in [
            InstallSource::Homebrew,
            InstallSource::Scoop,
            InstallSource::WinGet,
            InstallSource::Aur,
            InstallSource::Debian,
            InstallSource::Rpm,
            InstallSource::Flatpak,
            InstallSource::Snap,
            InstallSource::Docker,
        ] {
            assert!(
                source.upgrade_command().is_some(),
                "{source:?} needs an upgrade command"
            );
        }
        assert!(InstallSource::SystemPackage.upgrade_command().is_none());
        assert!(InstallSource::Direct.upgrade_command().is_none());
    }

    #[test]
    fn serde_names_match_what_the_frontend_expects() {
        // The default snake_case rule would mangle these two into "win_get"
        // and "app_image"; both carry explicit renames.
        let cases = [
            (InstallSource::WinGet, "\"winget\""),
            (InstallSource::AppImage, "\"appimage\""),
            (InstallSource::Homebrew, "\"homebrew\""),
            (InstallSource::SystemPackage, "\"system_package\""),
        ];
        for (source, expected) in cases {
            assert_eq!(serde_json::to_string(&source).unwrap(), expected);
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_marker_lives_in_the_share_directory() {
        assert_eq!(
            marker_path(Some(Path::new("/usr/bin/localrouter"))),
            Some(PathBuf::from("/usr/share/localrouter/install-source"))
        );
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn bundle_marker_sits_next_to_the_executable() {
        assert_eq!(
            marker_path(Some(Path::new(
                "/Applications/LocalRouter.app/Contents/MacOS/LocalRouter"
            ))),
            Some(PathBuf::from(
                "/Applications/LocalRouter.app/Contents/MacOS/install-source"
            ))
        );
    }
}
