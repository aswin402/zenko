//! Deterministic 3-State Parallel Tool Wave Scheduler.
//! Implements resource-key lock algebra over `ToolConcurrency`:
//! - `Shared` calls coalesce into bounded concurrent waves.
//! - `Keyed(k)` calls targeting *distinct* keys coalesce into concurrent waves, while
//!   conflicting writes to the same key serialize across consecutive waves.
//! - Transitions between `Shared` and `Keyed` flush the active wave to preserve strict
//!   read-after-write and write-after-read causal consistency.
//! - `Global` calls act as strict exclusive barriers.
//! Every result is reassembled in exact original `tool_calls` order.

use futures_util::stream::{self, StreamExt};
use std::collections::HashSet;
use std::future::Future;
use zenko_core::{ResourceLockKey, Result, ToolConcurrency};

#[derive(Debug, PartialEq, Eq)]
pub enum ToolWave<T> {
    Parallel(Vec<(usize, T)>),
    Sequential(usize, T),
}

enum ActiveBatch<T> {
    Empty,
    Shared(Vec<(usize, T)>),
    Keyed {
        items: Vec<(usize, T)>,
        keys: HashSet<ResourceLockKey>,
    },
}

impl<T> ActiveBatch<T> {
    fn flush_into(&mut self, waves: &mut Vec<ToolWave<T>>) {
        let taken = std::mem::replace(self, Self::Empty);
        let items = match taken {
            Self::Empty => return,
            Self::Shared(v) => v,
            Self::Keyed { items, .. } => items,
        };
        let mut iter = items.into_iter();
        let Some(first) = iter.next() else {
            return;
        };
        match iter.next() {
            None => waves.push(ToolWave::Sequential(first.0, first.1)),
            Some(second) => {
                let mut batch = Vec::with_capacity(2 + iter.len());
                batch.push(first);
                batch.push(second);
                batch.extend(iter);
                waves.push(ToolWave::Parallel(batch));
            }
        }
    }
}

/// Partitions an ordered list of tool invocations into deterministic parallel & sequential waves.
pub fn partition_tool_waves<T, F>(items: Vec<T>, classify: F) -> Vec<ToolWave<T>>
where
    F: Fn(&T) -> ToolConcurrency,
{
    let mut waves: Vec<ToolWave<T>> = Vec::with_capacity(items.len());
    let mut active: ActiveBatch<T> = ActiveBatch::Empty;

    for (idx, item) in items.into_iter().enumerate() {
        match classify(&item) {
            ToolConcurrency::Shared => match &mut active {
                ActiveBatch::Shared(batch) => batch.push((idx, item)),
                ActiveBatch::Empty => active = ActiveBatch::Shared(vec![(idx, item)]),
                ActiveBatch::Keyed { .. } => {
                    active.flush_into(&mut waves);
                    active = ActiveBatch::Shared(vec![(idx, item)]);
                }
            },
            ToolConcurrency::Keyed(key) => match &mut active {
                ActiveBatch::Keyed {
                    items: batch_items,
                    keys,
                } if !keys.contains(&key) => {
                    keys.insert(key);
                    batch_items.push((idx, item));
                }
                _ => {
                    active.flush_into(&mut waves);
                    let mut keys = HashSet::with_capacity(4);
                    keys.insert(key);
                    active = ActiveBatch::Keyed {
                        items: vec![(idx, item)],
                        keys,
                    };
                }
            },
            ToolConcurrency::Global => {
                active.flush_into(&mut waves);
                waves.push(ToolWave::Sequential(idx, item));
            }
        }
    }

    active.flush_into(&mut waves);
    waves
}

