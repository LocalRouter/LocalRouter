//! Install recipes for the engines Local Embedded providers run.
//!
//! LocalRouter ships no engine binaries. Most recipes list the commands a user
//! runs with their own package manager, in the order we recommend for their
//! platform. The UI shows every command with a Copy button; commands that need
//! no password prompt can also be run by the app (Install button), but only
//! these compiled-in commands: the frontend refers to an option by recipe id
//! and option id, never by command text.
//!
//! stable-diffusion.cpp and Ollaya are in no package manager, so their
//! options are downloads instead: when the user clicks Download, LocalRouter
//! fetches the GitHub release asset for this platform ([`DownloadSpec`]) into
//! its managed install folder ([`crate::managed`]).

use serde::Serialize;

use crate::platform::{Arch, Os, Platform};

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

/// Ollaya release LocalRouter downloads. Ollaya ships several releases a
/// day, so the download is pinned (bump by PR after testing).
pub const OLLAYA_VERSION: &str = "v0.7.5";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipeId {
    /// llama.cpp's `llama-server` (or the unified `llama` binary).
    /// Serialized like [`RecipeId::as_str`] so the UI can pass it back.
    #[serde(rename = "llamacpp")]
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
    /// stable-diffusion.cpp's `sd-server`
    #[serde(rename = "sdcpp")]
    SdCpp,
    /// Ollaya's `ollaya` (serves many decision models)
    Ollaya,
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
            RecipeId::SdCpp => "sdcpp",
            RecipeId::Ollaya => "ollaya",
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
            "sdcpp" => Some(RecipeId::SdCpp),
            "ollaya" => Some(RecipeId::Ollaya),
            _ => None,
        }
    }
}

/// Whether an install option is a shell command or a download LocalRouter
/// performs itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallKind {
    /// A package-manager command (shown with Copy; runnable without sudo).
    Command,
    /// LocalRouter downloads a release asset into its managed install folder.
    Download,
}

/// Matches a GitHub release asset by name. The parts of asset names that
/// change between releases (tag, commit, OS and toolkit versions) are left
/// unmatched: a name matches when it starts with `prefix`, ends with
/// `suffix`, and contains every `contains` fragment in between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetPattern {
    pub prefix: &'static str,
    pub contains: &'static [&'static str],
    pub suffix: &'static str,
}

impl AssetPattern {
    pub fn matches(&self, name: &str) -> bool {
        if name.len() < self.prefix.len() + self.suffix.len()
            || !name.starts_with(self.prefix)
            || !name.ends_with(self.suffix)
        {
            return false;
        }
        let middle = &name[self.prefix.len()..name.len() - self.suffix.len()];
        self.contains.iter().all(|c| middle.contains(c))
    }
}

/// What a download option fetches: the latest (or the pinned `tag`) release
/// of `repo`, its asset matching `asset` (a `.zip` or `.tar.zst`), plus
/// `extras` archives extracted next to the executable (e.g. the CUDA runtime
/// DLLs) or, with `extras_at_root`, at the root of the release folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadSpec {
    /// `owner/name` on GitHub.
    pub repo: &'static str,
    /// A release tag to install instead of the latest release.
    pub tag: Option<&'static str>,
    /// Build name, used in the install folder name (e.g. `vulkan`).
    pub build: &'static str,
    pub asset: AssetPattern,
    pub extras: &'static [AssetPattern],
    /// Extract `extras` at the release root (archives laid out like the
    /// main one) instead of next to the executable.
    pub extras_at_root: bool,
    /// Executable to find inside the archive (without `.exe`).
    pub binary: &'static str,
}

