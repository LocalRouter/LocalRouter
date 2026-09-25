//! "Will it fit?" memory estimate for running a GGUF model locally.
//!
//! The estimate follows llama.cpp's memory layout closely enough for a
//! traffic-light verdict:
//!
//! * **weights** ≈ the file size (sum of split parts, plus the projector when
//!   the caller includes it);
//! * **KV cache** = Σ over layers `n_ctx_layer × n_head_kv × (k_len × bpe +
//!   v_len × bpe)`, where `k_len`/`v_len` default to `n_embd / n_head`;
//! * **overhead** (compute buffers) ≈ KV for 256 tokens + 5 % of the weights;
//! * **budget** = the GPU budget (unified memory on Apple Silicon) or the
//!   currently available RAM when the GPU is unknown.

use serde::{Deserialize, Serialize};

use crate::gguf::GgufSummary;
use crate::hardware::HardwareInfo;

/// KV cache element type.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum KvCacheType {
    F16,
    Q8_0,
    Q4_0,
}

impl KvCacheType {
    /// Bytes per cached element.
    pub fn bytes_per_element(self) -> f64 {
        match self {
            KvCacheType::F16 => 2.0,
            KvCacheType::Q8_0 => 1.0625,
            KvCacheType::Q4_0 => 0.5625,
        }
    }
}

/// Traffic-light verdict.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum FitVerdict {
    /// Uses at most 85 % of the budget.
    Fits,
    /// Uses 85–100 % of the budget.
    Tight,
    /// Exceeds the budget.
    TooLarge,
    /// Not enough information.
    Unknown,
}

/// Memory estimate for one configuration.
#[derive(Serialize, Clone, Debug)]
pub struct FitEstimate {
    pub weights_bytes: u64,
    pub kv_bytes: u64,
    pub overhead_bytes: u64,
    pub total_bytes: u64,
    pub budget_bytes: Option<u64>,
    pub verdict: FitVerdict,
    /// The (per-slot) context length the estimate was made for.
    pub context_length: u64,
}

/// Fraction of the budget below which a model "fits".
const FITS_RATIO: f64 = 0.85;
/// Micro-batch added to the sliding-window cache size (llama.cpp `n_ubatch`).
const SWA_UBATCH: u64 = 512;
/// Tokens used for the compute-buffer approximation.
const OVERHEAD_TOKENS: u64 = 256;
/// Context lengths offered by [`max_context_that_fits`].
const CONTEXT_STEPS: [u64; 6] = [4096, 8192, 16384, 32768, 65536, 131072];
/// Context assumed when the caller passes 0 and the model does not say.
const DEFAULT_CONTEXT: u64 = 4096;

/// How many of `layers` use sliding-window attention.
///
/// GGUF headers do not carry the per-layer pattern for every architecture, so
/// this is approximate: Gemma 2 alternates (every 2nd layer is global), the
/// other Gemma models use Gemma 3's pattern of one global layer in six. For
/// every other architecture all layers are assumed to be global (an
/// over-estimate, never an under-estimate).
fn swa_layer_count(arch: Option<&str>, layers: u64) -> u64 {
    match arch {
        Some("gemma2") => layers - layers / 2,
        Some(a) if a.starts_with("gemma") => layers - layers / 6,
        _ => 0,
    }
}

