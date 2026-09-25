//! Install recipes for the engines Local Embedded providers run.
//!
//! LocalRouter never downloads or ships engine binaries. Each recipe lists the
//! commands a user runs with their own package manager, in the order we
//! recommend for their platform. The UI shows every command with a Copy button;
//! commands that need no password prompt can also be run by the app (Install
//! button), but only these compiled-in commands: the frontend refers to an
//! option by recipe id and option id, never by command text.

use serde::Serialize;

use crate::platform::{Os, Platform};

/// Kev has no PyPI package or console script, so it is run from its Git
/// repository through uv, pinned to this commit (bump by PR after testing).
pub const KEV_GIT_REV: &str = "eb45fd2381396eb7edc3964b753ebc1b0ab1da2b";

/// Python version Kev runs on (it requires >=3.12,<3.14).
pub const KEV_PYTHON: &str = "3.13";

/// Python version for the Laya tool environment.
pub const LAYA_PYTHON: &str = "3.12";

/// Python version for Von (requires >=3.12).
pub const VON_PYTHON: &str = "3.12";

/// Python version for Decider (requires >=3.11; its README uses 3.12).
pub const DECIDER_PYTHON: &str = "3.12";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipeId {
    /// llama.cpp's `llama-server` (or the unified `llama` binary)
    LlamaCpp,
    /// Astral's uv, used to install and run Laya and Kev
    Uv,
    /// Laya's `laya-serve`
    Laya,
    /// Kev, run through uv from its Git repository
    Kev,
    /// Von's `von` command (`uv tool install von-sdk`)
    Von,
    /// Decider, run through uv (`decider-ai` has no console script)
    Decider,
}

impl RecipeId {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecipeId::LlamaCpp => "llamacpp",
            RecipeId::Uv => "uv",
            RecipeId::Laya => "laya",
            RecipeId::Kev => "kev",
            RecipeId::Von => "von",
            RecipeId::Decider => "decider",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "llamacpp" => Some(RecipeId::LlamaCpp),
            "uv" => Some(RecipeId::Uv),
            "laya" => Some(RecipeId::Laya),
            "kev" => Some(RecipeId::Kev),
            "von" => Some(RecipeId::Von),
            "decider" => Some(RecipeId::Decider),
            _ => None,
        }
    }
}

/// One way to install an engine on this platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallOption {
    /// Stable id within the recipe, e.g. `brew`, `winget`, `apt`.
    pub id: &'static str,
    /// Short label, e.g. "Homebrew".
    pub label: &'static str,
    /// The exact command, shown to the user and (if runnable) run by the app.
    pub command: String,
    /// The program the command needs (e.g. `brew`), used to tell the user
    /// when their package manager is missing.
    pub program: &'static str,
    /// Needs `sudo`: shown for copying only, the app has no terminal for a
    /// password prompt.
    pub needs_sudo: bool,
    /// Extra guidance shown under the command.
    pub notes: Option<&'static str>,
}

