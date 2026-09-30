//! Download options: fetch the latest GitHub release asset of an engine and
//! install it into LocalRouter's managed folder ([`crate::managed`]).
//!
//! This is the only place LocalRouter downloads engine code, and it runs only
//! when the user clicks Download.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::install::{InstallSink, OutputStream};
use crate::managed::{self, CurrentPointer};
use crate::platform::Os;
use crate::recipes::{AssetPattern, DownloadSpec};

/// GitHub's REST API.
pub const GITHUB_API: &str = "https://api.github.com";

const PROGRESS_INTERVAL: Duration = Duration::from_secs(2);

/// A download option to run.
#[derive(Debug, Clone)]
pub(crate) struct DownloadJob {
    /// Engine name for messages, e.g. "stable-diffusion.cpp".
    pub display_name: &'static str,
    /// Option label, e.g. "Vulkan".
    pub label: &'static str,
    pub spec: DownloadSpec,
    /// OS the binary is for (decides `.exe` and permission bits).
    pub os: Os,
    /// API base URL (tests point it at a mock server).
    pub api_base: String,
    /// The recipe's managed folder.
    pub dir: PathBuf,
}

#[derive(Debug)]
pub(crate) enum DownloadError {
    Cancelled,
    Failed(String),
}

impl<E: std::fmt::Display> From<E> for DownloadError {
    fn from(e: E) -> Self {
        DownloadError::Failed(e.to_string())
    }
}

fn fail(msg: impl Into<String>) -> DownloadError {
    DownloadError::Failed(msg.into())
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ReleaseAsset {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    pub browser_download_url: String,
    /// `sha256:<hex>` published by GitHub for the asset, when present.
    #[serde(default)]
    pub digest: Option<String>,
}

/// The first asset whose name matches `pattern`.
pub(crate) fn select_asset<'a>(
    assets: &'a [ReleaseAsset],
    pattern: &AssetPattern,
) -> Option<&'a ReleaseAsset> {
    assets.iter().find(|a| pattern.matches(&a.name))
}

/// A release tag usable as a folder name.
fn check_tag(tag: &str) -> Result<(), DownloadError> {
    let ok = !tag.is_empty()
        && !tag.starts_with('.')
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(fail(format!("unexpected release tag '{tag}'")))
    }
}

