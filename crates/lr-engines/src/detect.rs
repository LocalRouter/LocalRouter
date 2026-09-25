//! Finding installed engines and describing how to install them.
//!
//! Lookup order: the provider's chosen file (`binary_path`), LocalRouter's
//! managed install ([`crate::managed`]), then the user's PATH.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;

use crate::managed::ManagedInstall;
use crate::platform::Platform;
use crate::process::host_command;
use crate::recipes::{kev_uv_run_args, recipe, EngineRecipe, InstallKind, InstallOption, RecipeId};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// An install option plus what we know about it on this machine.
#[derive(Debug, Clone, Serialize)]
pub struct InstallOptionView {
    #[serde(flatten)]
    pub option: InstallOption,
    /// The app may run this option (no password prompt needed).
    pub runnable: bool,
    /// The package manager the command uses was found on PATH (always true
    /// for downloads).
    pub program_found: bool,
    /// The option we suggest for this machine.
    pub recommended: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequirementStatus {
    pub recipe: RecipeId,
    pub display_name: &'static str,
    pub found: bool,
    pub path: Option<String>,
}

/// Where an engine's executable was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineSource {
    /// The file chosen in the provider's settings (`binary_path`).
    Override,
    /// LocalRouter's managed install.
    Managed,
    /// The user's PATH.
    Path,
}

/// Detection result for one engine, with everything the Engine tab shows.
#[derive(Debug, Clone, Serialize)]
pub struct EngineStatus {
    pub recipe: RecipeId,
    pub display_name: &'static str,
    /// The engine can be launched: its executable (and requirements) exist.
    pub found: bool,
    pub path: Option<String>,
    /// Which of the recipe's executables was found (e.g. `llama-server`).
    pub binary: Option<String>,
    /// Where `path` came from.
    pub source: Option<EngineSource>,
    /// Release tag of LocalRouter's managed install, when there is one
    /// (even if a chosen file takes precedence).
    pub managed_tag: Option<String>,
    /// Build of the managed install, e.g. `vulkan`.
    pub managed_build: Option<String>,
    pub version: Option<String>,
    /// llama.cpp build number, when reported.
    pub build: Option<u64>,
    pub supported: bool,
    pub unsupported_reason: Option<String>,
    /// A chosen or self-built executable works even when `supported` is
    /// false (the reason only means there is no prebuilt download).
    pub allow_own_binary: bool,
    pub requirements: Vec<RequirementStatus>,
    pub install: Vec<InstallOptionView>,
    pub docs_url: &'static str,
}

/// How to launch an engine: the program and the arguments that come before
/// the engine's own flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineCommand {
    pub program: PathBuf,
    pub leading_args: Vec<String>,
    /// Which recipe executable was found.
    pub binary: String,
}

/// How to launch `program` for a recipe: the unified `llama` binary runs as
/// `llama serve`; Kev runs as `uv tool run … python -m kev.serve`.
pub fn command_for(id: RecipeId, program: PathBuf) -> EngineCommand {
    let binary = program
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let leading_args = match id {
        RecipeId::LlamaCpp if binary == "llama" => vec!["serve".to_string()],
        RecipeId::Kev => kev_uv_run_args(&["-m", "kev.serve"]),
        RecipeId::Decider => {
            crate::recipes::decider_uv_run_args(Platform::current().is_apple_silicon())
        }
        _ => vec![],
    };
    EngineCommand {
        program,
        leading_args,
        binary,
    }
}

