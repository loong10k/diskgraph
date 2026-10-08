//! 固定四线程以内的批次采样；输入/输出顺序不变，不创建全局线程池或任务队列。

/// 参数：items 为当前暂存批次，work 为每个连续分片的独立采样器。
/// 返回：按输入分片顺序合并的结果；原错误与 panic 保留，空批次不启动工作。
pub(super) fn collect_ordered<T, U, E, F>(items: &[T], work: &F) -> Result<Vec<U>, E>
where
    T: Sync,
    U: Send,
    E: Send,
    F: Fn(&[T]) -> Result<Vec<U>, E> + Sync,
{
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let chunk_size = items.len().div_ceil(4);
    std::thread::scope(|scope| {
        let workers: Vec<_> = items
            .chunks(chunk_size)
            .map(|chunk| scope.spawn(move || work(chunk)))
            .collect();
        // 先收齐本批所有线程，再传播错误；不让暂存或下一批穿过尚未结束的采样。
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect();
        results
            .into_iter()
            .try_fold(Vec::with_capacity(items.len()), |mut all, chunk| {
                all.extend(chunk?);
                Ok(all)
            })
    })
}

#[cfg(test)]
mod tests {
    use super::collect_ordered;
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    #[test]
    fn four_bounded_workers_overlap_and_preserve_input_order() {
        let gate = (Mutex::new(0_usize), Condvar::new());
        let input: Vec<usize> = (0..103).collect();
        let output = collect_ordered(&input, &|chunk: &[usize]| -> Result<Vec<usize>, ()> {
            let (lock, ready) = &gate;
            let mut arrived = lock.lock().unwrap();
            *arrived += 1;
            ready.notify_all();
            let (arrived, _) = ready
                .wait_timeout_while(arrived, Duration::from_secs(1), |count| *count < 4)
                .unwrap();
            assert_eq!(
                *arrived, 4,
                "sampling did not overlap within the worker bound"
            );
            drop(arrived);
            // 后面的分片先完成，最终仍须保持节点与定位的原顺序。
            std::thread::sleep(Duration::from_millis((103 - chunk[0]) as u64));
            Ok(chunk.to_vec())
        })
        .unwrap();
        assert_eq!(output, input);
        assert_eq!(*gate.0.lock().unwrap(), 4);
    }

    #[test]
    fn empty_batch_never_calls_the_sampler() {
        let output: Vec<usize> = collect_ordered(&[], &|_: &[usize]| -> Result<Vec<usize>, ()> {
            panic!("empty input must not launch a worker")
        })
        .unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn original_error_and_panic_payload_are_preserved() {
        let error = collect_ordered(&[1, 2, 3, 4], &|chunk: &[i32]| {
            if chunk.contains(&3) {
                Err(37_u8)
            } else {
                Ok(chunk.to_vec())
            }
        });
        assert_eq!(error, Err(37));
        let panic = std::panic::catch_unwind(|| {
            let _: Result<Vec<i32>, ()> =
                collect_ordered(&[1], &|_: &[i32]| std::panic::panic_any(43_u8));
        })
        .unwrap_err();
        assert_eq!(*panic.downcast::<u8>().unwrap(), 43);
    }
}