/// Removes a folder when dropped (the install's scratch space).
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Run a download install. Returns the installed release tag.
pub(crate) async fn run_download(
    run_id: &str,
    job: &DownloadJob,
    sink: &Arc<dyn InstallSink>,
    token: &CancellationToken,
) -> Result<String, DownloadError> {
    let say = |line: &str| sink.on_line(run_id, OutputStream::Stdout, line);
    let client = reqwest::Client::builder()
        .user_agent(concat!("LocalRouter/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(120))
        .build()?;

    let url = match job.spec.tag {
        Some(tag) => {
            say(&format!(
                "Looking up {} release {tag} on github.com/{}",
                job.display_name, job.spec.repo
            ));
            format!(
                "{}/repos/{}/releases/tags/{tag}",
                job.api_base.trim_end_matches('/'),
                job.spec.repo
            )
        }
        None => {
            say(&format!(
                "Looking up the latest {} release on github.com/{}",
                job.display_name, job.spec.repo
            ));
            format!(
                "{}/repos/{}/releases/latest",
                job.api_base.trim_end_matches('/'),
                job.spec.repo
            )
        }
    };
    let release: Release = cancellable(token, async {
        client
            .get(&url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await?
            .error_for_status()?
            .json::<Release>()
            .await
    })
    .await?
    .map_err(|e| fail(format!("could not read the latest release: {e}")))?;
    check_tag(&release.tag_name)?;
    let tag = release.tag_name.clone();

    let primary = select_asset(&release.assets, &job.spec.asset).ok_or_else(|| {
        fail(format!(
            "release {tag} has no {} build for this platform",
            job.label
        ))
    })?;
    let mut assets = vec![primary.clone()];
    for extra in job.spec.extras {
        let asset = select_asset(&release.assets, extra).ok_or_else(|| {
            fail(format!(
                "release {tag} is missing a file the {} build needs",
                job.label
            ))
        })?;
        assets.push(asset.clone());
    }
    if job.spec.tag.is_none() {
        say(&format!("Latest release: {tag}"));
    }

    std::fs::create_dir_all(&job.dir)?;
    let work = job.dir.join(format!(".tmp-{run_id}"));
    let _cleanup = RemoveOnDrop(work.clone());
    let downloads = work.join("downloads");
    let extract = work.join("release");
    std::fs::create_dir_all(&downloads)?;
    std::fs::create_dir_all(&extract)?;

    let mut files = Vec::new();
    for asset in &assets {
        let dest = downloads.join(&asset.name);
        download_file(&client, asset, &dest, &say, token).await?;
        files.push(dest);
    }

    // Extract the main archive, find the executable, then put the extras
    // (runtime DLLs) next to it, or at the root for releases whose extras
    // share the main archive's layout.
    say(&format!("Extracting {}", primary.name));
    let exe_name = match job.os {
        Os::Windows => format!("{}.exe", job.spec.binary),
        Os::MacOs | Os::Linux => job.spec.binary.to_string(),
    };
    let (binary, files_written) = {
        let token = token.clone();
        let extract = extract.clone();
        let files = files.clone();
        let os = job.os;
        let extras_at_root = job.spec.extras_at_root;
        tokio::task::spawn_blocking(move || -> Result<(PathBuf, usize), DownloadError> {
            let mut count = extract_archive(&files[0], &extract, os, &token)?;
            let binary = find_file(&extract, &exe_name)
                .ok_or_else(|| fail(format!("the release archive contains no {exe_name}")))?;
            let extras_dir = if extras_at_root {
                extract.clone()
            } else {
                binary.parent().unwrap_or(&extract).to_path_buf()
            };
            for extra in &files[1..] {
                count += extract_archive(extra, &extras_dir, os, &token)?;
            }
            #[cfg(unix)]
            if os != Os::Windows {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))?;
            }
            Ok((binary, count))
        })
        .await
        .map_err(|e| fail(format!("extraction failed: {e}")))??
    };
    tracing::debug!(files_written, "extracted engine release");
    for f in &files {
        let _ = std::fs::remove_file(f);
    }
    if token.is_cancelled() {
        return Err(DownloadError::Cancelled);
    }

    // Point of no return: move the release into place and point at it.
    let folder = format!("{tag}-{}", job.spec.build);
    let final_dir = job.dir.join(&folder);
    let rel_binary = binary
        .strip_prefix(&extract)
        .map_err(|_| fail("executable outside the release folder"))?
        .to_path_buf();
    let backup = job.dir.join(format!(".old-{run_id}"));
    let had_previous = final_dir.exists();
    if had_previous {
        std::fs::rename(&final_dir, &backup).map_err(|e| {
            fail(format!(
                "could not replace the existing {folder} (stop the engine first): {e}"
            ))
        })?;
    }
    let restore = |final_dir: &Path| {
        let _ = std::fs::remove_dir_all(final_dir);
        if had_previous {
            let _ = std::fs::rename(&backup, final_dir);
        }
    };
    if let Err(e) = std::fs::rename(&extract, &final_dir) {
        restore(&final_dir);
        return Err(fail(format!("could not move the release into place: {e}")));
    }
    let pointer = CurrentPointer {
        tag: tag.clone(),
        build: job.spec.build.to_string(),
        asset: primary.name.clone(),
        binary: Path::new(&folder)
            .join(&rel_binary)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/"),
        installed_at: chrono::Utc::now(),
    };
    if let Err(e) = managed::write_current(&job.dir, &pointer) {
        restore(&final_dir);
        return Err(fail(format!("could not record the install: {e}")));
    }
    drop(_cleanup);
    for (path, e) in managed::remove_other_installs(&job.dir, &folder) {
        say(&format!(
            "Could not remove the older install {} ({e}); it is removed after the next install.",
            path.display()
        ));
    }
    say(&format!(
        "Installed {} {tag} ({} build) in {}",
        job.display_name,
        job.label,
        final_dir.display()
    ));
    Ok(tag)
}

/// Run `fut` unless the install is cancelled first.
async fn cancellable<T>(
    token: &CancellationToken,
    fut: impl std::future::Future<Output = T>,
) -> Result<T, DownloadError> {
    tokio::select! {
        v = fut => Ok(v),
        _ = token.cancelled() => Err(DownloadError::Cancelled),
    }
}

fn megabytes(bytes: u64) -> u64 {
    (bytes + 500_000) / 1_000_000
}

/// Stream one asset to `dest`, reporting progress.
async fn download_file(
    client: &reqwest::Client,
    asset: &ReleaseAsset,
    dest: &Path,
    say: &(dyn Fn(&str) + Sync),
    token: &CancellationToken,
) -> Result<(), DownloadError> {
    let mut response = cancellable(token, client.get(&asset.browser_download_url).send())
        .await??
        .error_for_status()
        .map_err(|e| fail(format!("could not download {}: {e}", asset.name)))?;
    let total = response
        .content_length()
        .filter(|n| *n > 0)
        .or((asset.size > 0).then_some(asset.size));
    let mut file = tokio::fs::File::create(dest).await?;
    let mut hasher = {
        use sha2::Digest as _;
        sha2::Sha256::new()
    };
    let mut done: u64 = 0;
    let mut last_pct: Option<u64> = None;
    let mut last_report = Instant::now();
    let report = |done: u64| match total {
        Some(total) => format!(
            "Downloading {}: {}% ({}/{} MB)",
            asset.name,
            done * 100 / total.max(1),
            megabytes(done),
            megabytes(total)
        ),
        None => format!("Downloading {}: {} MB", asset.name, megabytes(done)),
    };
    say(&report(0));
    while let Some(chunk) = cancellable(token, response.chunk())
        .await?
        .map_err(|e| fail(format!("download of {} failed: {e}", asset.name)))?
    {
        file.write_all(&chunk).await?;
        sha2::Digest::update(&mut hasher, &chunk);
        done += chunk.len() as u64;
        let pct = total.map(|t| done * 100 / t.max(1));
        let step = matches!((pct, last_pct), (Some(p), Some(l)) if p / 5 > l / 5)
            || (pct.is_some() && last_pct.is_none());
        if step || last_report.elapsed() >= PROGRESS_INTERVAL {
            say(&report(done));
            last_pct = pct.or(last_pct);
            last_report = Instant::now();
        }
    }
    file.flush().await?;
    file.sync_all().await?;
    if let Some(total) = total {
        if done != total {
            return Err(fail(format!(
                "download of {} was incomplete ({done} of {total} bytes)",
                asset.name
            )));
        }
    }
    if last_pct != Some(100) {
        say(&report(done));
    }
    // GitHub publishes a SHA-256 for every release asset; check it.
    if let Some(expected) = asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
    {
        let actual = hex::encode(sha2::Digest::finalize(hasher));
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(fail(format!(
                "{} does not match its published checksum (expected sha256 {expected}, got {actual})",
                asset.name
            )));
        }
        say(&format!("Verified {} (sha256)", asset.name));
    }
    Ok(())
}

