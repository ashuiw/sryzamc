use criterion::{criterion_group, criterion_main, Criterion};
use std::collections::BinaryHeap;
fn schedule_insert(c: &mut Criterion) {
    c.bench_function("scheduler_enqueue_100k", |b| {
        b.iter(|| {
            let mut q = BinaryHeap::new();
            for i in 0..100_000u64 {
                q.push(std::cmp::Reverse((i % 1000, i)));
            }
            q.len()
        })
    });
}
criterion_group!(benches, schedule_insert);
criterion_main!(benches);
