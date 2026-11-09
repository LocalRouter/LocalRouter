//! Request-bound secret capabilities. Never accepted from wire extensions.
use crate::AppResult;
use serde_json::Value;
use std::{future::Future, sync::Arc};

pub trait SecretContext: Send + Sync {
    fn register(&self, secret: &str) -> AppResult<()>;
    fn protect(&self, value: &mut Value) -> AppResult<()>;
    /// Mask already-protected originals in diagnostic data without detecting
    /// new secrets or changing the outbound request's authorization.
    /// A diagnostic-only handle must not keep request retention pins alive.
    fn monitor_context(&self) -> Option<Arc<dyn SecretContext>> {
        None
    }
    fn redact_monitor(&self, _value: &mut Value) -> AppResult<()> {
        Ok(())
    }
    /// Scan newly introduced content, allowing asynchronous user decisions.
    fn prepare<'a>(
        &'a self,
        value: &'a mut Value,
    ) -> std::pin::Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>> {
        Box::pin(async move { self.protect(value) })
    }
    fn restore(&self, value: &mut Value, strict: bool) -> AppResult<()>;
    fn restore_text(&self, text: &str, strict: bool) -> AppResult<String>;
    fn restore_text_at(&self, text: &str, strict: bool, _location: &str) -> AppResult<String> {
        self.restore_text(text, strict)
    }
    /// Metadata-only local diagnostic. Must never include original secret bytes.
    fn diagnostic(&self, _message: &str) {}
    fn active(&self) -> bool;
    fn validate_tool(&self, _name: Option<&str>, _arguments: &Value) -> AppResult<()> {
        Ok(())
    }
    fn validate_output(&self, _text: &str) -> AppResult<()> {
        Ok(())
    }
    fn validate_response(&self, _value: &Value) -> AppResult<()> {
        Ok(())
    }
    fn structured_output(&self) -> bool {
        false
    }
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
