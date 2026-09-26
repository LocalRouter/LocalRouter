//! Image model bundles for stable-diffusion.cpp.
//!
//! Modern image models are sets of files: diffusion weights, a VAE and a text
//! encoder. The catalog lists curated bundles (all ungated on Hugging Face);
//! the store remembers which bundle files are on disk. A diffusion GGUF from
//! the model library whose header names an architecture a bundle serves
//! (for example a fine-tuned Qwen-Image 2.1) becomes a *derived* model that
//! uses the bundle's other files.
//!
//! Downloads go through [`crate::DownloadManager`] with a purpose of
//! [`purpose_for`]; the completion hook hands those jobs to
//! [`ImageModelStore::record_completed`] instead of the library.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::download::CompletedDownload;
use crate::library::{Library, LibraryEntry};
use crate::util;
use crate::ModelKind;

const STORE_FILE: &str = "image_models.json";
const STORE_VERSION: u32 = 1;
/// Purpose prefix of image-model download jobs.
const PURPOSE_PREFIX: &str = "image:";

/// What a bundle file is for; each maps to one `sd-server` flag.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ImageRole {
    Diffusion,
    Vae,
    /// LLM text encoder (Qwen-Image, Z-Image, FLUX.2).
    Llm,
    /// Vision projector of the LLM text encoder, needed to edit images
    /// with a GGUF text encoder (Qwen-Image 2.1).
    LlmVision,
    T5xxl,
    ClipL,
    ClipG,
}

impl ImageRole {
    /// The `sd-server` flag that takes this file.
    pub fn flag(self) -> &'static str {
        match self {
            ImageRole::Diffusion => "--diffusion-model",
            ImageRole::Vae => "--vae",
            ImageRole::Llm => "--llm",
            ImageRole::LlmVision => "--llm_vision",
            ImageRole::T5xxl => "--t5xxl",
            ImageRole::ClipL => "--clip_l",
            ImageRole::ClipG => "--clip_g",
        }
    }
}

/// One file of a bundle.
#[derive(Clone, Copy, Debug)]
pub struct BundleFile {
    pub role: ImageRole,
    pub repo: &'static str,
    pub path: &'static str,
    pub size_bytes: u64,
}

/// A curated image model: its files and the generation defaults it needs.
#[derive(Clone, Copy, Debug)]
pub struct ImageBundle {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub files: &'static [BundleFile],
    /// Extra `sd-server` arguments (generation defaults for the family).
    pub server_args: &'static [&'static str],
    /// GGUF `general.architecture` values of diffusion files that can
    /// replace this bundle's diffusion file.
    pub architectures: &'static [&'static str],
    /// Default output size (`WxH`).
    pub default_size: &'static str,
}