/// Extract a release archive (`.zip` or `.tar.zst`) into `dest`.
pub(crate) fn extract_archive(
    path: &Path,
    dest: &Path,
    os: Os,
    token: &CancellationToken,
) -> Result<usize, DownloadError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name.ends_with(".tar.zst") {
        extract_tar_zst(path, dest, os, token)
    } else {
        extract_zip(path, dest, os, token)
    }
}

/// Extract a zstd-compressed tar into `dest`, with the same rules as
/// [`extract_zip`]: paths and link targets must stay inside `dest`; hard
/// links and special files are refused.
pub(crate) fn extract_tar_zst(
    path: &Path,
    dest: &Path,
    os: Os,
    token: &CancellationToken,
) -> Result<usize, DownloadError> {
    let file = std::io::BufReader::new(std::fs::File::open(path)?);
    let decoder = ruzstd::decoding::StreamingDecoder::new(file)
        .map_err(|e| fail(format!("{} is not a valid .tar.zst: {e}", path.display())))?;
    let mut archive = tar::Archive::new(decoder);
    let mut written = 0;
    for entry in archive
        .entries()
        .map_err(|e| fail(format!("{} is not a valid tar: {e}", path.display())))?
    {
        if token.is_cancelled() {
            return Err(DownloadError::Cancelled);
        }
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().to_string();
        let kind = entry.header().entry_type();
        // `./` alone is the archive root.
        if kind.is_dir() && matches!(name.trim_end_matches('/'), "" | ".") {
            continue;
        }
        let rel = managed::safe_relative(&name)
            .ok_or_else(|| fail(format!("unsafe path in archive: {name}")))?;
        let out = dest.join(&rel);
        if kind.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if kind.is_symlink() {
            let target = entry
                .link_name()?
                .map(|t| t.to_string_lossy().to_string())
                .unwrap_or_default();
            if !symlink_stays_inside(&rel, &target) {
                return Err(fail(format!("unsafe link in archive: {name} -> {target}")));
            }
            #[cfg(unix)]
            {
                let _ = std::fs::remove_file(&out);
                std::os::unix::fs::symlink(&target, &out)?;
                written += 1;
            }
            continue;
        }
        if !kind.is_file() {
            return Err(fail(format!("unsupported entry in archive: {name}")));
        }
        let mut f = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut f)?;
        written += 1;
        #[cfg(unix)]
        if os != Os::Windows {
            use std::os::unix::fs::PermissionsExt;
            let executable = entry.header().mode().is_ok_and(|m| m & 0o111 != 0);
            let mode = if executable { 0o755 } else { 0o644 };
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode))?;
        }
    }
    let _ = os;
    Ok(written)
}

