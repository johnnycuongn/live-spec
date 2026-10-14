//! Runs a group of agents (any futures) with at most `limit` in flight and
//! returns their results in job order. Nothing is spawned: the jobs run
//! inside the caller's task, so dropping the group cancels every job, and
//! with it each `claude` child (`kill_on_drop`).

use std::future::Future;

use futures_util::stream::{self, StreamExt};

pub async fn run_limited<F: Future>(limit: usize, jobs: Vec<F>) -> Vec<F::Output> {
    let mut done: Vec<(usize, F::Output)> = stream::iter(jobs.into_iter().enumerate())
        .map(|(i, job)| async move { (i, job.await) })
        .buffer_unordered(limit.max(1))
        .collect()
        .await;
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, out)| out).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn results_come_back_in_job_order() {
        let jobs: Vec<_> = [30u64, 10, 20]
            .into_iter()
            .map(|ms| async move {
                tokio::time::sleep(Duration::from_millis(ms)).await;
                ms
            })
            .collect();
        assert_eq!(run_limited(3, jobs).await, [30, 10, 20]);
    }

    #[tokio::test]
    async fn never_runs_more_than_the_limit() {
        let (running, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let jobs: Vec<_> = (0..6)
            .map(|_| {
                let (running, peak) = (&running, &peak);
                async move {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    running.fetch_sub(1, Ordering::SeqCst);
                }
            })
            .collect();
        run_limited(2, jobs).await;
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_zero_limit_still_runs_every_job() {
        let jobs: Vec<_> = (0..3).map(|i| async move { i }).collect();
        assert_eq!(run_limited(0, jobs).await, [0, 1, 2]);
    }

    #[tokio::test]
    async fn dropping_the_group_cancels_every_job() {
        let finished = AtomicUsize::new(0);
        let jobs: Vec<_> = (0..3)
            .map(|_| {
                let finished = &finished;
                async move {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    finished.fetch_add(1, Ordering::SeqCst);
                }
            })
            .collect();
        let cut_short = tokio::time::timeout(Duration::from_millis(50), run_limited(3, jobs)).await;
        assert!(cut_short.is_err());
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(finished.load(Ordering::SeqCst), 0, "no job may keep running after the group is dropped");
    }
}
