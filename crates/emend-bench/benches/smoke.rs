//! Smoke benchmark: verifies the Criterion pipeline compiles and runs.
//!
//! It measures a trivial, allocation-free `emend-core` call so that
//! `cargo bench` exercises the full harness before the real perf benches
//! (highlight, quick-open) arrive in later phases. Keep this free of
//! `unwrap`/`expect`/`panic` to honour the workspace lint policy (NFR-003).

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use emend_core::U16Range;

fn bench_u16range_end(c: &mut Criterion) {
    c.bench_function("u16range_end", |b| {
        b.iter(|| U16Range::new(black_box(3), black_box(4)).end());
    });
}

criterion_group!(benches, bench_u16range_end);
criterion_main!(benches);