/// Extract a zip into `dest`. Entries whose paths would leave `dest` (zip
/// slip) fail the whole extraction. Returns the number of files written.
pub(crate) fn extract_zip(
    zip_path: &Path,
    dest: &Path,
    os: Os,
    token: &CancellationToken,
) -> Result<usize, DownloadError> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| fail(format!("{} is not a valid zip: {e}", zip_path.display())))?;
    let mut written = 0;
    for i in 0..archive.len() {
        if token.is_cancelled() {
            return Err(DownloadError::Cancelled);
        }
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        let rel = managed::safe_relative(&name)
            .ok_or_else(|| fail(format!("unsafe path in archive: {name}")))?;
        let out = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if entry.is_symlink() {
            let mut target = String::new();
            std::io::Read::read_to_string(&mut entry, &mut target)?;
            if !symlink_stays_inside(&rel, &target) {
                return Err(fail(format!("unsafe link in archive: {name} -> {target}")));
            }
            #[cfg(unix)]
            {
                let _ = std::fs::remove_file(&out);
                std::os::unix::fs::symlink(&target, &out)?;
                written += 1;
            }
            // Windows releases contain no links; nothing to do there.
            continue;
        }
        let mut f = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut f)?;
        written += 1;
        #[cfg(unix)]
        if os != Os::Windows {
            use std::os::unix::fs::PermissionsExt;
            let executable = entry.unix_mode().is_some_and(|m| m & 0o111 != 0);
            let mode = if executable { 0o755 } else { 0o644 };
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode))?;
        }
    }
    let _ = os;
    Ok(written)
}

/// A relative link target that, resolved from the link's folder, stays
/// inside the extraction folder.
fn symlink_stays_inside(link: &Path, target: &str) -> bool {
    if target.is_empty() || target.starts_with(['/', '\\']) || target.contains(':') {
        return false;
    }
    // Depth of the folder containing the link, relative to the root.
    let mut depth = link.components().count() as i64 - 1;
    for part in target.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => depth += 1,
        }
    }
    true
}