/// One way to install an engine on this platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallOption {
    /// Stable id within the recipe, e.g. `brew`, `winget`, `apt`, `vulkan`.
    pub id: &'static str,
    /// Short label, e.g. "Homebrew".
    pub label: &'static str,
    pub kind: InstallKind,
    /// The exact command (command options only), shown to the user and (if
    /// runnable) run by the app.
    pub command: Option<String>,
    /// What a download option does, shown instead of a command.
    pub description: Option<String>,
    /// The program the command needs (e.g. `brew`), used to tell the user
    /// when their package manager is missing. `None` for downloads.
    pub program: Option<&'static str>,
    /// Needs `sudo`: shown for copying only, the app has no terminal for a
    /// password prompt.
    pub needs_sudo: bool,
    /// Extra guidance shown under the command.
    pub notes: Option<&'static str>,
    /// What to download (download options only; never sent to the UI).
    #[serde(skip)]
    pub download: Option<DownloadSpec>,
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
    /// A binary the user chose or built still runs where the recipe is
    /// unsupported (the reason only means "no prebuilt download").
    pub allow_own_binary: bool,
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
        kind: InstallKind::Command,
        command: Some(command.into()),
        description: None,
        program: Some(program),
        needs_sudo,
        notes,
        download: None,
    }
}

fn download(
    id: &'static str,
    label: &'static str,
    description: impl Into<String>,
    notes: Option<&'static str>,
    spec: DownloadSpec,
) -> InstallOption {
    InstallOption {
        id,
        label,
        kind: InstallKind::Download,
        command: None,
        description: Some(description.into()),
        program: None,
        needs_sudo: false,
        notes,
        download: Some(spec),
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
        RecipeId::SdCpp => sdcpp(platform),
        RecipeId::Ollaya => ollaya(platform),
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
        allow_own_binary: false,
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
        allow_own_binary: false,
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
        allow_own_binary: false,
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
        allow_own_binary: false,
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
        allow_own_binary: false,
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
        allow_own_binary: false,
    }
}

/// GitHub repository of stable-diffusion.cpp.
pub const SDCPP_REPO: &str = "leejet/stable-diffusion.cpp";

const SDCPP_NO_BUILD: &str = "No prebuilt stable-diffusion.cpp release for this platform; build it yourself and choose the sd-server file.";

const fn sd_asset(contains: &'static [&'static str], suffix: &'static str) -> AssetPattern {
    AssetPattern {
        prefix: "sd-",
        contains,
        suffix,
    }
}

const fn sd_spec(
    build: &'static str,
    asset: AssetPattern,
    extras: &'static [AssetPattern],
) -> DownloadSpec {
    DownloadSpec {
        repo: SDCPP_REPO,
        tag: None,
        build,
        asset,
        extras,
        extras_at_root: false,
        binary: "sd-server",
    }
}

/// `sd-master-<sha>-bin-Darwin-macOS-<ver>-arm64.zip`
pub const SDCPP_MACOS_METAL: DownloadSpec = sd_spec(
    "metal",
    sd_asset(&["-bin-Darwin-macOS-"], "-arm64.zip"),
    &[],
);
/// `sd-master-<sha>-bin-Linux-Ubuntu-<ver>-x86_64-vulkan.zip`
pub const SDCPP_LINUX_VULKAN: DownloadSpec = sd_spec(
    "vulkan",
    sd_asset(&["-bin-Linux-"], "-x86_64-vulkan.zip"),
    &[],
);
/// `sd-master-<sha>-bin-Linux-Ubuntu-<ver>-x86_64.zip`
pub const SDCPP_LINUX_CPU: DownloadSpec =
    sd_spec("cpu", sd_asset(&["-bin-Linux-"], "-x86_64.zip"), &[]);
/// `sd-master-<sha>-bin-Linux-Ubuntu-<ver>-x86_64-rocm-<rocm ver>.zip`
pub const SDCPP_LINUX_ROCM: DownloadSpec = sd_spec(
    "rocm",
    sd_asset(&["-bin-Linux-", "-x86_64-rocm-"], ".zip"),
    &[],
);
/// `sd-master-<sha>-bin-win-vulkan-x64.zip`
pub const SDCPP_WINDOWS_VULKAN: DownloadSpec =
    sd_spec("vulkan", sd_asset(&[], "-bin-win-vulkan-x64.zip"), &[]);
