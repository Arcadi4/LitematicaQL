//! Ordered, bounded worker windows. Only the coordinator calls host callbacks.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::sync_channel,
    Mutex,
};

pub(crate) const BATCH_BLOCKS: usize = 16 * 1024;

pub(crate) fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) {
        Err("Parallel preview cancelled.".into())
    } else {
        Ok(())
    }
}

/// Persistent scoped workers each own one task/result slot. At most `workers`
/// outputs are retained, results are consumed in input order, and every exit
/// closes its channels and joins its workers.
pub(crate) fn ordered<T: Send, R: Send>(
    mut jobs: impl Iterator<Item = Result<T, String>>,
    workers: usize,
    speed_first: bool,
    work: impl Fn(T, &AtomicBool) -> Result<R, String> + Sync,
    mut consume: impl FnMut(R) -> Result<(), String>,
    current: &impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    let workers = workers.clamp(1, 8).min(jobs.size_hint().1.unwrap_or(8));
    if workers == 0 {
        return current();
    }
    let cancelled = AtomicBool::new(false);
    let worker_error = Mutex::new(None::<String>);
    std::thread::scope(|scope| {
        let mut inputs = Vec::with_capacity(workers);
        let mut outputs = Vec::with_capacity(workers);
        let mut handles = Vec::with_capacity(workers);
        let result = (|| {
            for _ in 0..workers {
                current()?;
                let (input, receive) = sync_channel::<T>(1);
                let (send, output) = sync_channel(1);
                let work = &work;
                let cancelled = &cancelled;
                let worker_error = &worker_error;
                let handle = std::thread::Builder::new()
                    .name("preview-compute".into())
                    .spawn_scoped(scope, move || {
                        while let Ok(job) = receive.recv() {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    check_cancelled(cancelled)?;
                                    work(job, cancelled)
                                }))
                                .unwrap_or_else(|_| {
                                    Err("A parallel preview worker panicked.".into())
                                });
                            if let Err(error) = &result {
                                worker_error
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .get_or_insert_with(|| error.clone());
                                cancelled.store(true, Ordering::Relaxed);
                            }
                            if send.send(result).is_err() {
                                break;
                            }
                        }
                    })
                    .map_err(|error| format!("Unable to start preview worker: {error}"))?;
                inputs.push(input);
                outputs.push(output);
                handles.push(handle);
            }
            let mut admit = |input: &std::sync::mpsc::SyncSender<T>| {
                current()?;
                if cancelled.load(Ordering::Relaxed) {
                    return Ok(false);
                }
                let Some(job) = jobs.next() else {
                    return Ok(false);
                };
                let job = job?;
                if cancelled.load(Ordering::Relaxed) {
                    return Ok(false);
                }
                input
                    .send(job)
                    .map_err(|_| "A parallel preview worker stopped.".to_string())?;
                Ok::<_, String>(true)
            };
            let mut admitted = 0;
            let mut exhausted = false;
            loop {
                if admitted == 0 {
                    for input in &inputs {
                        if !admit(input)? {
                            break;
                        }
                        admitted += 1;
                    }
                }
                if admitted == 0 {
                    return worker_error
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone()
                        .map_or(Ok(()), Err);
                }
                let mut refilled = 0;
                for (slot, output) in outputs.iter().take(admitted).enumerate() {
                    let result = (|| loop {
                        match output.recv_timeout(std::time::Duration::from_millis(100)) {
                            Ok(result) => break Ok(result),
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                                current()?;
                                if let Some(error) = worker_error
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .clone()
                                {
                                    return Err(error);
                                }
                            }
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                return Err("A parallel preview worker stopped.".into());
                            }
                        }
                    })();
                    let result = result?;
                    current()?;
                    if let Some(error) = worker_error
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone()
                    {
                        return Err(error);
                    }
                    consume(result?)?;
                    if speed_first && !exhausted && admit(&inputs[slot])? {
                        refilled += 1;
                    } else if speed_first {
                        exhausted = true;
                    }
                }
                if speed_first && refilled == 0 {
                    return worker_error
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone()
                        .map_or(Ok(()), Err);
                }
                admitted = refilled;
            }
        })();
        cancelled.store(true, Ordering::Relaxed);
        drop(inputs);
        drop(outputs);
        let mut result = result;
        for handle in handles {
            if handle.join().is_err() && result.is_ok() {
                result = Err("A parallel preview worker panicked.".into());
            }
        }
        result
    })
}
