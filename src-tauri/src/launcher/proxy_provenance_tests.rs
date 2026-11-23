//! Authenticated trace provenance through the real AppFirewall and MITM
//! proxy: a request that loops between the native server and the proxy is
//! limited, prompted and counted once, while a forged, unsigned, stale or
//! replayed trace header stays a fresh, limited request.
use super::*;
use lr_types::{RequestTrace, TRACE_HEADER};

/// A trace header as the native server's provider HTTP client stamps it for
/// the first hop of `trace_id`.
fn signed(trace_id: &str) -> String {
    lr_types::install_trace_signing_key([5; 32]);
    let header = RequestTrace::parse(&format!("{trace_id};hop=1"))
        .unwrap()
        .signed_header_value();
    assert!(header.contains(";sig="), "a signing key is installed");
    header
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_looping_native_to_proxy_is_limited_and_counted_once() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), Some(1.0));
    let upstream = Upstream::new();
    upstream.open();
    let (port, roots) = serve_tls(upstream.clone()).await;
    let client = mitm_client(root.path(), &firewall, &request.client_id, roots).await;
    let url = format!("https://localhost:{port}/v1/chat/completions");
    let send = |header: String| {
        client
            .post(&url)
            .header(TRACE_HEADER, header)
            .json(&chat_body())
            .send()
    };

    // The native hop holds the request's only share, as the gateway does
    // while it calls the provider.
    let native = firewall
        .state
        .router
        .admit_direct_request(&request.client_id)
        .await
        .unwrap();
    // Its provider call reaches the proxy carrying the signed header: the
    // proxy recognizes it and neither refuses nor reserves.
    let header = signed("native-loop");
    assert_eq!(send(header.clone()).await.unwrap().status().as_u16(), 200);
    assert_eq!(upstream.arrived(), 1);
    assert_eq!(in_flight(&firewall), 1);

    // Replayed, stale, unsigned and forged headers are fresh requests,
    // refused by the share the native hop holds.
    let stale = RequestTrace::parse("native-loop;hop=1")
        .unwrap()
        .signed_header_value_at(unix_now() - lr_types::TRACE_SIGNATURE_MAX_AGE_SECS - 30)
        .unwrap();
    let mut bad_signature = header.clone();
    let last = bad_signature.pop().unwrap();
    bad_signature.push(if last == '0' { '1' } else { '0' });
    let tampered = header.replace("hop=1", "hop=2");
    for refused in [
        header.clone(),
        stale,
        tampered,
        bad_signature,
        "native-loop;hop=1".to_string(),
        "forged;hop=9".to_string(),
    ] {
        let status = send(refused.clone()).await.unwrap().status().as_u16();
        assert_eq!(status, 429, "{refused}");
    }
    assert_eq!(upstream.arrived(), 1);
    drop(native);

    // A fresh request now passes and is the strategy's only metrics row:
    // the duplicate hop (and the refusals) wrote none.
    assert_eq!(
        send("fresh;hop=1".into()).await.unwrap().status().as_u16(),
        200
    );
    until("the fresh request is recorded", || recorded(&firewall) == 1).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(recorded(&firewall), 1);
    until("every share is released", || in_flight(&firewall) == 0).await;
}

#[tokio::test]
async fn a_request_looping_proxy_to_native_is_admitted_once() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), Some(1.0));
    let router = firewall.state.router.clone();
    signed("install-key");

    // The proxy admits the request and stamps the copy it forwards.
    let mut headers = hyper::HeaderMap::new();
    let mut proxied = copy(&request, None);
    proxied.trace = lr_proxy::stamp_trace(&mut headers, true);
    assert!(!rejected(&firewall.evaluate(&proxied).await, 429));
    assert_eq!(in_flight(&firewall), 1);

    // The native server receives the forwarded copy: its trace verifies, so
    // it runs as the next (duplicate) hop and holds nothing.
    let forwarded = headers
        .get(TRACE_HEADER)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let native_trace = RequestTrace::from_inbound_header(Some(&forwarded));
    assert!(native_trace.is_duplicate());
    let looped = lr_types::with_outbound_trace(
        Some(native_trace),
        router.admit_direct_request(&request.client_id),
    )
    .await
    .expect("the duplicate hop is admitted");
    assert_eq!(in_flight(&firewall), 1);

    // The same header delivered again is a replay: a fresh, limited request.
    let replay = RequestTrace::from_inbound_header(Some(&forwarded));
    assert!(!replay.is_duplicate());
    assert!(lr_types::with_outbound_trace(
        Some(replay),
        router.admit_direct_request(&request.client_id)
    )
    .await
    .is_err());
    drop(looped);
    drop(proxied);
    assert_eq!(in_flight(&firewall), 0);
}

/// A verified duplicate hop that still reaches the firewall (its client
/// protects secrets) is not asked again: the model approval popup of the
/// first hop is not repeated. A fresh request is asked.
#[tokio::test]
async fn a_verified_duplicate_hop_is_not_prompted_again() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), None);
    firewall
        .config_manager
        .update(|c| {
            c.secret_scanning.action = lr_config::SecretScanAction::Redact;
            c.strategies[0].auto_config.as_mut().unwrap().permission =
                lr_config::PermissionState::Ask;
        })
        .unwrap();
    let mut duplicate = copy(&request, None);
    duplicate.trace = Some(RequestTrace::from_inbound_header(Some(&signed(
        "asked-once",
    ))));
    assert!(duplicate.trace.as_ref().unwrap().is_duplicate());
    let answered = tokio::time::timeout(Duration::from_secs(5), firewall.evaluate(&duplicate))
        .await
        .expect("no approval popup for a duplicate hop");
    assert!(
        !matches!(answered, RequestAction::Reject { .. }),
        "the duplicate hop is forwarded"
    );

    // A fresh request with the same policy waits for (or fails) the popup.
    let fresh = copy(&request, Some("asked-once;hop=1"));
    match tokio::time::timeout(Duration::from_millis(500), firewall.evaluate(&fresh)).await {
        Err(_) => {} // waiting for the user's answer
        Ok(action) => assert!(
            rejected(&action, 403),
            "without a popup window the ask fails closed"
        ),
    }
}