/// `sd-master-<sha>-bin-win-cuda12-x64.zip` plus the CUDA runtime DLLs from
/// `cudart-sd-bin-win-cu12-x64.zip`.
pub const SDCPP_WINDOWS_CUDA12: DownloadSpec = sd_spec(
    "cuda12",
    sd_asset(&[], "-bin-win-cuda12-x64.zip"),
    &[AssetPattern {
        prefix: "cudart-",
        contains: &["-win-cu12"],
        suffix: "-x64.zip",
    }],
);
/// `sd-master-<sha>-bin-win-cpu-x64.zip`
pub const SDCPP_WINDOWS_CPU: DownloadSpec =
    sd_spec("cpu", sd_asset(&[], "-bin-win-cpu-x64.zip"), &[]);
/// `sd-master-<sha>-bin-win-rocm-<rocm ver>-x64.zip`
pub const SDCPP_WINDOWS_ROCM: DownloadSpec =
    sd_spec("rocm", sd_asset(&["-bin-win-rocm-"], "-x64.zip"), &[]);

fn sd_download(
    id: &'static str,
    label: &'static str,
    notes: Option<&'static str>,
    spec: DownloadSpec,
) -> InstallOption {
    download(
        id,
        label,
        format!(
            "Downloads the latest stable-diffusion.cpp release ({label} build) from github.com/{SDCPP_REPO}"
        ),
        notes,
        spec,
    )
}

fn sdcpp(p: &Platform) -> EngineRecipe {
    let install = match (p.os, p.arch) {
        (Os::MacOs, Arch::Aarch64) => vec![sd_download(
            "metal",
            "Metal",
            Some("About 35 MB. Runs on the Apple GPU."),
            SDCPP_MACOS_METAL,
        )],
        (Os::Linux, Arch::X86_64) => vec![
            sd_download(
                "vulkan",
                "Vulkan",
                Some("Runs on NVIDIA, AMD and Intel GPUs; needs the GPU's Vulkan driver."),
                SDCPP_LINUX_VULKAN,
            ),
            sd_download(
                "cpu",
                "CPU",
                Some("No GPU needed, but much slower."),
                SDCPP_LINUX_CPU,
            ),
            sd_download(
                "rocm",
                "ROCm",
                Some("AMD GPUs with ROCm installed (about 260 MB)."),
                SDCPP_LINUX_ROCM,
            ),
        ],
        (Os::Windows, Arch::X86_64) => vec![
            sd_download(
                "vulkan",
                "Vulkan",
                Some("Runs on NVIDIA, AMD and Intel GPUs."),
                SDCPP_WINDOWS_VULKAN,
            ),
            sd_download(
                "cuda12",
                "CUDA 12",
                Some("NVIDIA GPUs; includes the CUDA runtime (about 900 MB)."),
                SDCPP_WINDOWS_CUDA12,
            ),
            sd_download(
                "cpu",
                "CPU",
                Some("No GPU needed, but much slower."),
                SDCPP_WINDOWS_CPU,
            ),
            sd_download(
                "rocm",
                "ROCm",
                Some("AMD GPUs (about 190 MB)."),
                SDCPP_WINDOWS_ROCM,
            ),
        ],
        _ => vec![],
    };
    let unsupported_reason = install.is_empty().then_some(SDCPP_NO_BUILD);
    EngineRecipe {
        id: RecipeId::SdCpp,
        display_name: "stable-diffusion.cpp",
        binaries: vec!["sd-server"],
        requires: vec![],
        install,
        docs_url: "https://github.com/leejet/stable-diffusion.cpp",
        unsupported_reason,
        allow_own_binary: true,
    }
}

/// GitHub repository of Ollaya.
pub const OLLAYA_REPO: &str = "ollaya-dev/ollaya";

const OLLAYA_INTEL_MAC: &str = "Ollaya has no build for Intel Macs.";
const OLLAYA_NO_BUILD: &str = "No Ollaya release for this platform.";

const fn ollaya_asset(suffix: &'static str) -> AssetPattern {
    AssetPattern {
        prefix: "ollaya-",
        contains: &[],
        suffix,
    }
}

const fn ollaya_spec(
    build: &'static str,
    asset: AssetPattern,
    extras: &'static [AssetPattern],
) -> DownloadSpec {
    DownloadSpec {
        repo: OLLAYA_REPO,
        tag: Some(OLLAYA_VERSION),
        build,
        asset,
        extras,
        extras_at_root: true,
        binary: "ollaya",
    }
}