/// Locate a recipe's executable, trying its binaries in preference order.
pub fn resolve_with(
    id: RecipeId,
    platform: &Platform,
    find: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<EngineCommand> {
    recipe(id, platform)
        .binaries
        .iter()
        .find_map(|name| find(name))
        .map(|program| command_for(id, program))
}

/// Find an engine: the override path (if it is a file), then the managed
/// install, then `find` (PATH).
pub fn locate(
    id: RecipeId,
    platform: &Platform,
    override_path: Option<&Path>,
    managed: Option<&ManagedInstall>,
    find: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<(EngineCommand, EngineSource)> {
    if let Some(path) = override_path.filter(|p| p.is_file()) {
        return Some((command_for(id, path.to_path_buf()), EngineSource::Override));
    }
    if let Some(m) = managed.filter(|m| m.binary.is_file()) {
        return Some((command_for(id, m.binary.clone()), EngineSource::Managed));
    }
    resolve_with(id, platform, find).map(|c| (c, EngineSource::Path))
}

/// Locate an engine: explicit override path, LocalRouter's managed install,
/// then the user's PATH.
pub fn resolve(id: RecipeId, override_path: Option<&Path>) -> Option<EngineCommand> {
    let platform = Platform::current();
    let managed = crate::managed::installed(id);
    locate(id, &platform, override_path, managed.as_ref(), &|name| {
        lr_utils::binary::find_binary(name)
    })
    .map(|(command, _)| command)
}

/// Detect an engine. With `refresh`, the cached shell PATH is rebuilt first
/// (and on Windows re-read from the registry) so tools installed while the
/// app runs are found.
pub async fn detect(id: RecipeId, override_path: Option<PathBuf>, refresh: bool) -> EngineStatus {
    let managed = tokio::task::spawn_blocking(move || {
        if refresh {
            lr_utils::binary::refresh_shell_env();
        }
        crate::managed::installed(id)
    })
    .await
    .ok()
    .flatten();
    let platform = Platform::current();
    let find = |name: &str| lr_utils::binary::find_binary(name);
    let mut status = detect_with(
        id,
        &platform,
        override_path.as_deref(),
        managed.as_ref(),
        &find,
    );
    if status.source == Some(EngineSource::Managed) {
        // The release tag says more than the binary would.
        status.version = status.managed_tag.clone();
    } else if let Some(path) = status.path.clone() {
        if let Some((version, build)) = probe_version(id, Path::new(&path)).await {
            status.version = Some(version);
            status.build = build;
        }
    }
    status
}

/// Detection without running anything (version left empty); injectable
/// managed install and PATH lookup for tests.
pub fn detect_with(
    id: RecipeId,
    platform: &Platform,
    override_path: Option<&Path>,
    managed: Option<&ManagedInstall>,
    find: &dyn Fn(&str) -> Option<PathBuf>,
) -> EngineStatus {
    let recipe: EngineRecipe = recipe(id, platform);
    let located = locate(id, platform, override_path, managed, find);
    let source = located.as_ref().map(|(_, s)| *s);
    let command = located.map(|(c, _)| c);

    let requirements: Vec<RequirementStatus> = recipe
        .requires
        .iter()
        .map(|req| {
            let r = crate::recipes::recipe(*req, platform);
            let path = r.binaries.iter().find_map(|b| find(b));
            RequirementStatus {
                recipe: *req,
                display_name: r.display_name,
                found: path.is_some(),
                path: path.map(|p| p.display().to_string()),
            }
        })
        .collect();

    let mut install: Vec<InstallOptionView> = recipe
        .install
        .iter()
        .map(|o| InstallOptionView {
            runnable: o.runnable(),
            program_found: match o.kind {
                InstallKind::Download => true,
                InstallKind::Command => o.program.is_some_and(|p| find(p).is_some()),
            },
            recommended: false,
            option: o.clone(),
        })
        .collect();
    // Recommend the first option the user can run right now, else the first.
    let pick = install
        .iter()
        .position(|o| o.runnable && o.program_found)
        .or(if install.is_empty() { None } else { Some(0) });
    if let Some(i) = pick {
        install[i].recommended = true;
    }

    let supported = recipe.unsupported_reason.is_none();
    EngineStatus {
        recipe: id,
        display_name: recipe.display_name,
        found: (supported || recipe.allow_own_binary)
            && command.is_some()
            && requirements.iter().all(|r| r.found),
        path: command.as_ref().map(|c| c.program.display().to_string()),
        binary: command.map(|c| c.binary),
        source,
        managed_tag: managed.map(|m| m.tag.clone()),
        managed_build: managed.map(|m| m.build.clone()),
        version: None,
        build: None,
        supported,
        unsupported_reason: recipe.unsupported_reason.map(str::to_string),
        allow_own_binary: recipe.allow_own_binary,
        requirements,
        install,
        docs_url: recipe.docs_url,
    }
}

/// Run `<binary> --version` and parse the result.
async fn probe_version(id: RecipeId, path: &Path) -> Option<(String, Option<u64>)> {
    match id {
        RecipeId::LlamaCpp | RecipeId::Uv | RecipeId::Kev | RecipeId::Decider => {
            let output = run_capture(path, &["--version"]).await?;
            match id {
                RecipeId::LlamaCpp => parse_llama_version(&output),
                _ => parse_uv_version(&output).map(|v| (v, None)),
            }
        }
        // `sd-server --version` (checked as the first argument, before any
        // other parsing) prints the version and exits.
        RecipeId::SdCpp => parse_sd_version(&run_capture(path, &["--version"]).await?),
        // laya-serve has no version flag and starts the server when run;
        // `von --version` is hard-coded upstream, so it says nothing.
        RecipeId::Laya | RecipeId::Von => None,
    }
}

/// Parse `sd-server --version`:
/// `stable-diffusion.cpp version master-920-2f88688, commit 2f88688`.
/// Release builds report version `unknown`; the commit is shown instead.
pub fn parse_sd_version(text: &str) -> Option<(String, Option<u64>)> {
    let re = regex::Regex::new(r"stable-diffusion\.cpp version (\S+), commit (\w+)").ok()?;
    let caps = re.captures(text)?;
    let version = caps.get(1)?.as_str();
    let commit = caps.get(2)?.as_str();
    let shown = if version == "unknown" {
        format!("commit {commit}")
    } else {
        version.to_string()
    };
    Some((shown, None))
}

/// Run a program with the user's shell environment and return combined
/// stdout+stderr, or `None` on failure or timeout.
pub(crate) async fn run_capture(path: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = host_command(path, args.iter().map(|s| s.to_string()), Vec::new());
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let child = cmd.spawn().ok()?;
    let output = tokio::time::timeout(PROBE_TIMEOUT, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Some(text)
}

/// Parse `llama-server --version`: `version: 0.5.0 (build 11146, commit 7fe450e19)`
/// on current builds, `version: 6153 (abc1234)` on older ones.
pub fn parse_llama_version(text: &str) -> Option<(String, Option<u64>)> {
    let re = regex::Regex::new(r"version: (\S+) \((?:build (\d+), commit (\w+)|(\w+))\)").ok()?;
    let caps = re.captures(text)?;
    let version = caps.get(1)?.as_str().to_string();
    let build = caps
        .get(2)
        .and_then(|b| b.as_str().parse().ok())
        // Old format: the version *is* the build number.
        .or_else(|| version.parse().ok());
    Some((version, build))
}

/// Parse `uv --version`: `uv 0.12.18 (abc 2026-09-01)`.
pub fn parse_uv_version(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("uv "))
        .and_then(|rest| rest.split_whitespace().next())
        .map(str::to_string)
}

/// Flags an installed llama.cpp build understands (older distro builds lack
/// some of the newer ones).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct LlamaCaps {
    pub fit: bool,
    pub flash_attn_auto: bool,
    pub no_webui: bool,
    pub offline: bool,
    pub jinja: bool,
    pub gpu_layers_auto: bool,
    pub api_key_env: bool,
}

pub fn parse_llama_help(help: &str) -> LlamaCaps {
    LlamaCaps {
        fit: help.contains("--fit"),
        flash_attn_auto: help.contains("--flash-attn") && help.contains("auto"),
        no_webui: help.contains("--no-webui"),
        offline: help.contains("--offline"),
        jinja: help.contains("--jinja"),
        gpu_layers_auto: help.contains("--gpu-layers") || help.contains("'auto'"),
        api_key_env: help.contains("LLAMA_API_KEY"),
    }
}

static LLAMA_CAPS: Mutex<Option<HashMap<PathBuf, LlamaCaps>>> = Mutex::new(None);

/// Probe (once per executable) which flags llama.cpp supports.
pub async fn llama_capabilities(command: &EngineCommand) -> LlamaCaps {
    if let Some(caps) = LLAMA_CAPS
        .lock()
        .as_ref()
        .and_then(|m| m.get(&command.program).copied())
    {
        return caps;
    }
    let mut args: Vec<&str> = command.leading_args.iter().map(String::as_str).collect();
    args.push("--help");
    let caps = run_capture(&command.program, &args)
        .await
        .map(|help| parse_llama_help(&help))
        .unwrap_or_default();
    LLAMA_CAPS
        .lock()
        .get_or_insert_with(HashMap::new)
        .insert(command.program.clone(), caps);
    caps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{Arch, Os};

    fn mac() -> Platform {
        Platform {
            os: Os::MacOs,
            arch: Arch::Aarch64,
            linux: None,
        }
    }

    #[test]
    fn version_parsing() {
        assert_eq!(
            parse_llama_version("version: 0.5.0 (build 11146, commit 7fe450e19)\nbuilt with clang"),
            Some(("0.5.0".to_string(), Some(11146)))
        );
        assert_eq!(
            parse_llama_version("version: 6153 (abc1234)"),
            Some(("6153".to_string(), Some(6153)))
        );
        assert_eq!(parse_llama_version("garbage"), None);
        assert_eq!(
            parse_uv_version("uv 0.12.18 (e3f1 2026-09-01)"),
            Some("0.12.18".to_string())
        );
    }

    #[test]
    fn help_parsing() {
        let caps = parse_llama_help(
            "-fa, --flash-attn [on|off|auto]\n--fit [on|off]\n--no-webui\n--offline\n--jinja\n-ngl, --gpu-layers\nenv: LLAMA_API_KEY",
        );
        assert!(caps.fit && caps.flash_attn_auto && caps.no_webui && caps.offline && caps.jinja);
        assert!(caps.api_key_env);
        assert_eq!(parse_llama_help(""), LlamaCaps::default());
    }

    #[test]
    fn unified_llama_binary_runs_as_serve() {
        let find = |name: &str| (name == "llama").then(|| PathBuf::from("/x/llama"));
        let cmd = resolve_with(RecipeId::LlamaCpp, &mac(), &find).unwrap();
        assert_eq!(cmd.leading_args, vec!["serve"]);
        let find = |name: &str| {
            matches!(name, "llama" | "llama-server").then(|| PathBuf::from(format!("/x/{name}")))
        };
        let cmd = resolve_with(RecipeId::LlamaCpp, &mac(), &find).unwrap();
        assert_eq!(cmd.binary, "llama-server", "llama-server preferred");
        assert!(cmd.leading_args.is_empty());
    }

    #[test]
    fn kev_resolves_to_uv() {
        let find = |name: &str| (name == "uv").then(|| PathBuf::from("/x/uv"));
        let cmd = resolve_with(RecipeId::Kev, &mac(), &find).unwrap();
        assert!(cmd
            .leading_args
            .iter()
            .any(|a| a.contains("jaredpalmer/kev@")));
    }

    #[test]
    fn detection_reports_requirements_and_recommendation() {
        // Nothing installed except Homebrew.
        let find = |name: &str| (name == "brew").then(|| PathBuf::from("/opt/homebrew/bin/brew"));
        let s = detect_with(RecipeId::Laya, &mac(), None, None, &find);
        assert!(!s.found);
        assert!(!s.requirements[0].found, "uv missing");
        let s = detect_with(RecipeId::LlamaCpp, &mac(), None, None, &find);
        assert!(!s.found);
        let rec: Vec<_> = s.install.iter().filter(|o| o.recommended).collect();
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0].option.id, "brew");
        assert!(rec[0].program_found);

        // Laya found only when uv is present too.
        let find = |name: &str| {
            matches!(name, "laya-serve" | "uv").then(|| PathBuf::from(format!("/bin/{name}")))
        };
        assert!(detect_with(RecipeId::Laya, &mac(), None, None, &find).found);
    }

    #[test]
    fn unsupported_platform_is_never_found() {
        let intel = Platform {
            os: Os::MacOs,
            arch: Arch::X86_64,
            linux: None,
        };
        let find = |name: &str| Some(PathBuf::from(format!("/bin/{name}")));
        let s = detect_with(RecipeId::Kev, &intel, None, None, &find);
        assert!(!s.supported);
        assert!(!s.found);
    }

    fn managed_at(dir: &Path) -> ManagedInstall {
        let release = dir.join("master-920-2f88688-metal");
        std::fs::create_dir_all(&release).unwrap();
        std::fs::write(release.join("sd-server"), b"x").unwrap();
        ManagedInstall {
            tag: "master-920-2f88688".into(),
            build: "metal".into(),
            asset: "sd-master-2f88688-bin-Darwin-macOS-26.6.2-arm64.zip".into(),
            binary: release.join("sd-server"),
            dir: release,
            installed_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn sdcpp_detection_order_is_override_managed_path() {
        let tmp = tempfile::tempdir().unwrap();
        let managed = managed_at(tmp.path());
        let chosen = tmp.path().join("my-sd-server");
        std::fs::write(&chosen, b"x").unwrap();
        let on_path =
            |name: &str| (name == "sd-server").then(|| PathBuf::from("/usr/bin/sd-server"));
        let nothing = |_: &str| None;

        let s = detect_with(
            RecipeId::SdCpp,
            &mac(),
            Some(&chosen),
            Some(&managed),
            &on_path,
        );
        assert_eq!(s.source, Some(EngineSource::Override));
        assert_eq!(s.path.as_deref(), Some(chosen.to_str().unwrap()));
        assert_eq!(s.managed_tag.as_deref(), Some("master-920-2f88688"));
        assert!(s.found);

        // A chosen file that no longer exists falls through.
        let gone = tmp.path().join("gone");
        let s = detect_with(
            RecipeId::SdCpp,
            &mac(),
            Some(&gone),
            Some(&managed),
            &on_path,
        );
        assert_eq!(s.source, Some(EngineSource::Managed));
        assert_eq!(s.binary.as_deref(), Some("sd-server"));
        assert_eq!(s.managed_build.as_deref(), Some("metal"));

        let s = detect_with(RecipeId::SdCpp, &mac(), None, None, &on_path);
        assert_eq!(s.source, Some(EngineSource::Path));
        assert_eq!(s.path.as_deref(), Some("/usr/bin/sd-server"));
        assert!(s.managed_tag.is_none());

        let s = detect_with(RecipeId::SdCpp, &mac(), None, None, &nothing);
        assert!(!s.found);
        assert!(s.source.is_none());
        // Download options are always "available" and runnable.
        assert!(s.install.iter().all(|o| o.program_found && o.runnable));
        assert!(s.install[0].recommended);

        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["recipe"], "sdcpp");
        assert_eq!(json["install"][0]["kind"], "download");
        assert!(json["install"][0]["command"].is_null());
    }

    #[test]
    fn sdcpp_runs_a_chosen_binary_where_there_is_no_download() {
        let tmp = tempfile::tempdir().unwrap();
        let chosen = tmp.path().join("sd-server");
        std::fs::write(&chosen, b"x").unwrap();
        let linux_arm = Platform {
            os: Os::Linux,
            arch: Arch::Aarch64,
            linux: None,
        };
        let s = detect_with(RecipeId::SdCpp, &linux_arm, Some(&chosen), None, &|_| None);
        assert!(!s.supported);
        assert!(s.allow_own_binary);
        assert!(s.found);
        assert_eq!(s.source, Some(EngineSource::Override));
        assert!(s.install.is_empty());
    }

    #[test]
    fn sd_version_parsing() {
        assert_eq!(
            parse_sd_version("stable-diffusion.cpp version unknown, commit 2f88688\n"),
            Some(("commit 2f88688".to_string(), None))
        );
        assert_eq!(
            parse_sd_version("stable-diffusion.cpp version master-920-2f88688, commit 2f88688"),
            Some(("master-920-2f88688".to_string(), None))
        );
        assert_eq!(parse_sd_version("usage: sd-server [options]"), None);
    }
}