/// The shallowest regular file named `name` under `root` (links are not
/// followed).
pub(crate) fn find_file(root: &Path, name: &str) -> Option<PathBuf> {
    let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let mut entries: Vec<_> = std::fs::read_dir(&dir).ok()?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in &entries {
            let Ok(t) = entry.file_type() else { continue };
            if t.is_file() && entry.file_name() == name {
                return Some(entry.path());
            }
        }
        for entry in entries {
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                queue.push_back(entry.path());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipes::{SDCPP_LINUX_VULKAN, SDCPP_WINDOWS_CUDA12};
    use parking_lot::Mutex;
    use std::io::Write as _;
    use wiremock::matchers::{header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use zip::write::SimpleFileOptions;

    #[derive(Default)]
    struct Lines(Mutex<Vec<String>>);
    impl InstallSink for Lines {
        fn on_line(&self, _run_id: &str, _stream: OutputStream, line: &str) {
            self.0.lock().push(line.to_string());
        }
        fn on_finished(&self, _: &str, _: Option<i32>, _: bool, _: Option<String>) {}
    }

    /// Build a zip from `(name, contents, unix mode)`; `None` contents is a
    /// folder, a name starting with `@` is a link to the contents.
    fn make_zip(entries: &[(&str, Option<&[u8]>, u32)]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            for (name, contents, mode) in entries {
                let opts = SimpleFileOptions::default().unix_permissions(*mode);
                if let Some(link) = name.strip_prefix('@') {
                    let target = std::str::from_utf8(contents.unwrap()).unwrap();
                    w.add_symlink(link, target, opts).unwrap();
                } else if let Some(data) = contents {
                    w.start_file(*name, opts).unwrap();
                    w.write_all(data).unwrap();
                } else {
                    w.add_directory(*name, opts).unwrap();
                }
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    fn write_zip(dir: &Path, entries: &[(&str, Option<&[u8]>, u32)]) -> PathBuf {
        let p = dir.join(format!("{}.zip", uuid::Uuid::new_v4()));
        std::fs::write(&p, make_zip(entries)).unwrap();
        p
    }

    #[test]
    fn selects_the_first_matching_asset() {
        let assets: Vec<ReleaseAsset> = [
            "cudart-sd-bin-win-cu12-x64.zip",
            "sd-master-2f88688-bin-Linux-Ubuntu-24.04-x86_64-vulkan.zip",
            "sd-master-2f88688-bin-Linux-Ubuntu-24.04-x86_64.zip",
        ]
        .iter()
        .map(|n| ReleaseAsset {
            name: n.to_string(),
            size: 1,
            browser_download_url: format!("https://example.invalid/{n}"),
            digest: None,
        })
        .collect();
        assert_eq!(
            select_asset(&assets, &SDCPP_LINUX_VULKAN.asset)
                .unwrap()
                .name,
            "sd-master-2f88688-bin-Linux-Ubuntu-24.04-x86_64-vulkan.zip"
        );
        assert!(select_asset(&assets, &SDCPP_WINDOWS_CUDA12.asset).is_none());
        assert_eq!(
            select_asset(&assets, &SDCPP_WINDOWS_CUDA12.extras[0])
                .unwrap()
                .name,
            "cudart-sd-bin-win-cu12-x64.zip"
        );
    }

    #[test]
    fn tags_must_be_plain_folder_names() {
        assert!(check_tag("master-920-2f88688").is_ok());
        for bad in ["", "..", ".hidden", "a/b", "a\\b", "v1 2"] {
            assert!(check_tag(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn extraction_rejects_zip_slip() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("out");
        std::fs::create_dir_all(&dest).unwrap();
        let token = CancellationToken::new();
        for evil in ["../evil", "a/../../evil", "/abs/evil", "C:\\evil"] {
            let zip = write_zip(
                tmp.path(),
                &[("ok.txt", Some(b"x"), 0o644), (evil, Some(b"x"), 0o644)],
            );
            let err = extract_zip(&zip, &dest, Os::Linux, &token).unwrap_err();
            assert!(
                matches!(&err, DownloadError::Failed(m) if m.contains("unsafe path")),
                "{evil}: {err:?}"
            );
        }
        assert!(!tmp.path().join("evil").exists());
    }

    #[cfg(unix)]
    #[test]
    fn extraction_rejects_links_that_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("out");
        std::fs::create_dir_all(&dest).unwrap();
        let token = CancellationToken::new();
        let zip = write_zip(
            tmp.path(),
            &[("@lib/link", Some(b"../../etc/passwd"), 0o777)],
        );
        assert!(extract_zip(&zip, &dest, Os::Linux, &token).is_err());
        let zip = write_zip(tmp.path(), &[("@abs", Some(b"/etc/passwd"), 0o777)]);
        assert!(extract_zip(&zip, &dest, Os::Linux, &token).is_err());
        // A link to a sibling library is fine.
        let zip = write_zip(
            tmp.path(),
            &[
                ("lib/libx.so.0", Some(b"so"), 0o755),
                ("@lib/libx.so", Some(b"libx.so.0"), 0o777),
            ],
        );
        extract_zip(&zip, &dest, Os::Linux, &token).unwrap();
        assert_eq!(std::fs::read(dest.join("lib/libx.so")).unwrap(), b"so");
    }

    #[test]
    fn extracts_nested_files_and_finds_the_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("out");
        std::fs::create_dir_all(&dest).unwrap();
        let zip = write_zip(
            tmp.path(),
            &[
                ("build/", None, 0o755),
                ("build/bin/sd-server", Some(b"server"), 0o755),
                ("build/bin/libstable-diffusion.dylib", Some(b"lib"), 0o644),
                ("sd-server.txt", Some(b"license"), 0o644),
            ],
        );
        let n = extract_zip(&zip, &dest, Os::MacOs, &CancellationToken::new()).unwrap();
        assert_eq!(n, 3);
        let bin = find_file(&dest, "sd-server").unwrap();
        assert_eq!(bin, dest.join("build").join("bin").join("sd-server"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&bin), 0o755);
            assert_eq!(mode(&dest.join("sd-server.txt")), 0o644);
        }
        assert!(find_file(&dest, "sd-server.exe").is_none());
    }

    #[test]
    fn cancelled_extraction_stops() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = write_zip(tmp.path(), &[("a", Some(b"x"), 0o644)]);
        let token = CancellationToken::new();
        token.cancel();
        assert!(matches!(
            extract_zip(&zip, tmp.path(), Os::Linux, &token),
            Err(DownloadError::Cancelled)
        ));
    }

    struct Fixture {
        server: MockServer,
        tmp: tempfile::TempDir,
    }

    impl Fixture {
        async fn new() -> Self {
            Self {
                server: MockServer::start().await,
                tmp: tempfile::tempdir().unwrap(),
            }
        }

        fn dir(&self) -> PathBuf {
            self.tmp.path().join("sdcpp")
        }

        fn job(&self, spec: DownloadSpec, os: Os) -> DownloadJob {
            DownloadJob {
                display_name: "stable-diffusion.cpp",
                label: "Test",
                spec,
                os,
                api_base: self.server.uri(),
                dir: self.dir(),
            }
        }

        async fn release(&self, tag: &str, assets: &[(&str, Vec<u8>)]) {
            self.release_at(
                "/repos/leejet/stable-diffusion.cpp/releases/latest",
                tag,
                assets,
            )
            .await
        }

        async fn release_at(&self, api_path: &str, tag: &str, assets: &[(&str, Vec<u8>)]) {
            let list: Vec<_> = assets
                .iter()
                .map(|(name, bytes)| {
                    serde_json::json!({
                        "name": name,
                        "size": bytes.len(),
                        "browser_download_url": format!("{}/dl/{name}", self.server.uri()),
                        "digest": format!("sha256:{}", {
                            use sha2::Digest as _;
                            hex::encode(sha2::Sha256::digest(bytes))
                        }),
                    })
                })
                .collect();
            Mock::given(method("GET"))
                .and(path(api_path))
                .and(header_exists("user-agent"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"tag_name": tag, "assets": list})),
                )
                .mount(&self.server)
                .await;
            for (name, bytes) in assets {
                Mock::given(method("GET"))
                    .and(path(format!("/dl/{name}")))
                    .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
                    .mount(&self.server)
                    .await;
            }
        }

        fn entries(&self) -> Vec<String> {
            let mut v: Vec<_> = std::fs::read_dir(self.dir())
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
                .collect();
            v.sort();
            v
        }
    }

    const LINUX_VULKAN: &str = "sd-master-2f88688-bin-Linux-Ubuntu-24.04-x86_64-vulkan.zip";

    #[tokio::test]
    async fn a_download_that_does_not_match_its_published_checksum_fails() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/asset.zip"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"tampered".to_vec()))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let asset = ReleaseAsset {
            name: "asset.zip".into(),
            size: 8,
            browser_download_url: format!("{}/asset.zip", server.uri()),
            digest: Some(format!("sha256:{}", "0".repeat(64))),
        };
        let err = download_file(
            &reqwest::Client::new(),
            &asset,
            &dir.path().join("asset.zip"),
            &|_| {},
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(format!("{err:?}").contains("checksum"), "{err:?}");
    }

    #[tokio::test]
    async fn installs_the_release_and_points_current_at_it() {
        let fx = Fixture::new().await;
        let zip = make_zip(&[
            ("sd-server", Some(b"#!/bin/sh\n"), 0o755),
            ("libstable-diffusion.so", Some(b"lib"), 0o755),
        ]);
        fx.release("master-920-2f88688", &[(LINUX_VULKAN, zip)])
            .await;
        // An older release and a leftover from a crashed install.
        std::fs::create_dir_all(fx.dir().join("master-900-aaaaaaa-vulkan")).unwrap();
        std::fs::create_dir_all(fx.dir().join(".tmp-crashed")).unwrap();

        let lines = Arc::new(Lines::default());
        let sink: Arc<dyn InstallSink> = lines.clone();
        let tag = run_download(
            "run1",
            &fx.job(SDCPP_LINUX_VULKAN, Os::Linux),
            &sink,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(tag, "master-920-2f88688");

        let installed = managed::installed_in(&fx.dir()).unwrap();
        assert_eq!(installed.tag, "master-920-2f88688");
        assert_eq!(installed.build, "vulkan");
        assert_eq!(installed.asset, LINUX_VULKAN);
        assert_eq!(
            installed.binary,
            fx.dir().join("master-920-2f88688-vulkan").join("sd-server")
        );
        assert!(fx
            .dir()
            .join("master-920-2f88688-vulkan/libstable-diffusion.so")
            .is_file());
        assert_eq!(
            fx.entries(),
            vec![
                "current.json".to_string(),
                "master-920-2f88688-vulkan".to_string()
            ]
        );
        let lines = lines.0.lock().clone();
        assert!(lines.iter().any(|l| l.contains("100%")), "{lines:?}");
        assert!(lines
            .last()
            .unwrap()
            .contains("Installed stable-diffusion.cpp master-920-2f88688"));

        // Reinstalling the same release replaces it in place.
        run_download(
            "run2",
            &fx.job(SDCPP_LINUX_VULKAN, Os::Linux),
            &sink,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(managed::installed_in(&fx.dir()).is_some());
        assert_eq!(fx.entries().len(), 2);
    }

    #[tokio::test]
    async fn cuda_build_puts_the_runtime_next_to_the_exe() {
        let fx = Fixture::new().await;
        let main = make_zip(&[
            ("sd-server.exe", Some(b"exe"), 0o644),
            ("stable-diffusion.dll", Some(b"dll"), 0o644),
        ]);
        let cudart = make_zip(&[("cudart64_12.dll", Some(b"cuda"), 0o644)]);
        fx.release(
            "master-920-2f88688",
            &[
                ("sd-master-2f88688-bin-win-cuda12-x64.zip", main),
                ("cudart-sd-bin-win-cu12-x64.zip", cudart),
            ],
        )
        .await;
        let sink: Arc<dyn InstallSink> = Arc::new(Lines::default());
        run_download(
            "run",
            &fx.job(SDCPP_WINDOWS_CUDA12, Os::Windows),
            &sink,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let installed = managed::installed_in(&fx.dir()).unwrap();
        assert!(installed
            .binary
            .ends_with("master-920-2f88688-cuda12/sd-server.exe"));
        assert!(installed.dir.join("cudart64_12.dll").is_file());
    }

    /// Build a `.tar.zst` from `(name, contents, mode)`; `None` contents is a
    /// folder, a name starting with `@` is a link to the contents.
    fn make_tar_zst(entries: &[(&str, Option<&[u8]>, u32)]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (name, contents, mode) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_mode(*mode);
            if let Some(link) = name.strip_prefix('@') {
                h.set_entry_type(tar::EntryType::Symlink);
                h.set_size(0);
                let target = std::str::from_utf8(contents.unwrap()).unwrap();
                b.append_link(&mut h, link, target).unwrap();
            } else if let Some(data) = contents {
                h.set_entry_type(tar::EntryType::Regular);
                h.set_size(data.len() as u64);
                b.append_data(&mut h, name, *data).unwrap();
            } else {
                h.set_entry_type(tar::EntryType::Directory);
                h.set_size(0);
                b.append_data(&mut h, name, std::io::empty()).unwrap();
            }
        }
        let tar = b.into_inner().unwrap();
        ruzstd::encoding::compress_to_vec(&tar[..], ruzstd::encoding::CompressionLevel::Fastest)
    }

    #[test]
    fn tar_zst_extraction_keeps_modes_and_rejects_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("ok.tar.zst");
        std::fs::write(
            &archive,
            make_tar_zst(&[
                ("./", None, 0o755),
                ("bin/", None, 0o755),
                ("bin/ollaya", Some(b"exe"), 0o755),
                ("lib/ollaya/llama/libllama.0.dylib", Some(b"lib"), 0o644),
                ("@lib/ollaya/llama/libllama.dylib", Some(b"libllama.0.dylib"), 0o777),
            ]),
        )
        .unwrap();
        let out = dir.path().join("out");
        let n = extract_archive(&archive, &out, Os::Linux, &CancellationToken::new()).unwrap();
        assert_eq!(n, 3);
        assert_eq!(std::fs::read(out.join("bin/ollaya")).unwrap(), b"exe");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &str| std::fs::metadata(out.join(p)).unwrap().permissions().mode();
            assert_eq!(mode("bin/ollaya") & 0o777, 0o755);
            assert_eq!(mode("lib/ollaya/llama/libllama.0.dylib") & 0o777, 0o644);
            assert_eq!(
                std::fs::read(out.join("lib/ollaya/llama/libllama.dylib")).unwrap(),
                b"lib"
            );
        }

        for (name, entries) in [
            ("slip", vec![("../evil", Some(&b"x"[..]), 0o644)]),
            ("link", vec![("@bin/escape", Some(&b"../../etc/passwd"[..]), 0o777)]),
        ] {
            let bad = dir.path().join(format!("{name}.tar.zst"));
            // tar::Builder refuses `..` itself, so write the header by hand.
            let bytes = if name == "slip" {
                let mut h = tar::Header::new_gnu();
                h.as_gnu_mut().unwrap().name[..6].copy_from_slice(b"../evi");
                h.set_size(1);
                h.set_mode(0o644);
                h.set_entry_type(tar::EntryType::Regular);
                h.set_cksum();
                let mut raw = h.as_bytes().to_vec();
                raw.extend_from_slice(&[b'x'; 512]);
                raw.extend_from_slice(&[0; 1024]);
                ruzstd::encoding::compress_to_vec(
                    &raw[..],
                    ruzstd::encoding::CompressionLevel::Fastest,
                )
            } else {
                make_tar_zst(&entries)
            };
            std::fs::write(&bad, bytes).unwrap();
            let err = extract_archive(
                &bad,
                &dir.path().join(format!("out-{name}")),
                Os::Linux,
                &CancellationToken::new(),
            )
            .unwrap_err();
            assert!(format!("{err:?}").contains("unsafe"), "{name}: {err:?}");
        }
    }

    #[tokio::test]
    async fn pinned_release_with_extras_at_the_root() {
        use crate::recipes::{OLLAYA_MACOS, OLLAYA_VERSION};
        let fx = Fixture::new().await;
        let main = make_tar_zst(&[
            ("bin/ollaya", Some(b"exe"), 0o755),
            ("lib/ollaya/llama/libllama.0.dylib", Some(b"lib"), 0o644),
        ]);
        let mlx = make_tar_zst(&[("lib/ollaya/mlx_metal/mlx.metallib", Some(b"metal"), 0o644)]);
        fx.release_at(
            &format!("/repos/ollaya-dev/ollaya/releases/tags/{OLLAYA_VERSION}"),
            OLLAYA_VERSION,
            &[
                ("ollaya-darwin-arm64-mlx.tar.zst", mlx),
                ("ollaya-darwin-arm64.tar.zst", main),
            ],
        )
        .await;
        let sink: Arc<dyn InstallSink> = Arc::new(Lines::default());
        let tag = run_download(
            "run",
            &fx.job(OLLAYA_MACOS, Os::MacOs),
            &sink,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(tag, OLLAYA_VERSION);
        let installed = managed::installed_in(&fx.dir()).unwrap();
        let root = fx.dir().join(format!("{OLLAYA_VERSION}-metal"));
        assert_eq!(installed.binary, root.join("bin/ollaya"));
        // The MLX kernels land in lib/ next to bin/, not inside bin/.
        assert!(root.join("lib/ollaya/mlx_metal/mlx.metallib").is_file());
        assert!(root.join("lib/ollaya/llama/libllama.0.dylib").is_file());
    }

    #[tokio::test]
    async fn failures_leave_the_previous_install_untouched() {
        let fx = Fixture::new().await;
        // Existing install.
        let old = fx.dir().join("master-900-aaaaaaa-vulkan");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("sd-server"), b"old").unwrap();
        managed::write_current(
            &fx.dir(),
            &CurrentPointer {
                tag: "master-900-aaaaaaa".into(),
                build: "vulkan".into(),
                asset: "old.zip".into(),
                binary: "master-900-aaaaaaa-vulkan/sd-server".into(),
                installed_at: chrono::Utc::now(),
            },
        )
        .unwrap();

        // The archive has no sd-server.
        let zip = make_zip(&[("sd-cli", Some(b"cli"), 0o755)]);
        fx.release("master-920-2f88688", &[(LINUX_VULKAN, zip)])
            .await;
        let sink: Arc<dyn InstallSink> = Arc::new(Lines::default());
        let err = run_download(
            "run",
            &fx.job(SDCPP_LINUX_VULKAN, Os::Linux),
            &sink,
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, DownloadError::Failed(m) if m.contains("no sd-server")),
            "{err:?}"
        );
        assert_eq!(
            managed::installed_in(&fx.dir()).unwrap().tag,
            "master-900-aaaaaaa"
        );
        assert_eq!(
            fx.entries(),
            vec![
                "current.json".to_string(),
                "master-900-aaaaaaa-vulkan".to_string()
            ],
            "scratch folder removed"
        );
    }

    #[tokio::test]
    async fn missing_build_is_reported() {
        let fx = Fixture::new().await;
        fx.release(
            "master-920-2f88688",
            &[("sd-master-2f88688-bin-win-cpu-x64.zip", make_zip(&[]))],
        )
        .await;
        let sink: Arc<dyn InstallSink> = Arc::new(Lines::default());
        let err = run_download(
            "run",
            &fx.job(SDCPP_LINUX_VULKAN, Os::Linux),
            &sink,
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, DownloadError::Failed(m) if m.contains("no Test build")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn cancelling_during_the_download_cleans_up() {
        let fx = Fixture::new().await;
        let zip = make_zip(&[("sd-server", Some(b"x"), 0o755)]);
        // The asset responds too slowly to finish before the cancel.
        let list = serde_json::json!({
            "tag_name": "master-920-2f88688",
            "assets": [{
                "name": LINUX_VULKAN,
                "size": zip.len(),
                "browser_download_url": format!("{}/slow", fx.server.uri()),
            }],
        });
        Mock::given(method("GET"))
            .and(path("/repos/leejet/stable-diffusion.cpp/releases/latest"))
            .respond_with(ResponseTemplate::new(200).set_body_json(list))
            .mount(&fx.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/slow"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(zip)
                    .set_delay(Duration::from_secs(30)),
            )
            .mount(&fx.server)
            .await;

        let token = CancellationToken::new();
        let sink: Arc<dyn InstallSink> = Arc::new(Lines::default());
        let job = fx.job(SDCPP_LINUX_VULKAN, Os::Linux);
        let canceller = {
            let token = token.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                token.cancel();
            })
        };
        let started = Instant::now();
        let err = run_download("run", &job, &sink, &token).await.unwrap_err();
        canceller.await.unwrap();
        assert!(matches!(err, DownloadError::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(managed::installed_in(&fx.dir()).is_none());
        assert!(fx.entries().is_empty(), "{:?}", fx.entries());
    }
}