/// `ollaya-darwin-arm64.tar.zst` plus the Apple GPU (MLX) kernels, which
/// Ollaya looks for in `lib/ollaya/mlx_metal` next to its `bin/`.
pub const OLLAYA_MACOS: DownloadSpec = ollaya_spec(
    "metal",
    ollaya_asset("darwin-arm64.tar.zst"),
    &[ollaya_asset("darwin-arm64-mlx.tar.zst")],
);
pub const OLLAYA_LINUX_CPU: DownloadSpec =
    ollaya_spec("cpu", ollaya_asset("linux-amd64.tar.zst"), &[]);
/// The CPU release plus its CUDA 13 pack (`lib/ollaya/cuda_v13`).
pub const OLLAYA_LINUX_CUDA: DownloadSpec = ollaya_spec(
    "cuda",
    ollaya_asset("linux-amd64.tar.zst"),
    &[ollaya_asset("linux-amd64-cuda.tar.zst")],
);
pub const OLLAYA_LINUX_ARM64: DownloadSpec =
    ollaya_spec("cpu", ollaya_asset("linux-arm64.tar.zst"), &[]);
pub const OLLAYA_WINDOWS_CPU: DownloadSpec =
    ollaya_spec("cpu", ollaya_asset("windows-amd64.zip"), &[]);
pub const OLLAYA_WINDOWS_CUDA: DownloadSpec = ollaya_spec(
    "cuda",
    ollaya_asset("windows-amd64.zip"),
    &[ollaya_asset("windows-amd64-cuda.zip")],
);

fn ollaya_download(
    id: &'static str,
    label: &'static str,
    notes: Option<&'static str>,
    spec: DownloadSpec,
) -> InstallOption {
    download(
        id,
        label,
        format!("Downloads Ollaya {OLLAYA_VERSION} ({label} build) from github.com/{OLLAYA_REPO}"),
        notes,
        spec,
    )
}

const OLLAYA_GLIBC: &str =
    "Needs glibc 2.38 or newer (Ubuntu 24.04, Debian 13, Fedora 39 or newer).";

