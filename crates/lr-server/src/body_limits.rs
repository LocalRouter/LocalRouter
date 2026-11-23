//! Native API request-body limits and the in-flight large-body budget.
//!
//! The native server holds whole requests in memory (typed translation,
//! routing, compression, caching and failover need them), so two bounds apply
//! to every request on the main API router:
//!
//! - a per-request limit, read live from `server.max_request_body_bytes`
//!   (file uploads keep their fixed limit), enforced on `Content-Length`
//!   before the body is read and on the bytes actually received;
//! - a global budget for large bodies. A body above the threshold
//!   (`server.large_body_threshold_bytes`, 8 MiB by default) reserves permits
//!   for its whole length (from `Content-Length` before it is read, or as
//!   chunks arrive) from the budget (`server.large_body_budget_bytes`,
//!   512 MiB). The reservation lasts until the response body is finished,
//!   because the parsed request lives that long. A request that cannot
//!   reserve within the wait (`server.large_body_wait_ms`, 3 s) gets a 503
//!   with `Retry-After`. All three are read live per request; a changed
//!   budget takes effect for new requests while requests already holding
//!   permits release them into the budget they came from, so the bytes held
//!   at once briefly stay within the old plus the new budget.
//!
//! Every layer below reads the same live limit, and a body rejected here is
//! answered with the structured 413/503 regardless of how the extractor that
//! saw the error would have mapped it.
use crate::middleware::error::ApiErrorResponse;
use axum::{
    body::{Body, Bytes},
    extract::{Request, State},
    http::{header::CONTENT_LENGTH, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use http_body::{Frame, SizeHint};
use parking_lot::Mutex;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::PollSemaphore;

const MIB: usize = 1024 * 1024;
/// `Retry-After` sent with a budget refusal.
pub const BUDGET_RETRY_AFTER_SECS: u64 = 2;
/// One permit covers this many bytes, keeping permit counts within `u32`.
const UNIT: usize = 1024;
/// File uploads: the largest file plus multipart framing and control fields.
pub const FILE_UPLOAD_LIMIT: usize = lr_batches::MAX_FILE_BYTES + 64 * 1024;

const ACCEPTED: u8 = 0;
const TOO_LARGE: u8 = 1;
const BUSY: u8 = 2;

/// The budget's semaphore and its size in permits. Replaced as a whole when
/// the configured budget changes.
struct Budget {
    semaphore: Arc<Semaphore>,
    units: usize,
}

/// Shared state of the body-limit middleware for one server instance.
#[derive(Clone)]
pub struct BodyLimits {
    config: Arc<lr_config::ConfigManager>,
    budget: Arc<Mutex<Budget>>,
    /// Threshold, budget and wait replacing the configured ones (tests).
    overridden: Option<lr_config::LargeBodyLimits>,
    /// A route-specific limit replacing the configured one.
    fixed: Option<usize>,
}

impl BodyLimits {
    pub fn new(config: Arc<lr_config::ConfigManager>) -> Self {
        let budget = config.large_body_limits().budget;
        Self::build(config, budget, None)
    }

    #[cfg(test)]
    pub(crate) fn with_budget(
        config: Arc<lr_config::ConfigManager>,
        budget: usize,
        threshold: usize,
        wait: Duration,
    ) -> Self {
        Self::build(
            config,
            budget,
            Some(lr_config::LargeBodyLimits {
                threshold,
                budget,
                wait,
            }),
        )
    }

    fn build(
        config: Arc<lr_config::ConfigManager>,
        budget: usize,
        overridden: Option<lr_config::LargeBodyLimits>,
    ) -> Self {
        let units = budget_units(budget);
        Self {
            config,
            budget: Arc::new(Mutex::new(Budget {
                semaphore: Arc::new(Semaphore::new(units)),
                units,
            })),
            overridden,
            fixed: None,
        }
    }

    /// The threshold, budget and wait for the next request.
    fn settings(&self) -> lr_config::LargeBodyLimits {
        self.overridden
            .unwrap_or_else(|| self.config.large_body_limits())
    }

    /// The semaphore of a budget of `bytes`, replacing the current one if the
    /// configured size changed. Permits held from a replaced semaphore go
    /// back to it and are dropped with it.
    fn budget_for(&self, bytes: usize) -> (Arc<Semaphore>, usize) {
        let units = budget_units(bytes);
        let mut budget = self.budget.lock();
        if budget.units != units {
            *budget = Budget {
                semaphore: Arc::new(Semaphore::new(units)),
                units,
            };
        }
        (budget.semaphore.clone(), budget.units)
    }

    /// The same budget with a fixed limit, for upload routes whose limit
    /// does not follow the JSON request limit (audio, image edits).
    pub fn with_fixed_limit(&self, limit: usize) -> Self {
        Self {
            fixed: Some(limit),
            ..self.clone()
        }
    }

    fn limit_for(&self, path: &str) -> usize {
        if let Some(limit) = self.fixed {
            limit
        } else if path == "/v1/files" {
            FILE_UPLOAD_LIMIT
        } else {
            self.config.request_body_limit()
        }
    }

    #[cfg(test)]
    pub(crate) fn available_bytes(&self) -> usize {
        self.budget.lock().semaphore.available_permits() * UNIT
    }
}

/// Permits for a budget of `bytes` (at least one).
fn budget_units(bytes: usize) -> usize {
    bytes.div_ceil(UNIT).clamp(1, Semaphore::MAX_PERMITS)
}

/// Permits for `bytes`, capped at the whole budget so one request larger than
/// the budget can still run alone instead of waiting forever.
fn units_for(bytes: usize, budget_units: usize) -> usize {
    bytes.div_ceil(UNIT).min(budget_units)
}

pub(crate) fn too_large(limit: usize) -> ApiErrorResponse {
    let size = if limit.is_multiple_of(MIB) {
        format!("{} MiB", limit / MIB)
    } else {
        format!("{limit} bytes")
    };
    ApiErrorResponse::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        "invalid_request_error",
        format!("Request body exceeds the {size} limit"),
    )
    .with_code("request_too_large")
}

fn busy() -> ApiErrorResponse {
    let mut error = ApiErrorResponse::service_unavailable(
        "LocalRouter is holding other large requests in memory; retry shortly",
    )
    .with_code("request_body_budget_exhausted");
    error.retry_after_secs = Some(BUDGET_RETRY_AFTER_SECS);
    error
}

pub async fn middleware(
    State(limits): State<BodyLimits>,
    request: Request,
    next: Next,
) -> Response {
    let limit = limits.limit_for(request.uri().path());
    let settings = limits.settings();
    let (budget, budget_units) = limits.budget_for(settings.budget);
    let declared = request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if declared.is_some_and(|length| length > limit as u64) {
        return too_large(limit).into_response();
    }
    let holder: Arc<Mutex<Option<OwnedSemaphorePermit>>> = Arc::new(Mutex::new(None));
    let mut reserved = 0;
    if let Some(length) = declared.map(|length| length as usize) {
        if length > settings.threshold {
            let units = units_for(length, budget_units);
            let acquire = budget.clone().acquire_many_owned(units as u32);
            match tokio::time::timeout(settings.wait, acquire).await {
                Ok(Ok(permit)) => {
                    *holder.lock() = Some(permit);
                    reserved = units;
                }
                _ => return busy().into_response(),
            }
        }
    }
    let outcome = Arc::new(AtomicU8::new(ACCEPTED));
    let (parts, body) = request.into_parts();
    let guarded = GuardedBody {
        inner: body,
        seen: 0,
        limit,
        threshold: settings.threshold,
        reserved,
        budget_units,
        semaphore: PollSemaphore::new(budget),
        holder: holder.clone(),
        pending: None,
        deadline: None,
        wait: settings.wait,
        outcome: outcome.clone(),
    };
    let response = next
        .run(Request::from_parts(parts, Body::new(guarded)))
        .await;
    match outcome.load(Ordering::Acquire) {
        TOO_LARGE => return too_large(limit).into_response(),
        BUSY => return busy().into_response(),
        _ => {}
    }
    if holder.lock().is_none() {
        return response;
    }
    let (parts, body) = response.into_parts();
    Response::from_parts(
        parts,
        Body::new(HoldReservation {
            inner: body,
            _holder: holder,
        }),
    )
}

#[derive(Debug)]
enum Rejected {
    TooLarge,
    Busy,
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "request body exceeds the configured limit",
            Self::Busy => "request body budget exhausted",
        })
    }
}

