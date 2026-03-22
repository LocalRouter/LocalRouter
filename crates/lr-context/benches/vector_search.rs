//! Benchmark: FTS5-only vs FTS5+vector search overhead.
//!
//! Measures indexing and search latency across small/medium/large content sizes,
//! with and without the embedding service attached.
//!
//! Run: `cargo bench -p lr-context --bench vector_search --features vector`
//!
//! Requires the embedding model to be downloaded. Skips gracefully if not available.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::sync::Arc;

use lr_context::ContentStore;

/// Try to create an EmbeddingService from the default config dir.
/// Returns None if the model is not downloaded (skips vector benchmarks).
fn try_embedding_service() -> Option<Arc<lr_embeddings::EmbeddingService>> {
    let home = dirs::home_dir()?;
    // Try dev config dir first, then production
    let dev_dir = home.join(".localrouter-dev");
    let prod_dir = home.join(".localrouter");
    let config_dir = if dev_dir.exists() { dev_dir } else { prod_dir };

    let service = Arc::new(lr_embeddings::EmbeddingService::new(&config_dir));
    if !service.is_downloaded() {
        eprintln!("Embedding model not downloaded — skipping vector benchmarks");
        return None;
    }
    if let Err(e) = service.ensure_loaded() {
        eprintln!(
            "Failed to load embedding model: {} — skipping vector benchmarks",
            e
        );
        return None;
    }
    Some(service)
}

/// Generate content of approximately the given byte size.
fn generate_content(approx_bytes: usize) -> String {
    let paragraph = "The Rust programming language is designed for performance and safety. \
        It provides memory safety without garbage collection through its ownership system. \
        Rust's type system and borrow checker ensure thread safety at compile time. \
        The language supports zero-cost abstractions, move semantics, and pattern matching.\n\n";

    let heading = "## Section\n\n";
    let block_size = heading.len() + paragraph.len();
    let repeats = (approx_bytes / block_size).max(1);

    let mut content = String::with_capacity(approx_bytes + 256);
    content.push_str("# Benchmark Document\n\n");
    for i in 0..repeats {
        content.push_str(&format!("## Section {}\n\n", i + 1));
        content.push_str(paragraph);
    }
    content
}

fn bench_index(c: &mut Criterion) {
    let embedding_service = try_embedding_service();

    let sizes: &[(&str, usize)] = &[
        ("small_1KB", 1_024),
        ("medium_10KB", 10_240),
        ("large_100KB", 102_400),
    ];

    let mut group = c.benchmark_group("index");
    group.sample_size(20);

    for (name, size) in sizes {
        let content = generate_content(*size);

        // FTS5-only
        group.bench_with_input(
            BenchmarkId::new("fts5_only", name),
            &content,
            |b, content| {
                b.iter(|| {
                    let store = ContentStore::new().unwrap();
                    store
                        .index(black_box("bench/doc"), black_box(content))
                        .unwrap();
                });
            },
        );

        // FTS5 + vector (if available)
        if let Some(ref es) = embedding_service {
            group.bench_with_input(
                BenchmarkId::new("fts5_vector", name),
                &content,
                |b, content| {
                    b.iter(|| {
                        let store = ContentStore::new().unwrap();
                        store.set_embedding_service(Arc::clone(es));
                        store
                            .index(black_box("bench/doc"), black_box(content))
                            .unwrap();
                    });
                },
            );
        }
    }

    group.finish();
}

fn bench_search(c: &mut Criterion) {
    let embedding_service = try_embedding_service();

    let sizes: &[(&str, usize)] = &[
        ("small_1KB", 1_024),
        ("medium_10KB", 10_240),
        ("large_100KB", 102_400),
    ];

    let mut group = c.benchmark_group("search");
    group.sample_size(30);

    for (name, size) in sizes {
        let content = generate_content(*size);

        // FTS5-only store
        let fts_store = ContentStore::new().unwrap();
        fts_store.index("bench/doc", &content).unwrap();

        group.bench_with_input(
            BenchmarkId::new("fts5_only", name),
            &fts_store,
            |b, store| {
                b.iter(|| {
                    store
                        .search(
                            black_box(&["programming language safety".to_string()]),
                            black_box(5),
                            None,
                            &Default::default(),
                        )
                        .unwrap();
                });
            },
        );

        // FTS5 + vector store (if available)
        if let Some(ref es) = embedding_service {
            let vec_store = ContentStore::new().unwrap();
            vec_store.set_embedding_service(Arc::clone(es));
            vec_store.index("bench/doc", &content).unwrap();

            group.bench_with_input(
                BenchmarkId::new("fts5_vector", name),
                &vec_store,
                |b, store| {
                    b.iter(|| {
                        store
                            .search(
                                black_box(&["programming language safety".to_string()]),
                                black_box(5),
                                None,
                                &Default::default(),
                            )
                            .unwrap();
                    });
                },
            );
        }
    }

    group.finish();
}

fn bench_rebuild_vectors(c: &mut Criterion) {
    let embedding_service = match try_embedding_service() {
        Some(es) => es,
        None => return,
    };

    let entry_counts: &[(&str, usize)] = &[("50_entries", 50), ("200_entries", 200)];

    let mut group = c.benchmark_group("rebuild_vectors");
    group.sample_size(10);

    for (name, count) in entry_counts {
        // Pre-build a store with N entries (FTS5 only)
        let store = ContentStore::new().unwrap();
        for i in 0..*count {
            let label = format!("entry/{}", i);
            let content = format!(
                "## Entry {}\n\nThis is entry number {} with some searchable content about topic {}.\n",
                i, i, i % 10
            );
            store.index(&label, &content).unwrap();
        }

        group.bench_with_input(BenchmarkId::new("rebuild", name), &store, |b, store| {
            b.iter(|| {
                // Attach service and rebuild
                store.set_embedding_service(Arc::clone(&embedding_service));
                store.rebuild_vectors().unwrap();
            });
        });
    }

    group.finish();
}

fn bench_cold_start(c: &mut Criterion) {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return,
    };
    let dev_dir = home.join(".localrouter-dev");
    let prod_dir = home.join(".localrouter");
    let config_dir = if dev_dir.exists() { dev_dir } else { prod_dir };

    let test_service = lr_embeddings::EmbeddingService::new(&config_dir);
    if !test_service.is_downloaded() {
        return;
    }

    let mut group = c.benchmark_group("cold_start");
    group.sample_size(10);

    group.bench_function("ensure_loaded", |b| {
        b.iter(|| {
            // Create a fresh service each iteration to measure cold load
            let service = lr_embeddings::EmbeddingService::new(&config_dir);
            service.ensure_loaded().unwrap();
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_index,
    bench_search,
    bench_rebuild_vectors,
    bench_cold_start,
);
criterion_main!(benches);
