//! Bridge between GPUI's executors and reqwest.
//!
//! reqwest needs a Tokio reactor, which GPUI does not provide. Requests run on
//! one small shared Tokio runtime, and GPUI tasks await the `JoinHandle`,
//! which is executor-agnostic, so entity updates still land on the main thread.

use std::future::Future;
use std::sync::OnceLock;

use crate::api::ApiError;

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("mwgui-http")
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// Runs an API future on the HTTP runtime and resolves with its result.
pub async fn run<F, T>(future: F) -> Result<T, ApiError>
where
    F: Future<Output = Result<T, ApiError>> + Send + 'static,
    T: Send + 'static,
{
    runtime()
        .spawn(future)
        .await
        .map_err(|e| ApiError::Network(format!("request task stopped: {e}")))?
}