fn ollaya(p: &Platform) -> EngineRecipe {
    let install = match (p.os, p.arch) {
        (Os::MacOs, Arch::Aarch64) => vec![ollaya_download(
            "metal",
            "Apple Silicon",
            Some("About 25 MB. Needs macOS 14 or newer."),
            OLLAYA_MACOS,
        )],
        (Os::Linux, Arch::X86_64) => vec![
            ollaya_download("cpu", "CPU", Some(OLLAYA_GLIBC), OLLAYA_LINUX_CPU),
            ollaya_download(
                "cuda",
                "NVIDIA CUDA",
                Some("Adds the CUDA runtime (about 1.5 GB); needs NVIDIA driver R580 or newer. Also needs glibc 2.38 or newer."),
                OLLAYA_LINUX_CUDA,
            ),
        ],
        (Os::Linux, Arch::Aarch64) => vec![ollaya_download(
            "cpu",
            "CPU",
            Some(OLLAYA_GLIBC),
            OLLAYA_LINUX_ARM64,
        )],
        (Os::Windows, Arch::X86_64) => vec![
            ollaya_download(
                "cpu",
                "CPU",
                Some("Needs the Microsoft Visual C++ runtime."),
                OLLAYA_WINDOWS_CPU,
            ),
            ollaya_download(
                "cuda",
                "NVIDIA CUDA",
                Some("Adds the CUDA runtime (about 1.5 GB); needs NVIDIA driver R580 or newer."),
                OLLAYA_WINDOWS_CUDA,
            ),
        ],
        _ => vec![],
    };
    let unsupported_reason = if p.is_intel_mac() {
        Some(OLLAYA_INTEL_MAC)
    } else {
        install.is_empty().then_some(OLLAYA_NO_BUILD)
    };
    EngineRecipe {
        id: RecipeId::Ollaya,
        display_name: "Ollaya",
        binaries: vec!["ollaya"],
        requires: vec![],
        install,
        docs_url: "https://github.com/ollaya-dev/ollaya",
        unsupported_reason,
        allow_own_binary: true,
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
        assert_eq!(
            r.install[0].command.as_deref(),
            Some("brew install llama.cpp")
        );
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
        assert!(r.install[0]
            .command
            .as_deref()
            .unwrap()
            .contains("--accept-package-agreements"));
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
        assert!(r.install[0]
            .command
            .as_deref()
            .unwrap()
            .contains(KEV_GIT_REV));
        let args = kev_uv_run_args(&["-m", "kev.serve"]);
        assert_eq!(&args[..2], &["tool", "run"]);
        assert!(args.contains(&kev_from_spec()));
        assert_eq!(args.last().unwrap(), "kev.serve");
    }

    #[test]
    fn decider_adds_metal_on_apple_silicon() {
        let arm = recipe(RecipeId::Decider, &plat(Os::MacOs, Arch::Aarch64, None));
        assert!(arm.install[0]
            .command
            .as_deref()
            .unwrap()
            .contains("decider-ai[serve,metal]"));
        let linux = recipe(RecipeId::Decider, &plat(Os::Linux, Arch::X86_64, None));
        assert!(linux.install[0]
            .command
            .as_deref()
            .unwrap()
            .contains("\"decider-ai[serve]\""));
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
            RecipeId::SdCpp,
            RecipeId::Ollaya,
        ] {
            assert_eq!(RecipeId::parse(id.as_str()), Some(id));
            assert_eq!(
                serde_json::to_value(id).unwrap(),
                serde_json::Value::String(id.as_str().to_string())
            );
        }
        assert_eq!(RecipeId::parse("nope"), None);
    }

    /// Asset names of stable-diffusion.cpp release master-920-2f88688.
    const SD_ASSETS: &[&str] = &[
        "cudart-sd-bin-win-cu12-x64.zip",
        "sd-master-2f88688-bin-Darwin-macOS-26.6.2-arm64.zip",
        "sd-master-2f88688-bin-Linux-Ubuntu-24.04-x86_64-rocm-7.14.0.zip",
        "sd-master-2f88688-bin-Linux-Ubuntu-24.04-x86_64-vulkan.zip",
        "sd-master-2f88688-bin-Linux-Ubuntu-24.04-x86_64.zip",
        "sd-master-2f88688-bin-win-cpu-x64.zip",
        "sd-master-2f88688-bin-win-cuda12-x64.zip",
        "sd-master-2f88688-bin-win-rocm-7.14.0-x64.zip",
        "sd-master-2f88688-bin-win-vulkan-x64.zip",
    ];

    fn only_match(pattern: &AssetPattern) -> &'static str {
        let hits: Vec<_> = SD_ASSETS
            .iter()
            .copied()
            .filter(|n| pattern.matches(n))
            .collect();
        assert_eq!(hits.len(), 1, "{pattern:?} matched {hits:?}");
        hits[0]
    }

    #[test]
    fn sdcpp_asset_patterns_pick_exactly_one_real_asset() {
        let cases = [
            (
                SDCPP_MACOS_METAL,
                "sd-master-2f88688-bin-Darwin-macOS-26.6.2-arm64.zip",
            ),
            (
                SDCPP_LINUX_VULKAN,
                "sd-master-2f88688-bin-Linux-Ubuntu-24.04-x86_64-vulkan.zip",
            ),
            (
                SDCPP_LINUX_CPU,
                "sd-master-2f88688-bin-Linux-Ubuntu-24.04-x86_64.zip",
            ),
            (
                SDCPP_LINUX_ROCM,
                "sd-master-2f88688-bin-Linux-Ubuntu-24.04-x86_64-rocm-7.14.0.zip",
            ),
            (
                SDCPP_WINDOWS_VULKAN,
                "sd-master-2f88688-bin-win-vulkan-x64.zip",
            ),
            (
                SDCPP_WINDOWS_CUDA12,
                "sd-master-2f88688-bin-win-cuda12-x64.zip",
            ),
            (SDCPP_WINDOWS_CPU, "sd-master-2f88688-bin-win-cpu-x64.zip"),
            (
                SDCPP_WINDOWS_ROCM,
                "sd-master-2f88688-bin-win-rocm-7.14.0-x64.zip",
            ),
        ];
        for (spec, expected) in cases {
            assert_eq!(only_match(&spec.asset), expected);
            assert_eq!(spec.binary, "sd-server");
        }
        assert_eq!(
            only_match(&SDCPP_WINDOWS_CUDA12.extras[0]),
            "cudart-sd-bin-win-cu12-x64.zip"
        );
    }

    /// Asset names of Ollaya release v0.7.5 (checksums and desktop apps
    /// included, which must never match).
    const OLLAYA_ASSETS: &[&str] = &[
        "ollaya-darwin-arm64-mlx.sha256",
        "ollaya-darwin-arm64-mlx.tar.zst",
        "ollaya-darwin-arm64-mlx.tgz",
        "ollaya-darwin-arm64.tar.zst",
        "ollaya-darwin-arm64.tgz",
        "ollaya-linux-amd64-cuda.sha256",
        "ollaya-linux-amd64-cuda.tar.zst",
        "ollaya-linux-amd64-cuda12.sha256",
        "ollaya-linux-amd64-cuda12.tar.zst",
        "Ollaya-linux-amd64.deb",
        "ollaya-linux-amd64.tar.zst",
        "ollaya-linux-arm64.tar.zst",
        "Ollaya-linux-x86_64.AppImage",
        "Ollaya-linux-x86_64.rpm",
        "Ollaya-macos-arm64.dmg",
        "ollaya-windows-amd64-cuda.sha256",
        "ollaya-windows-amd64-cuda.zip",
        "ollaya-windows-amd64-cuda12.sha256",
        "ollaya-windows-amd64-cuda12.zip",
        "ollaya-windows-amd64.zip",
        "Ollaya-windows-x64-setup.exe",
        "Ollaya-windows-x64.msi",
        "sha256sum.txt",
    ];

    fn only_ollaya_match(pattern: &AssetPattern) -> &'static str {
        let hits: Vec<_> = OLLAYA_ASSETS
            .iter()
            .copied()
            .filter(|n| pattern.matches(n))
            .collect();
        assert_eq!(hits.len(), 1, "{pattern:?} matched {hits:?}");
        hits[0]
    }

    #[test]
    fn ollaya_downloads_are_pinned_and_pick_exactly_one_asset() {
        let cases: [(DownloadSpec, &str, &[&str]); 6] = [
            (
                OLLAYA_MACOS,
                "ollaya-darwin-arm64.tar.zst",
                &["ollaya-darwin-arm64-mlx.tar.zst"],
            ),
            (OLLAYA_LINUX_CPU, "ollaya-linux-amd64.tar.zst", &[]),
            (
                OLLAYA_LINUX_CUDA,
                "ollaya-linux-amd64.tar.zst",
                &["ollaya-linux-amd64-cuda.tar.zst"],
            ),
            (OLLAYA_LINUX_ARM64, "ollaya-linux-arm64.tar.zst", &[]),
            (OLLAYA_WINDOWS_CPU, "ollaya-windows-amd64.zip", &[]),
            (
                OLLAYA_WINDOWS_CUDA,
                "ollaya-windows-amd64.zip",
                &["ollaya-windows-amd64-cuda.zip"],
            ),
        ];
        for (spec, main, extras) in cases {
            assert_eq!(only_ollaya_match(&spec.asset), main);
            let got: Vec<_> = spec.extras.iter().map(only_ollaya_match).collect();
            assert_eq!(got, extras);
            assert_eq!(spec.tag, Some(OLLAYA_VERSION));
            assert!(spec.extras_at_root);
            assert_eq!(spec.binary, "ollaya");
        }
    }

    #[test]
    fn ollaya_platforms() {
        let mac = recipe(RecipeId::Ollaya, &plat(Os::MacOs, Arch::Aarch64, None));
        assert!(mac.unsupported_reason.is_none());
        assert!(mac.requires.is_empty());
        assert_eq!(mac.install.len(), 1);
        assert_eq!(mac.install[0].kind, InstallKind::Download);
        let intel = recipe(RecipeId::Ollaya, &plat(Os::MacOs, Arch::X86_64, None));
        assert!(intel.unsupported_reason.is_some() && intel.install.is_empty());
        let linux = recipe(RecipeId::Ollaya, &plat(Os::Linux, Arch::X86_64, None));
        assert_eq!(
            linux.install.iter().map(|o| o.id).collect::<Vec<_>>(),
            ["cpu", "cuda"]
        );
        let win = recipe(RecipeId::Ollaya, &plat(Os::Windows, Arch::X86_64, None));
        assert_eq!(win.install.len(), 2);
        assert!(win.install.iter().all(|o| o.runnable()));
    }

    #[test]
    fn sdcpp_asset_patterns_survive_version_changes() {
        // A later tag, a newer Ubuntu/macOS/ROCm version.
        assert!(SDCPP_LINUX_VULKAN
            .asset
            .matches("sd-master-1234-abcdef0-bin-Linux-Ubuntu-26.04-x86_64-vulkan.zip"));
        assert!(SDCPP_MACOS_METAL
            .asset
            .matches("sd-master-abcdef0-bin-Darwin-macOS-27.1-arm64.zip"));
        assert!(SDCPP_WINDOWS_ROCM
            .asset
            .matches("sd-master-abcdef0-bin-win-rocm-7.20.1-x64.zip"));
        // Other architectures and builds do not match.
        assert!(!SDCPP_LINUX_CPU
            .asset
            .matches("sd-master-abcdef0-bin-Linux-Ubuntu-24.04-aarch64.zip"));
        assert!(!SDCPP_LINUX_CPU
            .asset
            .matches("sd-master-abcdef0-bin-Linux-Ubuntu-24.04-x86_64-vulkan.zip"));
        assert!(!SDCPP_WINDOWS_CPU
            .asset
            .matches("sd-master-abcdef0-bin-win-cpu-arm64.zip"));
        // Prefix and suffix may not overlap.
        let p = AssetPattern {
            prefix: "sd-",
            contains: &[],
            suffix: "-x64.zip",
        };
        assert!(!p.matches("sd-x64.zip"));
    }

    #[test]
    fn sdcpp_offers_downloads_per_platform() {
        let ids = |p: &Platform| -> Vec<&'static str> {
            recipe(RecipeId::SdCpp, p)
                .install
                .iter()
                .map(|o| o.id)
                .collect()
        };
        assert_eq!(ids(&plat(Os::MacOs, Arch::Aarch64, None)), vec!["metal"]);
        assert_eq!(
            ids(&plat(Os::Linux, Arch::X86_64, Some("ID=ubuntu\n"))),
            vec!["vulkan", "cpu", "rocm"]
        );
        assert_eq!(
            ids(&plat(Os::Windows, Arch::X86_64, None)),
            vec!["vulkan", "cuda12", "cpu", "rocm"]
        );
        let win = recipe(RecipeId::SdCpp, &plat(Os::Windows, Arch::X86_64, None));
        assert!(win.unsupported_reason.is_none());
        assert!(win.allow_own_binary);
        for o in &win.install {
            assert_eq!(o.kind, InstallKind::Download);
            assert!(o.command.is_none() && o.program.is_none());
            assert!(o.runnable());
            assert!(o.download.is_some());
            assert!(o
                .description
                .as_deref()
                .unwrap()
                .contains("github.com/leejet/stable-diffusion.cpp"));
        }
        let json = serde_json::to_value(&win.install[0]).unwrap();
        assert_eq!(json["kind"], "download");
        assert!(json.get("download").is_none(), "spec is not serialized");

        for p in [
            plat(Os::MacOs, Arch::X86_64, None),
            plat(Os::Linux, Arch::Aarch64, None),
            plat(Os::Windows, Arch::Aarch64, None),
        ] {
            let r = recipe(RecipeId::SdCpp, &p);
            assert!(r.install.is_empty());
            assert!(r
                .unsupported_reason
                .unwrap()
                .contains("choose the sd-server file"));
        }
    }

    #[test]
    fn command_options_serialize_their_kind() {
        let r = recipe(RecipeId::LlamaCpp, &plat(Os::MacOs, Arch::Aarch64, None));
        let json = serde_json::to_value(&r.install[0]).unwrap();
        assert_eq!(json["kind"], "command");
        assert_eq!(json["command"], "brew install llama.cpp");
        assert_eq!(json["program"], "brew");
        assert!(json["description"].is_null());
    }
}
