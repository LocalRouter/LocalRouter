//! The OS, CPU architecture and (on Linux) distribution we are running on,
//! used to pick which install commands to show.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Os {
    MacOs,
    Windows,
    Linux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Arch {
    Aarch64,
    X86_64,
    Other,
}

/// Linux distribution identity from `/etc/os-release`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct LinuxDistro {
    /// `ID`, e.g. `ubuntu`, `debian`, `arch`, `fedora`, `nixos`.
    pub id: String,
    /// `ID_LIKE` entries, e.g. `["debian"]` on Ubuntu.
    pub id_like: Vec<String>,
    /// `VERSION_ID`, e.g. `26.04`.
    pub version_id: Option<String>,
}

impl LinuxDistro {
    /// Whether the distro is `family` or derived from it.
    pub fn is(&self, family: &str) -> bool {
        self.id == family || self.id_like.iter().any(|l| l == family)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Platform {
    pub os: Os,
    pub arch: Arch,
    pub linux: Option<LinuxDistro>,
}

impl Platform {
    /// The platform this process is running on.
    pub fn current() -> Self {
        let os = if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        };
        let arch = match std::env::consts::ARCH {
            "aarch64" => Arch::Aarch64,
            "x86_64" => Arch::X86_64,
            _ => Arch::Other,
        };
        let linux = (os == Os::Linux).then(|| {
            std::fs::read_to_string("/etc/os-release")
                .map(|text| parse_os_release(&text))
                .unwrap_or_default()
        });
        Self { os, arch, linux }
    }

    pub fn is_apple_silicon(&self) -> bool {
        self.os == Os::MacOs && self.arch == Arch::Aarch64
    }

    pub fn is_intel_mac(&self) -> bool {
        self.os == Os::MacOs && self.arch == Arch::X86_64
    }
}

/// Parse the `KEY=value` lines of `/etc/os-release`.
pub fn parse_os_release(text: &str) -> LinuxDistro {
    let mut distro = LinuxDistro::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        match key.trim() {
            "ID" => distro.id = value.to_lowercase(),
            "ID_LIKE" => {
                distro.id_like = value.split_whitespace().map(|s| s.to_lowercase()).collect()
            }
            "VERSION_ID" => distro.version_id = Some(value.to_string()),
            _ => {}
        }
    }
    distro
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ubuntu_os_release() {
        let d =
            parse_os_release("NAME=\"Ubuntu\"\nVERSION_ID=\"26.04\"\nID=ubuntu\nID_LIKE=debian\n");
        assert_eq!(d.id, "ubuntu");
        assert!(d.is("debian"));
        assert_eq!(d.version_id.as_deref(), Some("26.04"));
    }

    #[test]
    fn parses_arch_like_distro() {
        let d = parse_os_release("ID=manjaro\nID_LIKE=\"arch\"\n");
        assert!(d.is("arch"));
        assert!(!d.is("debian"));
    }
}