/// Executes partitioned tool waves with bounded concurrency and returns outputs strictly in
/// original input index order.
pub async fn execute_tool_waves<T, R, Fut, C, E>(
    items: Vec<T>,
    max_parallel: usize,
    classify: C,
    exec: E,
) -> Result<Vec<R>>
where
    C: Fn(&T) -> ToolConcurrency,
    E: Fn(T) -> Fut,
    Fut: Future<Output = Result<R>>,
{
    let total = items.len();
    let waves = partition_tool_waves(items, classify);
    let mut ordered: Vec<(usize, R)> = Vec::with_capacity(total);
    let concurrency = max_parallel.max(1);

    for wave in waves {
        match wave {
            ToolWave::Sequential(idx, item) => {
                let res = exec(item).await?;
                ordered.push((idx, res));
            }
            ToolWave::Parallel(batch) => {
                let mut stream = stream::iter(batch.into_iter().map(|(idx, item)| {
                    let fut = exec(item);
                    async move { fut.await.map(|r| (idx, r)) }
                }))
                .buffer_unordered(concurrency);

                while let Some(step) = stream.next().await {
                    ordered.push(step?);
                }
            }
        }
    }

    ordered.sort_by_key(|(idx, _)| *idx);
    Ok(ordered.into_iter().map(|(_, r)| r).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    #[test]
    fn partitions_shared_keyed_and_global_with_causal_barriers() {
        let items = vec![
            ("read_a", ToolConcurrency::Shared),
            ("read_b", ToolConcurrency::Shared),
            ("write_f1", ToolConcurrency::Keyed(ResourceLockKey::File("src/a.rs".into()))),
            ("write_f2", ToolConcurrency::Keyed(ResourceLockKey::File("src/b.rs".into()))),
            ("write_f1_again", ToolConcurrency::Keyed(ResourceLockKey::File("src/a.rs".into()))),
            ("read_after_write", ToolConcurrency::Shared),
            ("shell", ToolConcurrency::Global),
            ("read_c", ToolConcurrency::Shared),
            ("read_d", ToolConcurrency::Shared),
        ];
        let waves = partition_tool_waves(items, |(_, c)| c.clone());
        // Wave 0: [read_a, read_b] (Parallel)
        // Wave 1: [write_f1, write_f2] (Parallel disjoint keys)
        // Wave 2: write_f1_again (Sequential due to conflict with src/a.rs)
        // Wave 3: read_after_write (Sequential barrier after keyed write)
        // Wave 4: shell (Global Sequential barrier)
        // Wave 5: [read_c, read_d] (Parallel)
        assert_eq!(waves.len(), 6);
        assert!(matches!(&waves[0], ToolWave::Parallel(v) if v.len() == 2));
        assert!(matches!(&waves[1], ToolWave::Parallel(v) if v.len() == 2));
        assert!(matches!(&waves[2], ToolWave::Sequential(4, ("write_f1_again", _))));
        assert!(matches!(&waves[3], ToolWave::Sequential(5, ("read_after_write", _))));
        assert!(matches!(&waves[4], ToolWave::Sequential(6, ("shell", _))));
        assert!(matches!(&waves[5], ToolWave::Parallel(v) if v.len() == 2));
    }

    #[tokio::test]
    async fn executes_shared_and_disjoint_keyed_waves_concurrently_in_order() {
        let active = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));

        // 3 shared reads, then 2 disjoint keyed writes, then 1 conflicting keyed write, then 1 global
        let items = vec![
            (0usize, ToolConcurrency::Shared, 70u64),
            (1usize, ToolConcurrency::Shared, 35u64),
            (2usize, ToolConcurrency::Shared, 70u64),
            (3usize, ToolConcurrency::Keyed(ResourceLockKey::File("a.rs".into())), 60u64),
            (4usize, ToolConcurrency::Keyed(ResourceLockKey::File("b.rs".into())), 60u64),
            (5usize, ToolConcurrency::Keyed(ResourceLockKey::File("a.rs".into())), 20u64),
            (6usize, ToolConcurrency::Global, 20u64),
        ];

        let started = Instant::now();
        let active_ref = active.clone();
        let max_ref = max_seen.clone();
        let results = execute_tool_waves(
            items,
            4,
            |(_, c, _)| c.clone(),
            move |(id, mode, delay_ms)| {
                let active_c = active_ref.clone();
                let max_c = max_ref.clone();
                async move {
                    let cur = active_c.fetch_add(1, Ordering::SeqCst) + 1;
                    if mode == ToolConcurrency::Global {
                        assert_eq!(cur, 1, "global tool must run exclusively");
                    }
                    max_c.fetch_max(cur, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    active_c.fetch_sub(1, Ordering::SeqCst);
                    Ok(format!("res_{id}"))
                }
            },
        )
        .await
        .expect("execute waves");

        let elapsed = started.elapsed();
        assert_eq!(
            results,
            vec!["res_0", "res_1", "res_2", "res_3", "res_4", "res_5", "res_6"]
        );
        assert!(
            max_seen.load(Ordering::SeqCst) >= 3,
            "expected concurrent execution of waves, max_seen={}",
            max_seen.load(Ordering::SeqCst)
        );
        // Sequential sum would be 335ms; parallelized waves should take ~170ms (+scheduling jitter)
        assert!(
            elapsed < Duration::from_millis(290),
            "parallel waves took too long: {elapsed:?}"
        );
    }
}
