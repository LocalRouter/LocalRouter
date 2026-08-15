//! Locating executables the way the user's terminal does.
//!
//! GUI processes do not inherit the PATH the user sees in a terminal:
//!
//! - a macOS `.app` launched from Finder/Dock starts with roughly
//!   `/usr/bin:/bin:/usr/sbin:/sbin`;
//! - a Linux `.desktop` launch inherits the session manager's PATH, which is
//!   set before the login shell sources `~/.zshrc` / `~/.profile`.
//!
//! Either way the directories where modern dev tooling actually installs —
//! `~/.local/bin`, `~/.opencode/bin`, fnm/nvm/volta node dirs, `~/.cargo/bin`,
//! `~/.bun/bin`, `/opt/homebrew/bin` — are missing, so a bare
//! [`which::which`] reports tools as "not installed" that run fine in the
//! user's terminal.
//!
//! [`find_binary`] resolves against the login-shell PATH (cached) and then a
//! small set of well-known user-local install directories.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// How long to wait for the login shell to print its PATH before giving up.
///
/// A misconfigured profile can block forever (e.g. one that prompts for
/// input); detection must not hang the app because of it.
#[cfg(unix)]
const SHELL_PATH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

static SHELL_ENV: OnceLock<HashMap<String, String>> = OnceLock::new();

/// Cached environment (currently just `PATH`) for locating and spawning
/// user-installed tools.
pub fn shell_env() -> HashMap<String, String> {
    SHELL_ENV.get_or_init(build_shell_env).clone()
}

/// The user's login-shell `PATH`, falling back to the process `PATH`.
pub fn shell_path() -> Option<String> {
    shell_env().get("PATH").cloned()
}

fn build_shell_env() -> HashMap<String, String> {
    let mut env = HashMap::new();

    let path = login_shell_path().or_else(|| std::env::var("PATH").ok());

    if let Some(path) = path {
        env.insert("PATH".to_string(), path);
    }

    env
}

/// Ask the user's login shell for its `PATH`.
///
/// Runs `$SHELL -lic 'echo $PATH'`: `-l` sources the login profile, `-i` the
/// interactive rc file. Both are needed because tool installers write their
/// PATH export to either one depending on the shell.
///
/// Returns `None` on Windows, where GUI processes already inherit the user's
/// full environment from the registry.
#[cfg(unix)]
fn login_shell_path() -> Option<String> {
    use std::process::{Command, Stdio};

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());

    // Discard stdin so an interactive profile that tries to read from the
    // terminal gets EOF instead of blocking, and drop stderr so shell noise
    // ("you have mail", instrumentation banners) never reaches the parser.
    let mut child = Command::new(&shell)
        .args(["-lic", "echo $PATH"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .inspect_err(|e| tracing::debug!("Could not run login shell {shell}: {e}"))
        .ok()?;

    let output = match wait_with_timeout(&mut child, SHELL_PATH_TIMEOUT) {
        Some(output) => output,
        None => {
            tracing::warn!("Login shell {shell} did not return a PATH in time; killing it");
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };

    if !output.status.success() {
        tracing::debug!("Login shell {shell} exited with {}", output.status);
        return None;
    }

    // An interactive shell may print banners before our echo, so take the
    // last non-empty line rather than the whole of stdout.
    let path = String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .to_string();

    // A PATH with no separator and no root-anchored entry is almost certainly
    // profile output we mistook for the answer.
    if !path.contains('/') {
        tracing::debug!("Ignoring implausible PATH from {shell}: {path}");
        return None;
    }

    tracing::info!("Resolved login-shell PATH from {shell}");
    Some(path)
}

#[cfg(not(unix))]
fn login_shell_path() -> Option<String> {
    None
}

/// Wait for `child`, giving up after `timeout` and returning its output.
///
/// `std::process::Child` has no timed wait, so poll `try_wait`. The polling
/// interval is short enough to stay responsive and long enough not to spin.
#[cfg(unix)]
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: std::time::Duration,
) -> Option<std::process::Output> {
    use std::io::Read;

    let deadline = std::time::Instant::now() + timeout;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                if let Some(mut pipe) = child.stdout.take() {
                    let _ = pipe.read_to_end(&mut stdout);
                }
                return Some(std::process::Output {
                    status,
                    stdout,
                    stderr: Vec::new(),
                });
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    }
}