/// KV cache size in bytes for `ctx` tokens per slot, or `None` when the
/// header lacks the attention geometry.
pub fn kv_cache_bytes(
    summary: &GgufSummary,
    ctx: u64,
    kv: KvCacheType,
    parallel_slots: u32,
) -> Option<u64> {
    let layers = summary.block_count.filter(|&l| l > 0)?;
    let head_count = summary.head_count.filter(|&h| h > 0);
    let n_head_kv = summary.head_count_kv.or(head_count)?;
    let default_dim = match (summary.embedding_length, head_count) {
        (Some(e), Some(h)) => Some(e / h),
        _ => None,
    };
    let k_len = summary.key_length.or(default_dim)?;
    let v_len = summary.value_length.or(default_dim).unwrap_or(k_len);
    let bpe = kv.bytes_per_element();
    let per_token_layer = n_head_kv as f64 * (k_len as f64 * bpe + v_len as f64 * bpe);

    let slots = u64::from(parallel_slots.max(1));
    let n_ctx = ctx.saturating_mul(slots);
    let (swa_layers, swa_ctx) = match summary.sliding_window.filter(|&w| w > 0) {
        Some(w) => (
            swa_layer_count(summary.architecture.as_deref(), layers),
            n_ctx.min(w.saturating_mul(slots).saturating_add(SWA_UBATCH)),
        ),
        None => (0, n_ctx),
    };
    let full_layers = layers - swa_layers;
    let cells = full_layers as f64 * n_ctx as f64 + swa_layers as f64 * swa_ctx as f64;
    Some((per_token_layer * cells) as u64)
}

/// Estimate memory use of running a model.
///
/// * `file_size_bytes`: total size of the GGUF (all split parts, plus the
///   projector if it will be loaded).
/// * `summary`: header facts. When `None`, the KV cache is estimated as 0 and
///   the verdict is based on weights + overhead only (an under-estimate; the
///   UI should label it as such). The verdict is `Unknown` when there is no
///   summary and the size is 0.
/// * `context_length`: tokens per slot (0 = the trained context, or 4096).
/// * `parallel_slots`: concurrent sequences; the KV cache holds
///   `context_length × parallel_slots` tokens.
pub fn estimate(
    file_size_bytes: u64,
    summary: Option<&GgufSummary>,
    context_length: u64,
    kv: KvCacheType,
    parallel_slots: u32,
    hw: &HardwareInfo,
) -> FitEstimate {
    let context_length = if context_length == 0 {
        summary
            .and_then(|s| s.context_length)
            .filter(|&c| c > 0)
            .unwrap_or(DEFAULT_CONTEXT)
    } else {
        context_length
    };
    let kv_bytes = summary
        .and_then(|s| kv_cache_bytes(s, context_length, kv, parallel_slots))
        .unwrap_or(0);
    let overhead_kv = summary
        .and_then(|s| kv_cache_bytes(s, OVERHEAD_TOKENS, kv, 1))
        .unwrap_or(0);
    let overhead_bytes = overhead_kv + file_size_bytes / 20;
    let total_bytes = file_size_bytes + kv_bytes + overhead_bytes;
    let budget_bytes = hw
        .gpu_budget_bytes
        .or(Some(hw.available_ram_bytes))
        .filter(|&b| b > 0);

    let verdict = match budget_bytes {
        _ if summary.is_none() && file_size_bytes == 0 => FitVerdict::Unknown,
        None => FitVerdict::Unknown,
        Some(budget) => {
            let ratio = total_bytes as f64 / budget as f64;
            if ratio <= FITS_RATIO {
                FitVerdict::Fits
            } else if ratio <= 1.0 {
                FitVerdict::Tight
            } else {
                FitVerdict::TooLarge
            }
        }
    };

    FitEstimate {
        weights_bytes: file_size_bytes,
        kv_bytes,
        overhead_bytes,
        total_bytes,
        budget_bytes,
        verdict,
        context_length,
    }
}

