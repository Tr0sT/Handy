//! Shared lifecycle primitives, kept independent of Tauri for regression tests.

use std::future::Future;
use std::time::Duration;

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

pub(crate) async fn complete_unless_cancelled<F, C>(
    operation: F,
    is_cancelled: C,
) -> Option<F::Output>
where
    F: Future,
    C: Fn() -> bool,
{
    tokio::pin!(operation);
    loop {
        if is_cancelled() {
            return None;
        }
        if let Ok(result) =
            tokio::time::timeout(CANCELLATION_POLL_INTERVAL, operation.as_mut()).await
        {
            // Cancellation may have raced with the successful last poll.
            return (!is_cancelled()).then_some(result);
        }
    }
}

/// Commit audio/history before invoking a fallible or cancellable provider.
/// The outer Result is persistence; the provider keeps its own failure outcome.
pub(crate) async fn persist_then_transcribe<P, F, T, S>(
    persist: P,
    transcribe: F,
) -> Result<(S, T::Output), String>
where
    P: Future<Output = Result<S, String>>,
    F: FnOnce() -> T,
    T: Future,
{
    let saved = persist.await?;
    Ok((saved, transcribe().await))
}

/// Local inference may still own an engine after cancellation, so drain it
/// before allowing a new recording. Cloud futures can be dropped immediately.
pub(crate) async fn run_provider<L, H, C>(
    cloud: bool,
    local: L,
    http: H,
    is_cancelled: C,
) -> Result<Option<String>, String>
where
    L: Future<Output = Result<String, String>>,
    H: Future<Output = Result<String, String>>,
    C: Fn() -> bool,
{
    if is_cancelled() {
        return Ok(None);
    }
    if cloud {
        complete_unless_cancelled(http, is_cancelled)
            .await
            .transpose()
    } else {
        let result = local.await;
        if is_cancelled() {
            Ok(None)
        } else {
            result.map(Some)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    #[tokio::test]
    async fn persistence_precedes_provider_and_survives_all_failure_outcomes() {
        for outcome in [
            Err("HTTP 500".to_owned()),
            Err("timeout".to_owned()),
            Err("HTTP 401".to_owned()),
            Ok(Some(String::new())),
            Ok(None),
        ] {
            let persisted = Cell::new(false);
            let result = persist_then_transcribe(
                async {
                    persisted.set(true);
                    Ok::<_, String>(42)
                },
                || async {
                    assert!(persisted.get());
                    outcome.clone()
                },
            )
            .await
            .unwrap();
            assert_eq!(result, (42, outcome));
            assert!(persisted.get());
        }
    }

    #[tokio::test]
    async fn failed_persistence_does_not_start_upload() {
        let called = Cell::new(false);
        let result =
            persist_then_transcribe(async { Err::<(), _>("disk full".into()) }, || async {
                called.set(true);
            })
            .await;
        assert!(result.is_err());
        assert!(!called.get());
    }

    #[tokio::test]
    async fn batch_and_live_dispatch_to_the_selected_provider_only() {
        for cloud in [true, false] {
            let local_calls = Cell::new(0);
            let cloud_calls = Cell::new(0);
            let result = run_provider(
                cloud,
                async {
                    local_calls.set(1);
                    Ok("local".into())
                },
                async {
                    cloud_calls.set(1);
                    Ok("cloud".into())
                },
                || false,
            )
            .await
            .unwrap();
            assert_eq!(
                result.as_deref(),
                Some(if cloud { "cloud" } else { "local" })
            );
            assert_eq!(local_calls.get(), i32::from(!cloud));
            assert_eq!(cloud_calls.get(), i32::from(cloud));
        }
    }

    #[tokio::test]
    async fn cancellation_drops_stalled_cloud_future_and_allows_the_next_request() {
        struct DropFlag(Arc<AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&cancelled);
        let flag = DropFlag(Arc::clone(&dropped));
        let cancel = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            signal.store(true, Ordering::SeqCst);
        });
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            run_provider(
                true,
                async { panic!("local provider must not run") },
                async move {
                    let _flag = flag;
                    std::future::pending::<Result<String, String>>().await
                },
                || cancelled.load(Ordering::SeqCst),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        cancel.await.unwrap();
        assert_eq!(result, None);
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(
            run_provider(
                true,
                async { panic!("wrong provider") },
                async { Ok("next".into()) },
                || false
            )
            .await
            .unwrap()
            .as_deref(),
            Some("next")
        );
    }

    #[tokio::test]
    async fn cancellation_does_not_detach_local_engine_work() {
        let cancelled = Cell::new(false);
        let drained = Cell::new(false);
        let result = run_provider(
            false,
            async {
                cancelled.set(true);
                tokio::task::yield_now().await;
                drained.set(true);
                Ok("old".into())
            },
            async { panic!("wrong provider") },
            || cancelled.get(),
        )
        .await
        .unwrap();
        assert_eq!(result, None);
        assert!(drained.get());
    }

    #[tokio::test]
    async fn final_poll_cancellation_discards_the_result() {
        let cancelled = Cell::new(false);
        let result = complete_unless_cancelled(
            async {
                cancelled.set(true);
                "old"
            },
            || cancelled.get(),
        )
        .await;
        assert_eq!(result, None);
    }
}
