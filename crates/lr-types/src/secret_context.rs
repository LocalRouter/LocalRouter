//! Request-bound secret capabilities. Never accepted from wire extensions.
use crate::AppResult;
use serde_json::Value;
use std::{future::Future, sync::Arc};

pub trait SecretContext: Send + Sync {
    fn register(&self, secret: &str) -> AppResult<()>;
    fn protect(&self, value: &mut Value) -> AppResult<()>;
    fn restore(&self, value: &mut Value, strict: bool) -> AppResult<()>;
    fn restore_text(&self, text: &str, strict: bool) -> AppResult<String>;
    fn active(&self) -> bool;
    fn scanned(&self) -> bool {
        false
    }
    fn mark_scanned(&self) {}
}

tokio::task_local! {
    static CONTEXT: Option<Arc<dyn SecretContext>>;
}
pub fn current() -> Option<Arc<dyn SecretContext>> {
    CONTEXT.try_with(Clone::clone).ok().flatten()
}
pub async fn scope<F: Future>(context: Option<Arc<dyn SecretContext>>, future: F) -> F::Output {
    CONTEXT.scope(context, future).await
}