/// The curated bundles, smallest first.
pub const BUNDLES: &[ImageBundle] = &[
    ImageBundle {
        id: "flux2-klein-4b",
        name: "FLUX.2 Klein 4B",
        description: "Fast 4-step text-to-image from Black Forest Labs (Apache-2.0). Runs in about 6 GB of memory.",
        files: &[
            BundleFile {
                role: ImageRole::Diffusion,
                repo: "leejet/FLUX.2-klein-4B-GGUF",
                path: "flux-2-klein-4b-Q4_0.gguf",
                size_bytes: 2_460_000_000,
            },
            BundleFile {
                role: ImageRole::Vae,
                repo: "Comfy-Org/flux2-dev",
                path: "split_files/vae/flux2-vae.safetensors",
                size_bytes: 336_000_000,
            },
            BundleFile {
                role: ImageRole::Llm,
                repo: "unsloth/Qwen3-4B-GGUF",
                path: "Qwen3-4B-Q4_K_M.gguf",
                size_bytes: 2_500_000_000,
            },
        ],
        server_args: &["--cfg-scale", "1.0", "--steps", "4", "--sampling-method", "euler"],
        architectures: &[],
        default_size: "1024x1024",
    },
    ImageBundle {
        id: "z-image-turbo",
        name: "Z-Image Turbo",
        description: "Photorealistic 8-step text-to-image from Tongyi (Apache-2.0), good with text in images. Runs in about 7 GB of memory.",
        files: &[
            BundleFile {
                role: ImageRole::Diffusion,
                repo: "leejet/Z-Image-Turbo-GGUF",
                path: "z_image_turbo-Q4_K.gguf",
                size_bytes: 3_860_000_000,
            },
            BundleFile {
                role: ImageRole::Vae,
                repo: "Comfy-Org/z_image_turbo",
                path: "split_files/vae/ae.safetensors",
                size_bytes: 335_000_000,
            },
            BundleFile {
                role: ImageRole::Llm,
                repo: "unsloth/Qwen3-4B-Instruct-2507-GGUF",
                path: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
                size_bytes: 2_500_000_000,
            },
        ],
        server_args: &["--cfg-scale", "1.0", "--steps", "8"],
        architectures: &[],
        default_size: "1024x1024",
    },
    ImageBundle {
        id: "qwen-image-2.1",
        name: "Qwen-Image 2.1",
        description: "Qwen's image model with strong prompt following, text rendering and image editing. Needs about 12 GB of memory.",
        files: &[
            BundleFile {
                role: ImageRole::Diffusion,
                repo: "leejet/Qwen-Image-2.1-GGUF",
                path: "qwen_image_2.1-Q4_K.gguf",
                size_bytes: 4_200_000_000,
            },
            BundleFile {
                role: ImageRole::Vae,
                repo: "Comfy-Org/Qwen-Image-2.1",
                path: "vae/qwen_image_2.1_vae_bf16.safetensors",
                size_bytes: 680_000_000,
            },
            BundleFile {
                role: ImageRole::Llm,
                repo: "Qwen/Qwen3-VL-8B-Instruct-GGUF",
                path: "Qwen3VL-8B-Instruct-Q4_K_M.gguf",
                size_bytes: 5_030_000_000,
            },
            BundleFile {
                role: ImageRole::LlmVision,
                repo: "Qwen/Qwen3-VL-8B-Instruct-GGUF",
                path: "mmproj-Qwen3VL-8B-Instruct-F16.gguf",
                size_bytes: 1_160_000_000,
            },
        ],
        server_args: &["--cfg-scale", "6.0", "--sampling-method", "euler"],
        architectures: &["qwen_image21"],
        default_size: "1024x1024",
    },
];

/// The bundle with `id`.
pub fn bundle(id: &str) -> Option<&'static ImageBundle> {
    BUNDLES.iter().find(|b| b.id == id)
}

/// The download purpose tag for image model `model_id`.
pub fn purpose_for(model_id: &str) -> String {
    format!("{PURPOSE_PREFIX}{model_id}")
}

/// The image model id a download purpose names, if it is an image download.
pub fn model_for_purpose(purpose: &str) -> Option<&str> {
    purpose.strip_prefix(PURPOSE_PREFIX)
}

fn file_key(repo: &str, path: &str) -> String {
    format!("{repo}/{path}")
}

/// An image model as listed for the user.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ImageModel {
    /// Bundle id, or the library entry id for a derived model.
    pub id: String,
    pub name: String,
    pub description: String,
    /// The bundle this model uses (itself for curated bundles).
    pub bundle_id: String,
    /// Bytes still to download (0 when downloaded).
    pub missing_bytes: u64,
    /// Total size of the model's files.
    pub total_bytes: u64,
    pub downloaded: bool,
}

/// The files and arguments to launch `sd-server` for one model.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageModelLaunch {
    /// `(role, local path)` for every file.
    pub files: Vec<(ImageRole, PathBuf)>,
    pub server_args: Vec<String>,
    pub default_size: String,
}

impl ImageModelLaunch {
    /// Flag/value pairs for the files followed by the family's defaults.
    pub fn args(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (role, path) in &self.files {
            out.push(role.flag().to_string());
            out.push(path.display().to_string());
        }
        out.extend(self.server_args.iter().cloned());
        out
    }
}

/// Files to fetch for a model, grouped by repository.
pub type DownloadPlan = Vec<(String, Vec<String>)>;

#[derive(Serialize, Deserialize, Default)]
struct StoreFile {
    version: u32,
    /// `repo/path` → local path of every bundle file on disk.
    files: BTreeMap<String, PathBuf>,
}

/// Which bundle files are on disk, persisted to `{storage}/image_models.json`.
pub struct ImageModelStore {
    path: PathBuf,
    state: Mutex<StoreFile>,
}