/// The largest context from {4K, 8K, 16K, 32K, 64K, 128K} (capped at the
/// trained context) whose estimate is not `TooLarge` (`Tight` counts as
/// fitting). A model trained on fewer than 4096 tokens is offered its trained
/// context. Returns `None` without a summary with attention geometry (the KV
/// cache cannot be estimated) or when nothing fits.
pub fn max_context_that_fits(
    file_size_bytes: u64,
    summary: Option<&GgufSummary>,
    kv: KvCacheType,
    parallel_slots: u32,
    hw: &HardwareInfo,
) -> Option<u64> {
    let s = summary?;
    kv_cache_bytes(s, 1, kv, parallel_slots)?;
    let trained = s.context_length.filter(|&c| c > 0);
    let mut candidates: Vec<u64> = CONTEXT_STEPS
        .iter()
        .copied()
        .filter(|&c| trained.is_none_or(|t| c <= t))
        .collect();
    if candidates.is_empty() {
        candidates.push(trained?);
    }
    candidates.into_iter().rev().find(|&ctx| {
        matches!(
            estimate(file_size_bytes, summary, ctx, kv, parallel_slots, hw).verdict,
            FitVerdict::Fits | FitVerdict::Tight
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn qwen3_8b() -> GgufSummary {
        GgufSummary {
            architecture: Some("qwen3".into()),
            context_length: Some(40960),
            embedding_length: Some(4096),
            block_count: Some(36),
            head_count: Some(32),
            head_count_kv: Some(8),
            key_length: Some(128),
            value_length: Some(128),
            quant: Some("Q4_K_M".into()),
            ..Default::default()
        }
    }

    fn gemma3_4b() -> GgufSummary {
        GgufSummary {
            architecture: Some("gemma3".into()),
            context_length: Some(131072),
            embedding_length: Some(2560),
            block_count: Some(34),
            head_count: Some(8),
            head_count_kv: Some(4),
            key_length: Some(256),
            value_length: Some(256),
            sliding_window: Some(1024),
            ..Default::default()
        }
    }

    fn hw(budget: Option<u64>, available: u64) -> HardwareInfo {
        HardwareInfo {
            os: "test".into(),
            arch: "test".into(),
            total_ram_bytes: available * 2,
            available_ram_bytes: available,
            cpu_cores: 8,
            unified_memory: budget.is_some(),
            gpu_budget_bytes: budget,
        }
    }

    fn within(actual: f64, expected: f64, pct: f64) -> bool {
        ((actual - expected) / expected).abs() <= pct / 100.0
    }

    #[test]
    fn qwen3_8b_golden() {
        let weights = 5_030_000_000u64;
        let s = qwen3_8b();
        let e = estimate(
            weights,
            Some(&s),
            40960,
            KvCacheType::F16,
            1,
            &hw(Some(64 * GIB), 0),
        );
        assert!(
            within(e.kv_bytes as f64, 5.6 * GIB as f64, 5.0),
            "kv {}",
            e.kv_bytes
        );
        assert_eq!(e.weights_bytes, weights);
        // Overhead: KV(256) = 36 MiB + 5 % of weights.
        let kv256 = 36 * 256 * 8 * 512;
        assert_eq!(e.overhead_bytes, kv256 + weights / 20);
        assert_eq!(e.total_bytes, weights + e.kv_bytes + e.overhead_bytes);
        assert_eq!(e.verdict, FitVerdict::Fits);
        assert_eq!(e.context_length, 40960);

        // Defaulting k/v length from n_embd / n_head gives the same result.
        let mut s2 = s.clone();
        s2.key_length = None;
        s2.value_length = None;
        assert_eq!(
            kv_cache_bytes(&s2, 40960, KvCacheType::F16, 1),
            kv_cache_bytes(&s, 40960, KvCacheType::F16, 1)
        );
    }

    #[test]
    fn gemma3_4b_swa_golden() {
        let kv = kv_cache_bytes(&gemma3_4b(), 32768, KvCacheType::F16, 1).unwrap();
        assert!(within(kv as f64, 0.85e9, 5.0), "kv {kv}");
        // Without SWA awareness it would be far larger.
        let mut no_swa = gemma3_4b();
        no_swa.sliding_window = None;
        assert!(kv_cache_bytes(&no_swa, 32768, KvCacheType::F16, 1).unwrap() > 4 * kv);
    }

    #[test]
    fn kv_types_and_slots_scale() {
        let s = qwen3_8b();
        let f16 = kv_cache_bytes(&s, 8192, KvCacheType::F16, 1).unwrap() as f64;
        let q8 = kv_cache_bytes(&s, 8192, KvCacheType::Q8_0, 1).unwrap() as f64;
        let q4 = kv_cache_bytes(&s, 8192, KvCacheType::Q4_0, 1).unwrap() as f64;
        assert!(within(q8, f16 * 1.0625 / 2.0, 0.01));
        assert!(within(q4, f16 * 0.5625 / 2.0, 0.01));
        let two = kv_cache_bytes(&s, 8192, KvCacheType::F16, 2).unwrap() as f64;
        assert!(within(two, 2.0 * f16, 0.01));
        // Slot count 0 behaves like 1.
        assert_eq!(
            kv_cache_bytes(&s, 8192, KvCacheType::F16, 0),
            kv_cache_bytes(&s, 8192, KvCacheType::F16, 1)
        );
    }

    #[test]
    fn verdict_thresholds() {
        // No summary: total = weights × 1.05.
        let weights = 1_000_000_000u64;
        let total = weights + weights / 20;
        let at = |budget: u64| {
            estimate(
                weights,
                None,
                4096,
                KvCacheType::F16,
                1,
                &hw(Some(budget), 0),
            )
        };
        assert_eq!(at(total * 2).verdict, FitVerdict::Fits);
        assert_eq!(at((total as f64 / 0.9) as u64).verdict, FitVerdict::Tight);
        assert_eq!(at(total).verdict, FitVerdict::Tight);
        assert_eq!(at(total - 1).verdict, FitVerdict::TooLarge);
        assert_eq!(at(total).kv_bytes, 0);

        // Unknown GPU → available RAM is the budget.
        let e = estimate(weights, None, 4096, KvCacheType::F16, 1, &hw(None, 8 * GIB));
        assert_eq!(e.budget_bytes, Some(8 * GIB));
        assert_eq!(e.verdict, FitVerdict::Fits);

        // Nothing known at all.
        let e = estimate(0, None, 4096, KvCacheType::F16, 1, &hw(None, 8 * GIB));
        assert_eq!(e.verdict, FitVerdict::Unknown);
        // No budget.
        let e = estimate(weights, None, 4096, KvCacheType::F16, 1, &hw(None, 0));
        assert_eq!(e.verdict, FitVerdict::Unknown);
    }

    #[test]
    fn zero_context_uses_trained() {
        let s = qwen3_8b();
        let e = estimate(1, Some(&s), 0, KvCacheType::F16, 1, &hw(None, GIB));
        assert_eq!(e.context_length, 40960);
        let e = estimate(1, None, 0, KvCacheType::F16, 1, &hw(None, GIB));
        assert_eq!(e.context_length, 4096);
    }

    #[test]
    fn max_context() {
        let s = qwen3_8b();
        let weights = 5_030_000_000u64;
        // Plenty of memory: capped at the trained 40960 → 32768.
        assert_eq!(
            max_context_that_fits(
                weights,
                Some(&s),
                KvCacheType::F16,
                1,
                &hw(Some(64 * GIB), 0)
            ),
            Some(32768)
        );
        // 8 GiB: weights + overhead ≈ 5.32 GB; 16K ctx (2.25 GiB KV) is Tight
        // (90 %), 32K is too large.
        let ctx = max_context_that_fits(
            weights,
            Some(&s),
            KvCacheType::F16,
            1,
            &hw(Some(8 * GIB), 0),
        );
        assert_eq!(ctx, Some(16384));
        // Too small for anything.
        assert_eq!(
            max_context_that_fits(weights, Some(&s), KvCacheType::F16, 1, &hw(Some(GIB), 0)),
            None
        );
        // No summary → unknown.
        assert_eq!(
            max_context_that_fits(weights, None, KvCacheType::F16, 1, &hw(Some(64 * GIB), 0)),
            None
        );
        // Short trained context is offered as-is.
        let mut short = qwen3_8b();
        short.context_length = Some(2048);
        assert_eq!(
            max_context_that_fits(
                1000,
                Some(&short),
                KvCacheType::F16,
                1,
                &hw(Some(64 * GIB), 0)
            ),
            Some(2048)
        );
    }
}
