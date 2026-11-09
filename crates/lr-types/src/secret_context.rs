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
    fn protection_diagnostic(&self, message: &str) {
        self.diagnostic(message);
    }
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

/// Transactional request adapter. Only an explicit human denial is fatal.
pub fn protect(context: &dyn SecretContext, value: &mut Value) -> AppResult<()> {
    let mut candidate = value.clone();
    match context.protect(&mut candidate) {
        Ok(()) => *value = candidate,
        Err(crate::AppError::SecretScanDenied) => return Err(crate::AppError::SecretScanDenied),
        Err(_) => context
            .protection_diagnostic("Secret masking could not complete; original content forwarded"),
    }
    Ok(())
}
pub async fn prepare(context: &dyn SecretContext, value: &mut Value) -> AppResult<()> {
    let mut candidate = value.clone();
    match context.prepare(&mut candidate).await {
        Ok(()) => *value = candidate,
        Err(crate::AppError::SecretScanDenied) => return Err(crate::AppError::SecretScanDenied),
        Err(_) => context.protection_diagnostic(
            "Secret Scan could not complete for follow-up content; original content forwarded",
        ),
    }
    Ok(())
}

#[cfg(test)]
mod nonblocking_tests {
    use super::*;
    struct Failure(bool);
    impl SecretContext for Failure {
        fn register(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        fn protect(&self, value: &mut Value) -> AppResult<()> {
            *value = serde_json::json!({"partial":"must not leak"});
            if self.0 {
                Err(crate::AppError::SecretScanDenied)
            } else {
                Err(crate::AppError::Internal("fixture".into()))
            }
        }
        fn restore(&self, _: &mut Value, _: bool) -> AppResult<()> {
            Ok(())
        }
        fn restore_text(&self, s: &str, _: bool) -> AppResult<String> {
            Ok(s.into())
        }
        fn active(&self) -> bool {
            true
        }
    }
    #[tokio::test]
    async fn adapters_rollback_partial_mutations_and_only_propagate_human_denial() {
        for deny in [false, true] {
            let context = Failure(deny);
            let mut request = serde_json::json!({"original":true});
            let original = request.clone();
            assert_eq!(protect(&context, &mut request).is_err(), deny);
            assert_eq!(request, original);
            assert_eq!(prepare(&context, &mut request).await.is_err(), deny);
            assert_eq!(request, original);
        }
    }
}