impl std::error::Error for Rejected {}

/// Request body that enforces the limit on received bytes and reserves budget
/// before handing on a chunk that takes the body past the threshold. While a
/// reservation is pending the chunk is held and the connection is not read,
/// so the client is back-pressured.
struct GuardedBody {
    inner: Body,
    seen: usize,
    limit: usize,
    threshold: usize,
    reserved: usize,
    budget_units: usize,
    semaphore: PollSemaphore,
    holder: Arc<Mutex<Option<OwnedSemaphorePermit>>>,
    pending: Option<Frame<Bytes>>,
    deadline: Option<Pin<Box<tokio::time::Sleep>>>,
    wait: Duration,
    outcome: Arc<AtomicU8>,
}

impl GuardedBody {
    fn reject(&mut self, reason: Rejected) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        self.pending = None;
        let code = match reason {
            Rejected::TooLarge => TOO_LARGE,
            Rejected::Busy => BUSY,
        };
        self.outcome.store(code, Ordering::Release);
        Poll::Ready(Some(Err(axum::Error::new(reason))))
    }
}

impl http_body::Body for GuardedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        loop {
            if this.pending.is_some() {
                let wanted = units_for(this.seen, this.budget_units);
                if wanted > this.reserved {
                    let step = wanted - this.reserved;
                    match this.semaphore.poll_acquire_many(cx, step as u32) {
                        Poll::Ready(Some(permit)) => {
                            this.deadline = None;
                            this.reserved = wanted;
                            let mut holder = this.holder.lock();
                            match holder.as_mut() {
                                Some(held) => held.merge(permit),
                                None => *holder = Some(permit),
                            }
                        }
                        Poll::Ready(None) => return this.reject(Rejected::Busy),
                        Poll::Pending => {
                            let wait = this.wait;
                            let deadline = this
                                .deadline
                                .get_or_insert_with(|| Box::pin(tokio::time::sleep(wait)));
                            if deadline.as_mut().poll(cx).is_ready() {
                                return this.reject(Rejected::Busy);
                            }
                            return Poll::Pending;
                        }
                    }
                }
                return Poll::Ready(this.pending.take().map(Ok));
            }
            match Pin::new(&mut this.inner).poll_frame(cx) {
                Poll::Ready(Some(Ok(frame))) => {
                    if let Some(data) = frame.data_ref() {
                        this.seen = this.seen.saturating_add(data.len());
                        if this.seen > this.limit {
                            return this.reject(Rejected::TooLarge);
                        }
                    }
                    if this.seen > this.threshold {
                        this.pending = Some(frame);
                        continue;
                    }
                    return Poll::Ready(Some(Ok(frame)));
                }
                other => return other,
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.pending.is_none() && self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        if self.pending.is_some() {
            SizeHint::default()
        } else {
            self.inner.size_hint()
        }
    }
}

/// Response body that keeps the request's budget reservation until the
/// response is finished or dropped.
struct HoldReservation {
    inner: Body,
    _holder: Arc<Mutex<Option<OwnedSemaphorePermit>>>,
}

impl http_body::Body for HoldReservation {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.get_mut().inner).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
#[path = "body_limits_tests.rs"]
mod tests;
