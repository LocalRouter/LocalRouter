# Compression Model Benchmarks

**Date**: 2026-03-09
**Machine**: Apple Silicon (macOS), Metal GPU acceleration
**Build**: Release (`--release`)
**Model**: BERT Base Multilingual Cased (LLMLingua-2)
**Framework**: Candle (pure-Rust ML) with Metal backend

## BERT Base Results

| Metric | Value |
|--------|-------|
| Disk Size | 676.5 MB (~0.7 GB) |
| Cold Start | 1,044 ms (~1s) |
| Memory (RSS delta) | 736.9 MB (~0.7 GB) |

### Per-Request Latency (10 iterations, rate=0.5)

| Text Size | Words | Avg | Min | Max |
|-----------|-------|-----|-----|-----|
| Short | 11 | 9.1ms | 8.9ms | 9.8ms |
| Medium | 95 | 24.1ms | 22.9ms | 32.5ms |
| Long | 283 | 103.1ms | 100.4ms | 104.3ms |

### Debug Build Comparison

For reference, debug build numbers (significantly slower due to no optimizations):

| Metric | Debug | Release | Speedup |
|--------|-------|---------|---------|
| Cold Start | 3,490ms | 1,044ms | 3.3x |
| Memory | 860 MB | 737 MB | 1.2x |
| Latency (short) | 12.3ms | 9.1ms | 1.4x |
| Latency (medium) | 29.6ms | 24.1ms | 1.2x |
| Latency (long) | 115.1ms | 103.1ms | 1.1x |

## XLM-RoBERTa Large (Estimated)

Not benchmarked (model not downloaded). Estimates based on architecture differences:
- 24 layers vs 12 (BERT), hidden_size 1024 vs 768
- Roughly ~2.5-3x compute and memory

| Metric | Estimated |
|--------|-----------|
| Disk Size | ~2.2 GB |
| Cold Start | ~2.5s |
| Memory | ~2 GB |
| Latency | ~30-300ms |

## UI Constants

Values used in `src/components/compression/types.ts` for the Resource Requirements card:

```typescript
export const COMPRESSION_REQUIREMENTS = {
  bert: {
    DISK_GB: '~0.7',
    MEMORY_GB: '~0.7',
    COLD_START_SECS: '~1',
    PER_REQUEST_MS: '~10-100',
  },
  'xlm-roberta': {
    DISK_GB: '~2.2',
    MEMORY_GB: '~2',
    COLD_START_SECS: '~2.5',
    PER_REQUEST_MS: '~30-300',
  },
};
```

## How to Re-run

```bash
# Ensure model is downloaded via the UI first
LOCALROUTER_ENV=dev cargo test -p lr-compression --test benchmark --release -- --nocapture --ignored
```

## Notes

- Latency scales roughly linearly with word count (BERT max sequence length is 512 tokens)
- Metal acceleration is used automatically on macOS; CPU fallback on Linux/Windows
- Memory is measured via RSS delta (process memory before/after model load)
- Cold start includes SafeTensors mmap + tokenizer load + Metal shader compilation