impl InstallOption {
    pub fn runnable(&self) -> bool {
        !self.needs_sudo
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineRecipe {
    pub id: RecipeId,
    pub display_name: &'static str,
    /// Executables that satisfy this recipe, in preference order.
    pub binaries: Vec<&'static str>,
    /// Recipes that must be installed first.
    pub requires: Vec<RecipeId>,
    /// Install options for this platform, recommended first.
    pub install: Vec<InstallOption>,
    pub docs_url: &'static str,
    /// Why the engine can't run on this platform, if it can't.
    pub unsupported_reason: Option<&'static str>,
}

fn opt(
    id: &'static str,
    label: &'static str,
    command: impl Into<String>,
    program: &'static str,
    needs_sudo: bool,
    notes: Option<&'static str>,
) -> InstallOption {
    InstallOption {
        id,
        label,
        command: command.into(),
        program,
        needs_sudo,
        notes,
    }
}

/// The `--from` spec for Kev's pinned Git revision.
pub fn kev_from_spec() -> String {
    format!("kev[serve] @ git+https://github.com/jaredpalmer/kev@{KEV_GIT_REV}")
}

/// Arguments (after `uv`) that run a Python command inside Kev's environment.
pub fn kev_uv_run_args(python_args: &[&str]) -> Vec<String> {
    let mut args = vec![
        "tool".to_string(),
        "run".to_string(),
        "--python".to_string(),
        KEV_PYTHON.to_string(),
        "--from".to_string(),
        kev_from_spec(),
        "python".to_string(),
    ];
    args.extend(python_args.iter().map(|s| s.to_string()));
    args
}

/// The `--from` spec for Decider. Apple Silicon adds the `metal` extra
/// (MLX kernels).
pub fn decider_from_spec(apple_silicon: bool) -> String {
    if apple_silicon {
        "decider-ai[serve,metal]".to_string()
    } else {
        "decider-ai[serve]".to_string()
    }
}

/// Arguments (after `uv`) that run Decider's server through uvicorn, which
/// is a dependency of `decider-ai[serve]`.
pub fn decider_uv_run_args(apple_silicon: bool) -> Vec<String> {
    vec![
        "tool".into(),
        "run".into(),
        "--python".into(),
        DECIDER_PYTHON.into(),
        "--from".into(),
        decider_from_spec(apple_silicon),
        "uvicorn".into(),
        "decider.serve:app".into(),
        "--host".into(),
        "127.0.0.1".into(),
    ]
}

pub fn recipe(id: RecipeId, platform: &Platform) -> EngineRecipe {
    match id {
        RecipeId::LlamaCpp => llamacpp(platform),
        RecipeId::Uv => uv(platform),
        RecipeId::Laya => laya(platform),
        RecipeId::Kev => kev(platform),
        RecipeId::Von => von(platform),
        RecipeId::Decider => decider(platform),
    }
}

fn llamacpp(p: &Platform) -> EngineRecipe {
    let install = match p.os {
        Os::MacOs => {
            let brew_notes = if p.is_intel_mac() {
                Some("Intel Macs build llama.cpp from source (this can take a while) and run it on the CPU.")
            } else {
                None
            };
            vec![
                opt("brew", "Homebrew", "brew install llama.cpp", "brew", false, brew_notes),
                opt("macports", "MacPorts", "sudo port install llama.cpp", "port", true, None),
            ]
        }
        Os::Windows => vec![
            opt(
                "winget",
                "WinGet",
                "winget install --id ggml.llamacpp -e --accept-source-agreements --accept-package-agreements",
                "winget",
                false,
                Some("Installs the Vulkan build, which runs on NVIDIA, AMD and Intel GPUs."),
            ),
            opt(
                "scoop",
                "Scoop",
                "scoop bucket add versions; scoop install versions/llama.cpp-vulkan",
                "scoop",
                false,
                Some("Other builds: versions/llama.cpp-cu13 (NVIDIA CUDA), versions/llama.cpp-cpu."),
            ),
        ],
        Os::Linux => {
            let distro = p.linux.clone().unwrap_or_default();
            let mut options = Vec::new();
            let brew = opt(
                "brew",
                "Homebrew",
                "brew install llama.cpp",
                "brew",
                false,
                Some("Works on any distribution with Homebrew installed."),
            );
            let apt = opt(
                "apt",
                "apt",
                "sudo apt install llama.cpp libggml0-backend-vulkan",
                "apt",
                true,
                Some("Ubuntu 26.04 or newer, or Debian testing."),
            );
            let pacman = opt(
                "pacman",
                "pacman",
                "sudo pacman -S llama-cpp ggml-vulkan",
                "pacman",
                true,
                Some("Use ggml-cuda or ggml-hip instead of ggml-vulkan for CUDA or ROCm."),
            );
            let nix = opt(
                "nix",
                "Nix",
                "nix profile add nixpkgs#llama-cpp-vulkan",
                "nix",
                false,
                None,
            );
            if distro.is("arch") {
                options.push(pacman.clone());
            } else if distro.is("nixos") {
                options.push(nix.clone());
            } else if distro.is("debian") || distro.is("ubuntu") {
                options.push(apt.clone());
            }
            options.push(brew);
            for o in [apt, pacman, nix] {
                if !options.iter().any(|x| x.id == o.id) {
                    options.push(o);
                }
            }
            options
        }
    };
    EngineRecipe {
        id: RecipeId::LlamaCpp,
        display_name: "llama.cpp",
        binaries: vec!["llama-server", "llama"],
        requires: vec![],
        install,
        docs_url: "https://github.com/ggml-org/llama.cpp/blob/master/docs/install.md",
        unsupported_reason: None,
    }
}

fn uv(p: &Platform) -> EngineRecipe {
    let install = match p.os {
        Os::Windows => vec![
            opt(
                "winget",
                "WinGet",
                "winget install --id astral-sh.uv -e --accept-source-agreements --accept-package-agreements",
                "winget",
                false,
                None,
            ),
            opt(
                "powershell",
                "Installer script",
                "powershell -ExecutionPolicy ByPass -c \"irm https://astral.sh/uv/install.ps1 | iex\"",
                "powershell",
                false,
                None,
            ),
        ],
        Os::MacOs | Os::Linux => {
            let script = opt(
                "script",
                "Installer script",
                "curl -LsSf https://astral.sh/uv/install.sh | sh",
                "curl",
                false,
                Some("Installs uv into ~/.local/bin."),
            );
            let brew = opt("brew", "Homebrew", "brew install uv", "brew", false, None);
            if p.is_apple_silicon() {
                vec![brew, script]
            } else {
                vec![script, brew]
            }
        }
    };
    EngineRecipe {
        id: RecipeId::Uv,
        display_name: "uv",
        binaries: vec!["uv"],
        requires: vec![],
        install,
        docs_url: "https://docs.astral.sh/uv/getting-started/installation/",
        unsupported_reason: None,
    }
}

const NO_INTEL_MAC_PYTORCH: &str =
    "Not available on Intel Macs: current PyTorch releases no longer support them.";

fn laya(p: &Platform) -> EngineRecipe {
    EngineRecipe {
        id: RecipeId::Laya,
        display_name: "Laya",
        binaries: vec!["laya-serve"],
        requires: vec![RecipeId::Uv],
        install: vec![
            opt(
                "uv-tool",
                "uv",
                format!("uv tool install --python {LAYA_PYTHON} \"laya[serve]\""),
                "uv",
                false,
                Some("Installs the official laya package and its laya-serve command (downloads PyTorch, about 1-3 GB)."),
            ),
            opt(
                "uv-tool-gpu",
                "uv, GPU-matched PyTorch",
                format!("uv tool install --python {LAYA_PYTHON} --torch-backend auto \"laya[serve]\""),
                "uv",
                false,
                Some("Picks the PyTorch build matching your GPU driver (CUDA) instead of the default."),
            ),
        ],
        docs_url: "https://github.com/NandhaKishorM/laya",
        unsupported_reason: p.is_intel_mac().then_some(NO_INTEL_MAC_PYTORCH),
    }
}

fn kev(p: &Platform) -> EngineRecipe {
    let prepare = format!(
        "uv tool run --python {KEV_PYTHON} --from \"{}\" python -c \"import kev.serve; print('Kev is installed and ready.')\"",
        kev_from_spec()
    );
    EngineRecipe {
        id: RecipeId::Kev,
        display_name: "Kev",
        // Kev runs through uv; there is no Kev executable to find.
        binaries: vec!["uv"],
        requires: vec![RecipeId::Uv],
        install: vec![opt(
            "prepare",
            "Prepare Kev",
            prepare,
            "uv",
            false,
            Some("Downloads Kev and PyTorch (several GB) into uv's cache so the first start is quick. Optional: the first start does this anyway."),
        )],
        docs_url: "https://github.com/jaredpalmer/kev",
        unsupported_reason: p.is_intel_mac().then_some(NO_INTEL_MAC_PYTORCH),
    }
}

fn von(p: &Platform) -> EngineRecipe {
    EngineRecipe {
        id: RecipeId::Von,
        display_name: "Von",
        binaries: vec!["von"],
        requires: vec![RecipeId::Uv],
        install: vec![opt(
            "uv-tool",
            "uv",
            format!("uv tool install --python {VON_PYTHON} von-sdk"),
            "uv",
            false,
            Some("Installs the von command (downloads PyTorch, about 1-3 GB). The model (about 3 GB) downloads when it first loads."),
        )],
        docs_url: "https://github.com/wfzyx/von",
        unsupported_reason: p.is_intel_mac().then_some(NO_INTEL_MAC_PYTORCH),
    }
}

fn decider(p: &Platform) -> EngineRecipe {
    let prepare = format!(
        "uv tool run --python {DECIDER_PYTHON} --from \"{}\" python -c \"import decider.serve; print('Decider is installed and ready.')\"",
        decider_from_spec(p.is_apple_silicon())
    );
    EngineRecipe {
        id: RecipeId::Decider,
        display_name: "Decider",
        // Decider runs through uv; it has no executable of its own.
        binaries: vec!["uv"],
        requires: vec![RecipeId::Uv],
        install: vec![opt(
            "prepare",
            "Prepare Decider",
            prepare,
            "uv",
            false,
            Some("Downloads Decider and PyTorch into uv's cache so the first start is quick. Optional: the first start does this anyway."),
        )],
        docs_url: "https://github.com/Mapika/decider",
        unsupported_reason: p.is_intel_mac().then_some(NO_INTEL_MAC_PYTORCH),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{parse_os_release, Arch};

    fn plat(os: Os, arch: Arch, os_release: Option<&str>) -> Platform {
        Platform {
            os,
            arch,
            linux: os_release.map(parse_os_release),
        }
    }

    #[test]
    fn macos_llamacpp_uses_homebrew_first() {
        let r = recipe(RecipeId::LlamaCpp, &plat(Os::MacOs, Arch::Aarch64, None));
        assert_eq!(r.install[0].command, "brew install llama.cpp");
        assert!(r.install[0].runnable());
        assert!(r.install[0].notes.is_none());
        assert!(!r
            .install
            .iter()
            .find(|o| o.id == "macports")
            .unwrap()
            .runnable());
        let intel = recipe(RecipeId::LlamaCpp, &plat(Os::MacOs, Arch::X86_64, None));
        assert!(intel.install[0].notes.unwrap().contains("from source"));
    }

    #[test]
    fn linux_llamacpp_prefers_the_distro_package() {
        let arch = recipe(
            RecipeId::LlamaCpp,
            &plat(Os::Linux, Arch::X86_64, Some("ID=arch\n")),
        );
        assert_eq!(arch.install[0].id, "pacman");
        let ubuntu = recipe(
            RecipeId::LlamaCpp,
            &plat(Os::Linux, Arch::X86_64, Some("ID=ubuntu\nID_LIKE=debian\n")),
        );
        assert_eq!(ubuntu.install[0].id, "apt");
        let other = recipe(
            RecipeId::LlamaCpp,
            &plat(Os::Linux, Arch::X86_64, Some("ID=gentoo\n")),
        );
        assert_eq!(other.install[0].id, "brew");
        // Every option appears exactly once.
        let ids: Vec<_> = ubuntu.install.iter().map(|o| o.id).collect();
        let mut dedup = ids.clone();
        dedup.dedup();
        assert_eq!(ids.len(), dedup.len());
    }

    #[test]
    fn windows_uses_winget_non_interactively() {
        let r = recipe(RecipeId::LlamaCpp, &plat(Os::Windows, Arch::X86_64, None));
        assert!(r.install[0].command.contains("--accept-package-agreements"));
        assert!(r.install.iter().all(|o| !o.needs_sudo));
    }

    #[test]
    fn python_engines_need_uv_and_skip_intel_macs() {
        for id in [
            RecipeId::Laya,
            RecipeId::Kev,
            RecipeId::Von,
            RecipeId::Decider,
        ] {
            let r = recipe(id, &plat(Os::MacOs, Arch::Aarch64, None));
            assert_eq!(r.requires, vec![RecipeId::Uv]);
            assert!(r.unsupported_reason.is_none());
            let intel = recipe(id, &plat(Os::MacOs, Arch::X86_64, None));
            assert!(intel.unsupported_reason.is_some());
        }
    }

    #[test]
    fn kev_is_pinned_to_a_commit() {
        let r = recipe(RecipeId::Kev, &plat(Os::Linux, Arch::X86_64, None));
        assert!(r.install[0].command.contains(KEV_GIT_REV));
        let args = kev_uv_run_args(&["-m", "kev.serve"]);
        assert_eq!(&args[..2], &["tool", "run"]);
        assert!(args.contains(&kev_from_spec()));
        assert_eq!(args.last().unwrap(), "kev.serve");
    }

    #[test]
    fn decider_adds_metal_on_apple_silicon() {
        let arm = recipe(RecipeId::Decider, &plat(Os::MacOs, Arch::Aarch64, None));
        assert!(arm.install[0].command.contains("decider-ai[serve,metal]"));
        let linux = recipe(RecipeId::Decider, &plat(Os::Linux, Arch::X86_64, None));
        assert!(linux.install[0].command.contains("\"decider-ai[serve]\""));
        let args = decider_uv_run_args(false);
        assert_eq!(&args[..2], &["tool", "run"]);
        assert!(args.windows(2).any(|w| w == ["--host", "127.0.0.1"]));
        assert!(args.contains(&"decider.serve:app".to_string()));
    }

    #[test]
    fn recipe_ids_round_trip() {
        for id in [
            RecipeId::LlamaCpp,
            RecipeId::Uv,
            RecipeId::Laya,
            RecipeId::Kev,
            RecipeId::Von,
            RecipeId::Decider,
        ] {
            assert_eq!(RecipeId::parse(id.as_str()), Some(id));
        }
        assert_eq!(RecipeId::parse("nope"), None);
    }
}
