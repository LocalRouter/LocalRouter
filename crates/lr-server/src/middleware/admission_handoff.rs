//! One [`lr_router::AdmissionHandoff`] per HTTP request.
//!
//! The router hands a finished call's strategy reservation to the request's
//! handoff instead of releasing it, because the strategy check reads the
//! metrics rows this server writes after the call (or after a stream ends).
//! The middleware runs the handler with a fresh handoff in scope and moves
//! the handoff into the response body: it is released when the body has
//! been sent or dropped, after every finalize step that ran while producing
//! it. Body polls run with the handoff in scope too, so a stream that ends
//! during a poll hands its reservation to the same request.

use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::{Body, Bytes};
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use http_body::{Frame, SizeHint};
use lr_router::AdmissionHandoff;

pub async fn middleware(req: Request, next: Next) -> Response {
    let handoff = AdmissionHandoff::new();
    let response = handoff.clone().scope(next.run(req)).await;
    hold_until_body_ends(response, handoff)
}

/// Keep `handoff` alive (and current during polls) until `response`'s body
/// is finished or dropped.
pub fn hold_until_body_ends(response: Response, handoff: AdmissionHandoff) -> Response {
    let (parts, body) = response.into_parts();
    Response::from_parts(
        parts,
        Body::new(HandoffBody {
            inner: body,
            handoff,
        }),
    )
}

struct HandoffBody {
    inner: Body,
    handoff: AdmissionHandoff,
}

impl http_body::Body for HandoffBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = &mut *self;
        let inner = &mut this.inner;
        this.handoff.sync_scope(|| Pin::new(inner).poll_frame(cx))
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use tower::ServiceExt;

    fn reserved(manager: &lr_router::RateLimiterManager) -> lr_router::AdmissionReservation {
        manager
            .reserve_strategy(
                "s",
                lr_router::rate_limit::ReservedUsage {
                    requests: 1,
                    ..Default::default()
                },
                |_| Ok::<(), ()>(()),
            )
            .expect("admitted")
    }

    #[tokio::test]
    async fn reservations_handed_off_in_a_handler_live_until_the_body_is_dropped() {
        let manager = std::sync::Arc::new(lr_router::RateLimiterManager::new(None));
        let in_handler = manager.clone();
        let app = axum::Router::new()
            .route(
                "/",
                get(move || {
                    let manager = in_handler.clone();
                    async move {
                        lr_router::hand_off(reserved(&manager), None);
                        "ok"
                    }
                }),
            )
            .layer(axum::middleware::from_fn(middleware));

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        // The handler has returned; the response body still holds it.
        assert_eq!(manager.strategy_in_flight("s").requests, 1);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(&body[..], b"ok");
        assert_eq!(manager.strategy_in_flight("s").requests, 0);
    }

    #[tokio::test]
    async fn a_stream_ending_during_a_body_poll_hands_off_to_the_same_request() {
        let manager = std::sync::Arc::new(lr_router::RateLimiterManager::new(None));
        let in_stream = manager.clone();
        let app = axum::Router::new()
            .route(
                "/",
                get(move || {
                    let manager = in_stream.clone();
                    async move {
                        let stream = futures::stream::once(async move {
                            // Runs while the body is polled, after the
                            // handler returned.
                            lr_router::hand_off(reserved(&manager), None);
                            Ok::<_, std::io::Error>(Bytes::from_static(b"chunk"))
                        });
                        Body::from_stream(stream)
                    }
                }),
            )
            .layer(axum::middleware::from_fn(middleware));

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let mut body = response.into_body().into_data_stream();
        let chunk = futures::StreamExt::next(&mut body).await.unwrap().unwrap();
        assert_eq!(chunk, Bytes::from_static(b"chunk"));
        // Handed to the request's handoff, which the body still holds.
        assert_eq!(manager.strategy_in_flight("s").requests, 1);
        drop(body);
        assert_eq!(manager.strategy_in_flight("s").requests, 0);
    }

    #[tokio::test]
    async fn without_a_handoff_a_reservation_is_released_at_once() {
        let manager = lr_router::RateLimiterManager::new(None);
        lr_router::hand_off(reserved(&manager), None);
        assert_eq!(manager.strategy_in_flight("s").requests, 0);
    }
}