impl ImageModelStore {
    /// Open (or start) the store in `storage_dir`. An unreadable file starts
    /// empty (the files stay on disk; downloading again finds them).
    pub fn open(storage_dir: &Path) -> Self {
        let path = storage_dir.join(STORE_FILE);
        let state = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<StoreFile>(&b).ok())
            .unwrap_or_default();
        Self {
            path,
            state: Mutex::new(state),
        }
    }

    fn persist(&self, state: &StoreFile) {
        let file = StoreFile {
            version: STORE_VERSION,
            files: state.files.clone(),
        };
        match serde_json::to_vec_pretty(&file) {
            Ok(bytes) => {
                if let Err(e) = util::write_atomic(&self.path, &bytes) {
                    tracing::warn!("could not save the image model list: {e}");
                }
            }
            Err(e) => tracing::warn!("could not serialize the image model list: {e}"),
        }
    }

    /// Local path of a bundle file, if it is on disk.
    fn local(&self, file: &BundleFile) -> Option<PathBuf> {
        self.state
            .lock()
            .files
            .get(&file_key(file.repo, file.path))
            .filter(|p| p.is_file())
            .cloned()
    }

    /// Library entries that can stand in for `bundle`'s diffusion file.
    fn derived_entries<'a>(
        bundle: &ImageBundle,
        entries: &'a [LibraryEntry],
    ) -> impl Iterator<Item = &'a LibraryEntry> {
        let archs = bundle.architectures;
        entries.iter().filter(move |e| {
            e.kind == ModelKind::Unsupported
                && e.architecture
                    .as_deref()
                    .is_some_and(|a| archs.contains(&a))
        })
    }

    /// Every image model: the curated bundles, then models derived from
    /// library diffusion files.
    pub fn catalog(&self, library: &Library) -> Vec<ImageModel> {
        let entries = library.list();
        let mut out = Vec::new();
        for b in BUNDLES {
            let missing: u64 = b
                .files
                .iter()
                .filter(|f| self.local(f).is_none())
                .map(|f| f.size_bytes)
                .sum();
            out.push(ImageModel {
                id: b.id.to_string(),
                name: b.name.to_string(),
                description: b.description.to_string(),
                bundle_id: b.id.to_string(),
                missing_bytes: missing,
                total_bytes: b.files.iter().map(|f| f.size_bytes).sum(),
                downloaded: missing == 0,
            });
        }
        for b in BUNDLES {
            for e in Self::derived_entries(b, &entries) {
                if bundle(&e.id).is_some() {
                    continue;
                }
                let support = b.files.iter().filter(|f| f.role != ImageRole::Diffusion);
                let missing: u64 = support
                    .clone()
                    .filter(|f| self.local(f).is_none())
                    .map(|f| f.size_bytes)
                    .sum();
                out.push(ImageModel {
                    id: e.id.clone(),
                    name: format!("{} ({} pipeline)", e.display_name, b.name),
                    description: format!(
                        "Your library file {} with {}'s VAE and text encoder.",
                        e.display_name, b.name
                    ),
                    bundle_id: b.id.to_string(),
                    missing_bytes: missing,
                    total_bytes: e.size_bytes + support.map(|f| f.size_bytes).sum::<u64>(),
                    downloaded: missing == 0 && e.model_path.is_file(),
                });
            }
        }
        out
    }

    /// The bundle and (for a derived model) the library entry behind `id`.
    fn resolve(
        &self,
        id: &str,
        library: &Library,
    ) -> Option<(&'static ImageBundle, Option<LibraryEntry>)> {
        if let Some(b) = bundle(id) {
            return Some((b, None));
        }
        let entry = library.get(id)?;
        let b = BUNDLES.iter().find(|b| {
            Self::derived_entries(b, std::slice::from_ref(&entry))
                .next()
                .is_some()
        })?;
        Some((b, Some(entry)))
    }

    /// Files still missing for `id`, grouped by repository (empty when the
    /// model is complete). `None` for unknown ids.
    pub fn download_plan(&self, id: &str, library: &Library) -> Option<DownloadPlan> {
        let (b, derived) = self.resolve(id, library)?;
        let mut plan: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for f in b.files {
            if derived.is_some() && f.role == ImageRole::Diffusion {
                continue;
            }
            if self.local(f).is_none() {
                plan.entry(f.repo.to_string())
                    .or_default()
                    .push(f.path.to_string());
            }
        }
        Some(plan.into_iter().collect())
    }

    /// Launch files and arguments for a downloaded model.
    pub fn launch(&self, id: &str, library: &Library) -> Option<ImageModelLaunch> {
        let (b, derived) = self.resolve(id, library)?;
        let mut files = Vec::new();
        for f in b.files {
            let path = match (&derived, f.role) {
                (Some(entry), ImageRole::Diffusion) => {
                    Some(entry.model_path.clone()).filter(|p| p.is_file())?
                }
                _ => self.local(f)?,
            };
            files.push((f.role, path));
        }
        Some(ImageModelLaunch {
            files,
            server_args: b.server_args.iter().map(|s| s.to_string()).collect(),
            default_size: b.default_size.to_string(),
        })
    }

    /// Record a finished image download. Returns false (and records nothing)
    /// when the download was not for an image model.
    pub fn record_completed(&self, done: &CompletedDownload) -> bool {
        if done
            .purpose
            .as_deref()
            .and_then(model_for_purpose)
            .is_none()
        {
            return false;
        }
        let mut state = self.state.lock();
        for (repo_path, local, _, _) in &done.files {
            state
                .files
                .insert(file_key(&done.repo, repo_path), local.clone());
        }
        self.persist(&state);
        true
    }

    /// Forget a model's files (optionally deleting them). Files another
    /// downloaded model still uses are kept; library files are never touched.
    pub fn remove(&self, id: &str, library: &Library, delete_files: bool) -> bool {
        let Some((b, derived)) = self.resolve(id, library) else {
            return false;
        };
        let own: Vec<&BundleFile> = b
            .files
            .iter()
            .filter(|f| !(derived.is_some() && f.role == ImageRole::Diffusion))
            .collect();
        // Files other complete models still need.
        let catalog = self.catalog(library);
        let mut keep = std::collections::HashSet::new();
        for m in catalog.iter().filter(|m| m.downloaded && m.id != id) {
            if let Some((ob, oderived)) = self.resolve(&m.id, library) {
                for f in ob.files {
                    if !(oderived.is_some() && f.role == ImageRole::Diffusion) {
                        keep.insert(file_key(f.repo, f.path));
                    }
                }
            }
        }
        let mut state = self.state.lock();
        for f in own {
            let key = file_key(f.repo, f.path);
            if keep.contains(&key) {
                continue;
            }
            if let Some(path) = state.files.remove(&key) {
                if delete_files {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        self.persist(&state);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gguf::test_support::GgufBuilder;

    fn done(repo: &str, files: &[(&str, &Path)], purpose: Option<&str>) -> CompletedDownload {
        CompletedDownload {
            repo: repo.into(),
            revision: "c0ffee".into(),
            files: files
                .iter()
                .map(|(p, local)| (p.to_string(), local.to_path_buf(), 1, None))
                .collect(),
            purpose: purpose.map(str::to_string),
        }
    }

    /// Write every file of `b` (except skipped roles) and record it.
    fn install(store: &ImageModelStore, dir: &Path, b: &ImageBundle, skip: &[ImageRole]) {
        for f in b.files.iter().filter(|f| !skip.contains(&f.role)) {
            let local = dir.join(f.repo).join(f.path);
            std::fs::create_dir_all(local.parent().unwrap()).unwrap();
            std::fs::write(&local, b"x").unwrap();
            assert!(store.record_completed(&done(
                f.repo,
                &[(f.path, &local)],
                Some(&purpose_for(b.id))
            )));
        }
    }

    #[test]
    fn catalog_is_consistent() {
        let mut ids = std::collections::HashSet::new();
        for b in BUNDLES {
            assert!(ids.insert(b.id), "duplicate bundle id {}", b.id);
            let roles: Vec<ImageRole> = b.files.iter().map(|f| f.role).collect();
            assert!(roles.contains(&ImageRole::Diffusion), "{}", b.id);
            assert!(roles.contains(&ImageRole::Vae), "{}", b.id);
            for f in b.files {
                assert!(!f.repo.is_empty() && f.repo.contains('/'));
                assert!(!f.path.starts_with('/') && !f.path.contains(".."));
                assert!(f.size_bytes > 0);
            }
            assert_eq!(b.server_args.len() % 2, 0, "{} flag/value pairs", b.id);
        }
    }

    #[test]
    fn purpose_round_trip() {
        assert_eq!(
            model_for_purpose(&purpose_for("z-image-turbo")),
            Some("z-image-turbo")
        );
        assert_eq!(model_for_purpose("library"), None);
    }

    #[test]
    fn downloads_complete_a_bundle_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().join("models"));
        let store = ImageModelStore::open(dir.path());
        let b = bundle("z-image-turbo").unwrap();

        let plan = store.download_plan(b.id, &library).unwrap();
        assert_eq!(plan.len(), 3, "one job per repository");
        assert!(store.launch(b.id, &library).is_none());

        install(&store, dir.path(), b, &[]);
        assert_eq!(store.download_plan(b.id, &library).unwrap(), vec![]);
        let m = store
            .catalog(&library)
            .into_iter()
            .find(|m| m.id == b.id)
            .unwrap();
        assert!(m.downloaded && m.missing_bytes == 0);

        let launch = store.launch(b.id, &library).unwrap();
        let args = launch.args();
        for flag in ["--diffusion-model", "--vae", "--llm", "--steps"] {
            assert!(args.contains(&flag.to_string()), "{flag} in {args:?}");
        }

        // Reopened from disk.
        let again = ImageModelStore::open(dir.path());
        assert!(again.launch(b.id, &library).is_some());

        // A deleted file makes the model incomplete again.
        std::fs::remove_file(dir.path().join(b.files[1].repo).join(b.files[1].path)).unwrap();
        assert!(again.launch(b.id, &library).is_none());
        assert_eq!(again.download_plan(b.id, &library).unwrap().len(), 1);
    }

    #[test]
    fn library_downloads_are_not_taken() {
        let dir = tempfile::tempdir().unwrap();
        let store = ImageModelStore::open(dir.path());
        let local = dir.path().join("m.gguf");
        std::fs::write(&local, b"x").unwrap();
        assert!(!store.record_completed(&done("org/m", &[("m.gguf", &local)], None)));
        assert!(!store.path.exists());
    }

    #[test]
    fn library_diffusion_file_makes_a_derived_model() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().join("models"));
        let gguf = dir.path().join("uncensored.gguf");
        std::fs::write(
            &gguf,
            GgufBuilder::new()
                .str("general.architecture", "qwen_image21")
                .u32("general.file_type", 2)
                .tensor("img_in.weight")
                .build_file(400),
        )
        .unwrap();
        let entry = library.import_file(&gguf).unwrap();
        assert_eq!(entry.kind, ModelKind::Unsupported);

        let store = ImageModelStore::open(dir.path());
        let derived = store
            .catalog(&library)
            .into_iter()
            .find(|m| m.id == entry.id)
            .expect("derived model listed");
        assert_eq!(derived.bundle_id, "qwen-image-2.1");
        assert!(!derived.downloaded);
        // Only the support files are fetched.
        let plan = store.download_plan(&entry.id, &library).unwrap();
        let files: Vec<&String> = plan.iter().flat_map(|(_, f)| f).collect();
        assert_eq!(files.len(), 3, "VAE, text encoder and its vision projector");
        assert!(files
            .iter()
            .all(|f| !f.ends_with("qwen_image_2.1-Q4_K.gguf")));

        let b = bundle("qwen-image-2.1").unwrap();
        install(&store, dir.path(), b, &[ImageRole::Diffusion]);
        let launch = store.launch(&entry.id, &library).unwrap();
        assert_eq!(
            launch.files[0],
            (ImageRole::Diffusion, entry.model_path.clone())
        );
        // The curated bundle itself is still incomplete (its own diffusion
        // file was never downloaded).
        assert!(store.launch(b.id, &library).is_none());

        // Removing the derived model keeps the library file.
        assert!(store.remove(&entry.id, &library, true));
        assert!(gguf.is_file());
        assert!(store.launch(&entry.id, &library).is_none());
    }

    #[test]
    fn removing_keeps_files_shared_with_another_complete_model() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().join("models"));
        let gguf = dir.path().join("variant.gguf");
        std::fs::write(
            &gguf,
            GgufBuilder::new()
                .str("general.architecture", "qwen_image21")
                .tensor("img_in.weight")
                .build_file(400),
        )
        .unwrap();
        let entry = library.import_file(&gguf).unwrap();
        let store = ImageModelStore::open(dir.path());
        let b = bundle("qwen-image-2.1").unwrap();
        install(&store, dir.path(), b, &[]);
        assert!(store.launch(&entry.id, &library).is_some());
        assert!(store.remove(b.id, &library, true));
        // The VAE and text encoder stay for the derived model.
        assert!(store.launch(&entry.id, &library).is_some());
        assert!(store.launch(b.id, &library).is_none());
    }
}