/// Directories tools commonly install into that a GUI PATH usually misses.
///
/// Only used after both the process PATH and the login-shell PATH have failed,
/// so this list is a safety net for the case where the shell probe itself
/// could not run (locked-down `$SHELL`, container, sandbox).
fn fallback_bin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(home) = dirs::home_dir() {
        dirs.extend([
            home.join(".local/bin"),
            home.join("bin"),
            home.join(".opencode/bin"),
            home.join(".cargo/bin"),
            home.join(".bun/bin"),
            home.join(".deno/bin"),
            home.join(".volta/bin"),
            home.join(".npm-global/bin"),
            home.join(".yarn/bin"),
            home.join(".claude/local"),
        ]);
    }

    dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/snap/bin"),
    ]);

    dirs
}

/// Locate an executable by name the way the user's terminal would.
///
/// Tries, in order: the process `PATH`, the login-shell `PATH`, then
/// well-known user-local install directories. Returns the resolved absolute
/// path, or `None` if the tool genuinely is not installed.
pub fn find_binary(name: &str) -> Option<PathBuf> {
    if let Ok(path) = which::which(name) {
        return Some(path);
    }

    // Relative PATH entries resolve against cwd; absolute ones (the ones we
    // care about) do not. Root is a harmless stand-in when cwd is unavailable,
    // which can happen in sandboxed GUI contexts.
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

    if let Some(shell_path) = shell_path() {
        if let Ok(path) = which::which_in(name, Some(&shell_path), &cwd) {
            return Some(path);
        }
    }

    for dir in fallback_bin_dirs() {
        let candidate = dir.join(name);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }

    None
}

/// Whether `path` is something we can actually execute.
///
/// Symlinks are followed (`metadata`, not `symlink_metadata`) because these
/// install dirs are full of them — `~/.local/bin/agy` is typically a link into
/// a versioned directory.
fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };

    if !meta.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_env_always_yields_a_path() {
        // Either the login shell answered or we fell back to the process
        // PATH; a completely absent PATH would break every lookup.
        let env = shell_env();
        assert!(env.contains_key("PATH"), "shell_env() produced no PATH");
    }

    #[test]
    fn shell_path_is_cached_and_stable() {
        assert_eq!(shell_path(), shell_path());
    }

    #[cfg(unix)]
    #[test]
    fn resolved_path_contains_a_real_directory() {
        let path = shell_path().expect("a PATH should always resolve");
        assert!(
            path.split(':').any(|entry| Path::new(entry).is_dir()),
            "no entry in resolved PATH exists: {path}"
        );
    }

    #[test]
    fn finds_a_binary_that_exists() {
        // `sh` is present on every unix; `cmd` on every Windows.
        let name = if cfg!(unix) { "sh" } else { "cmd" };
        let found = find_binary(name);
        assert!(found.is_some(), "{name} should be locatable");
    }

    #[test]
    fn missing_binary_resolves_to_none() {
        assert!(find_binary("lr-definitely-not-a-real-binary-xyz").is_none());
    }

    #[test]
    fn directories_are_not_mistaken_for_executables() {
        // Guards the is_file() check: /usr/bin is executable-by-mode but is
        // not something we can run.
        assert!(!is_executable_file(Path::new("/usr")));
    }

    #[test]
    fn fallback_dirs_are_absolute() {
        for dir in fallback_bin_dirs() {
            assert!(dir.is_absolute(), "{dir:?} should be absolute");
        }
    }
}
