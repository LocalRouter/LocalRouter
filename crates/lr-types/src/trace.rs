//! Cross-hop request trace.
//!
//! Every LocalRouter component that forwards an LLM request (the gateway, the
//! HTTPS inspection proxy, the reverse proxy) stamps the outgoing request with
//! [`TRACE_HEADER`] for correlation. A plain header is caller-controlled and
//! does not prove that another router enforced policy: an inbound wire trace
//! keeps its correlation ID but starts a fresh enforcement/accounting hop.
//!
//! LocalRouter signs the headers it stamps with a per-install key (HMAC-SHA256
//! over the trace fields, a timestamp and a nonce; see
//! [`install_trace_signing_key`]). An inbound header whose signature verifies,
//! is fresh ([`TRACE_SIGNATURE_MAX_AGE_SECS`]) and whose nonce was never seen
//! before was stamped by an earlier hop of this install: the request runs as
//! that hop's duplicate ([`RequestTrace::from_inbound_header`]), so it is not
//! enforced, prompted or counted twice.
//!
//! Wire format: `X-LocalRouter-Trace: <trace_id>;hop=<n>`, signed:
//! `<trace_id>;hop=<n>;ts=<unix secs>;nonce=<32 hex>;sig=<64 hex>`.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

/// Header name (lowercase, as HTTP header names are case-insensitive).
pub const TRACE_HEADER: &str = "x-localrouter-trace";

/// Identity of one logical request as it travels through LocalRouter hops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestTrace {
    /// Stable id shared by every hop of the same logical request.
    pub trace_id: String,
    /// 1 for the first LocalRouter hop, incremented by each forwarding hop.
    pub hop: u32,
}

impl RequestTrace {
    /// A fresh trace for a request LocalRouter is seeing for the first time.
    pub fn new() -> Self {
        Self {
            trace_id: uuid::Uuid::new_v4().to_string(),
            hop: 1,
        }
    }

    /// Parse a header value. Returns `None` for anything malformed so a bogus
    /// header is simply treated as absent.
    pub fn parse(header: &str) -> Option<Self> {
        let mut parts = header.split(';').map(str::trim);
        let trace_id = parts.next().filter(|s| !s.is_empty())?;
        if !trace_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            || trace_id.len() > 128
        {
            return None;
        }
        let mut hop = None;
        for part in parts {
            if let Some(v) = part.strip_prefix("hop=") {
                hop = Some(v.parse::<u32>().ok()?);
            }
        }
        Some(Self {
            trace_id: trace_id.to_string(),
            hop: hop.filter(|h| *h >= 1)?,
        })
    }

    /// The trace to stamp on the request when forwarding it onward.
    pub fn next_hop(&self) -> Self {
        Self {
            trace_id: self.trace_id.clone(),
            hop: self.hop.saturating_add(1),
        }
    }

    /// Serialized header value.
    pub fn header_value(&self) -> String {
        format!("{};hop={}", self.trace_id, self.hop)
    }

    /// Whether an earlier LocalRouter hop already handled this request.
    pub fn is_duplicate(&self) -> bool {
        self.hop > 1
    }

    /// Build the trace for an untrusted inbound wire request. Preserve its
    /// correlation ID, but never let its claimed hop bypass enforcement or
    /// accounting. `next_hop` is only appropriate for a trusted handoff (an
    /// in-process one, or a header [`Self::from_inbound_header`] verified).
    pub fn outbound_for(inbound: Option<&RequestTrace>) -> Self {
        inbound
            .map(|trace| Self {
                trace_id: trace.trace_id.clone(),
                hop: 1,
            })
            .unwrap_or_default()
    }
}

impl Default for RequestTrace {
    fn default() -> Self {
        Self::new()
    }
}

// ---- signed traces ----------------------------------------------------------

/// Oldest signed trace header accepted, in seconds. Both hops of a looped
/// request run in this process (or on this machine): the next hop reads the
/// header milliseconds after it was stamped.
pub const TRACE_SIGNATURE_MAX_AGE_SECS: u64 = 60;
/// How far in the future a signed timestamp may be (clock steps).
const MAX_FUTURE_SKEW_SECS: u64 = 5;
/// Nonces remembered at once; a full cache refuses to verify (fail safe).
const REPLAY_CACHE_CAPACITY: usize = 65_536;
/// Domain separation for the MAC input.
const MAC_CONTEXT: &str = "lr-trace-v1";

/// This install's trace signing key (see [`install_trace_signing_key`]).
static SIGNING_KEY: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();

/// Install this install's trace signing key, once per process: the first
/// call wins and later calls return `false`. Until a key is installed,
/// LocalRouter stamps unsigned headers and trusts no inbound header. The key
/// is a local secret: only MACs computed with it ever leave the process.
pub fn install_trace_signing_key(key: [u8; 32]) -> bool {
    SIGNING_KEY.set(key).is_ok()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn trace_mac(key: &[u8; 32], trace: &RequestTrace, ts: u64, nonce: &str) -> Hmac<Sha256> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(
        format!(
            "{MAC_CONTEXT}\n{}\n{}\n{ts}\n{nonce}",
            trace.trace_id, trace.hop
        )
        .as_bytes(),
    );
    mac
}

/// The signature parts of a wire header.
struct WireSignature {
    ts: u64,
    nonce: String,
    sig: Vec<u8>,
}

impl WireSignature {
    fn parse(header: &str) -> Option<Self> {
        let (mut ts, mut nonce, mut sig) = (None, None, None);
        for part in header.split(';').map(str::trim) {
            if let Some(v) = part.strip_prefix("ts=") {
                ts = Some(v.parse::<u64>().ok()?);
            } else if let Some(v) = part.strip_prefix("nonce=") {
                if v.len() != 32 || !v.chars().all(|c| c.is_ascii_hexdigit()) {
                    return None;
                }
                nonce = Some(v.to_ascii_lowercase());
            } else if let Some(v) = part.strip_prefix("sig=") {
                sig = Some(hex::decode(v).ok().filter(|s| s.len() == 32)?);
            }
        }
        Some(Self {
            ts: ts?,
            nonce: nonce?,
            sig: sig?,
        })
    }
}

/// Why an inbound trace header is (not) trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Provenance {
    /// Signed by this install, fresh, first use: an earlier hop of ours.
    Verified,
    /// No complete signature (an external caller, or signing disabled).
    Unsigned,
    /// No signing key installed: nothing can be verified.
    NoKey,
    /// The MAC does not match (forged, tampered, or another install's key).
    BadSignature,
    /// Outside the freshness window.
    Stale,
    /// A nonce seen before (a replayed header), or the replay cache is full.
    Replayed,
}

/// Nonces of verified headers, kept for the freshness window.
#[derive(Default)]
pub(crate) struct ReplayCache {
    /// Nonce → the timestamp it was signed at.
    seen: parking_lot::Mutex<std::collections::HashMap<String, u64>>,
    /// When expired nonces were last swept (at most once a second).
    swept_at: std::sync::atomic::AtomicU64,
}

impl ReplayCache {
    /// Remember `nonce` (signed at `ts`); `false` if it was seen before or
    /// there is no room for it.
    fn first_use(&self, nonce: &str, ts: u64, now: u64) -> bool {
        use std::sync::atomic::Ordering;
        let mut seen = self.seen.lock();
        if seen.contains_key(nonce) {
            return false;
        }
        if seen.len() >= REPLAY_CACHE_CAPACITY / 2
            && self.swept_at.swap(now, Ordering::Relaxed) != now
        {
            // Expired nonces can no longer verify (stale): forget them.
            seen.retain(|_, signed| signed.saturating_add(TRACE_SIGNATURE_MAX_AGE_SECS) >= now);
        }
        if seen.len() >= REPLAY_CACHE_CAPACITY {
            return false;
        }
        seen.insert(nonce.to_string(), ts);
        true
    }
}

static REPLAY_CACHE: std::sync::LazyLock<ReplayCache> =
    std::sync::LazyLock::new(ReplayCache::default);

impl RequestTrace {
    /// Header value for an outgoing request: signed with this install's key
    /// (`;ts=..;nonce=..;sig=..` appended) when one is installed, so the
    /// next LocalRouter hop can trust it; the plain value otherwise.
    pub fn signed_header_value(&self) -> String {
        match SIGNING_KEY.get() {
            Some(key) => self.signed_with(key, unix_now()),
            None => self.header_value(),
        }
    }

    /// [`Self::signed_header_value`] signed as of `ts` (unix seconds), or
    /// `None` without an installed key. For tests of the freshness window.
    #[doc(hidden)]
    pub fn signed_header_value_at(&self, ts: u64) -> Option<String> {
        SIGNING_KEY.get().map(|key| self.signed_with(key, ts))
    }

    fn signed_with(&self, key: &[u8; 32], ts: u64) -> String {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let sig = hex::encode(trace_mac(key, self, ts, &nonce).finalize().into_bytes());
        format!("{};ts={ts};nonce={nonce};sig={sig}", self.header_value())
    }

    /// The trace to run an inbound request under, from its trace header.
    ///
    /// A header this install signed (fresh, never seen before) proves an
    /// earlier LocalRouter hop already handled the request: the result is
    /// its next hop, a duplicate. Anything else (absent, malformed, unsigned,
    /// forged, stale, replayed) keeps only its correlation ID and starts a
    /// fresh enforcement/accounting hop ([`Self::outbound_for`]).
    pub fn from_inbound_header(header: Option<&str>) -> Self {
        let Some((header, inbound)) =
            header.and_then(|h| Self::parse(h).map(|inbound| (h, inbound)))
        else {
            return Self::new();
        };
        match verify_inbound(
            &inbound,
            header,
            SIGNING_KEY.get(),
            unix_now(),
            &REPLAY_CACHE,
        ) {
            Provenance::Verified => inbound.next_hop(),
            _ => Self::outbound_for(Some(&inbound)),
        }
    }
}

/// Check the signature of `header` (whose trace fields parsed as `inbound`).
pub(crate) fn verify_inbound(
    inbound: &RequestTrace,
    header: &str,
    key: Option<&[u8; 32]>,
    now: u64,
    replay: &ReplayCache,
) -> Provenance {
    let Some(signature) = WireSignature::parse(header) else {
        return Provenance::Unsigned;
    };
    let Some(key) = key else {
        return Provenance::NoKey;
    };
    if trace_mac(key, inbound, signature.ts, &signature.nonce)
        .verify_slice(&signature.sig)
        .is_err()
    {
        return Provenance::BadSignature;
    }
    if signature.ts.saturating_add(TRACE_SIGNATURE_MAX_AGE_SECS) < now
        || signature.ts > now.saturating_add(MAX_FUTURE_SKEW_SECS)
    {
        return Provenance::Stale;
    }
    if !replay.first_use(&signature.nonce, signature.ts, now) {
        return Provenance::Replayed;
    }
    Provenance::Verified
}

tokio::task_local! {
    /// The trace of the request the current task is handling, as it will be
    /// stamped on any upstream request this task sends. Set once per inbound
    /// request by the server / proxy entry points; read by the HTTP client
    /// middleware (to add the header) and by the accounting layers (to skip
    /// counting a duplicate hop). `None` means dedupe is disabled.
    pub static OUTBOUND_TRACE: Option<RequestTrace>;
}

/// Run `fut` with `trace` as the current request trace.
pub async fn with_outbound_trace<F: std::future::Future>(
    trace: Option<RequestTrace>,
    fut: F,
) -> F::Output {
    OUTBOUND_TRACE.scope(trace, fut).await
}

/// The current request trace, if one is in scope.
pub fn current_outbound_trace() -> Option<RequestTrace> {
    OUTBOUND_TRACE.try_with(|t| t.clone()).ok().flatten()
}

/// Whether the request the current task is handling was already handled by
/// an earlier LocalRouter hop — in which case it must be passed through
/// unmodified and must not be counted in stats again.
pub fn is_duplicate_hop() -> bool {
    current_outbound_trace().is_some_and(|t| t.is_duplicate())
}

/// `tokio::spawn` that carries the current request trace into the new task
/// (task-locals are not inherited by spawned tasks otherwise).
pub fn spawn_traced<F>(fut: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(crate::secret_context::scope(
        crate::secret_context::current(),
        crate::upstream::with_probe(
            crate::upstream::current_probe(),
            OUTBOUND_TRACE.scope(current_outbound_trace(), fut),
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let t = RequestTrace::new();
        assert_eq!(t.hop, 1);
        assert!(!t.is_duplicate());
        let parsed = RequestTrace::parse(&t.header_value()).unwrap();
        assert_eq!(parsed, t);
    }

    #[test]
    fn next_hop_keeps_id() {
        let t = RequestTrace::parse("abc-123;hop=1").unwrap();
        let n = t.next_hop();
        assert_eq!(n.trace_id, "abc-123");
        assert_eq!(n.hop, 2);
        assert!(n.is_duplicate());
        assert_eq!(n.header_value(), "abc-123;hop=2");
    }

    #[test]
    fn parse_tolerates_whitespace() {
        let t = RequestTrace::parse(" abc ; hop=3 ").unwrap();
        assert_eq!(t.hop, 3);
    }

    #[test]
    fn malformed_is_none() {
        assert!(RequestTrace::parse("").is_none());
        assert!(RequestTrace::parse("abc").is_none());
        assert!(RequestTrace::parse("abc;hop=0").is_none());
        assert!(RequestTrace::parse("abc;hop=x").is_none());
        assert!(RequestTrace::parse(";hop=1").is_none());
        assert!(RequestTrace::parse("a b;hop=1").is_none());
        assert!(RequestTrace::parse("é;hop=1").is_none());
    }

    #[tokio::test]
    async fn task_local_scope_and_spawn() {
        assert!(current_outbound_trace().is_none());
        assert!(!is_duplicate_hop());
        let t = RequestTrace::parse("abc;hop=2").unwrap();
        with_outbound_trace(Some(t.clone()), async {
            assert_eq!(current_outbound_trace(), Some(t.clone()));
            assert!(is_duplicate_hop());
            // Plain spawn loses the scope; spawn_traced keeps it.
            assert!(!tokio::spawn(async { is_duplicate_hop() }).await.unwrap());
            assert!(spawn_traced(async { is_duplicate_hop() }).await.unwrap());
        })
        .await;
        assert!(!is_duplicate_hop());
    }

    #[test]
    fn outbound_for_inbound() {
        assert_eq!(RequestTrace::outbound_for(None).hop, 1);
        let inbound = RequestTrace::parse("x;hop=2").unwrap();
        let out = RequestTrace::outbound_for(Some(&inbound));
        assert_eq!((out.trace_id.as_str(), out.hop), ("x", 1));
        assert!(!out.is_duplicate());
    }

    #[tokio::test]
    async fn forged_wire_hop_never_grants_duplicate_privileges() {
        for claimed_hop in [1, 2, 99, u32::MAX] {
            let inbound = RequestTrace::parse(&format!("external;hop={claimed_hop}")).unwrap();
            with_outbound_trace(Some(RequestTrace::outbound_for(Some(&inbound))), async {
                assert!(!is_duplicate_hop());
                assert!(!spawn_traced(async { is_duplicate_hop() }).await.unwrap());
            })
            .await;
        }
    }

    // ---- signed traces --------------------------------------------------------

    const KEY: [u8; 32] = [7; 32];
    const OTHER_KEY: [u8; 32] = [9; 32];
    const NOW: u64 = 1_800_000_000;

    fn verify(header: &str, key: Option<&[u8; 32]>, now: u64, cache: &ReplayCache) -> Provenance {
        let inbound = RequestTrace::parse(header).expect("trace fields parse");
        verify_inbound(&inbound, header, key, now, cache)
    }

    #[test]
    fn signed_header_keeps_the_plain_fields_readable() {
        let t = RequestTrace::parse("abc;hop=1").unwrap();
        let signed = t.signed_with(&KEY, NOW);
        assert!(signed.starts_with("abc;hop=1;ts=1800000000;nonce="));
        assert_eq!(RequestTrace::parse(&signed), Some(t));
    }

    #[test]
    fn a_genuine_signature_verifies_once() {
        let cache = ReplayCache::default();
        let signed = RequestTrace::parse("abc;hop=1")
            .unwrap()
            .signed_with(&KEY, NOW);
        assert_eq!(
            verify(&signed, Some(&KEY), NOW + 2, &cache),
            Provenance::Verified
        );
        // The same header again is a replay.
        assert_eq!(
            verify(&signed, Some(&KEY), NOW + 3, &cache),
            Provenance::Replayed
        );
        // Changing the case of the nonce does not make it a new one.
        let (head, rest) = signed.split_once("nonce=").unwrap();
        let (nonce, tail) = rest.split_at(32);
        let recased = format!("{head}nonce={}{tail}", nonce.to_ascii_uppercase());
        assert_eq!(
            verify(&recased, Some(&KEY), NOW + 3, &cache),
            Provenance::Replayed
        );
    }

    #[test]
    fn forged_tampered_and_foreign_signatures_are_rejected() {
        let cache = ReplayCache::default();
        let signed = RequestTrace::parse("abc;hop=1")
            .unwrap()
            .signed_with(&KEY, NOW);
        // Another install's key.
        assert_eq!(
            verify(&signed, Some(&OTHER_KEY), NOW, &cache),
            Provenance::BadSignature
        );
        // A claimed hop, id or timestamp the signature does not cover.
        for tampered in [
            signed.replace("hop=1", "hop=2"),
            signed.replace("abc;", "abd;"),
            signed.replace("ts=1800000000", "ts=1800000001"),
        ] {
            assert_eq!(
                verify(&tampered, Some(&KEY), NOW, &cache),
                Provenance::BadSignature
            );
        }
        // A made-up signature.
        let forged = format!(
            "abc;hop=1;ts={NOW};nonce={};sig={}",
            "0".repeat(32),
            "ab".repeat(32)
        );
        assert_eq!(
            verify(&forged, Some(&KEY), NOW, &cache),
            Provenance::BadSignature
        );
        // Malformed or partial signatures are unsigned headers.
        for unsigned in [
            "abc;hop=1".to_string(),
            format!("abc;hop=1;ts={NOW}"),
            format!("abc;hop=1;ts={NOW};nonce={};sig=zz", "0".repeat(32)),
            format!("abc;hop=1;ts={NOW};nonce=short;sig={}", "ab".repeat(32)),
        ] {
            assert_eq!(
                verify(&unsigned, Some(&KEY), NOW, &cache),
                Provenance::Unsigned
            );
        }
        // Without a key nothing verifies.
        assert_eq!(verify(&signed, None, NOW, &cache), Provenance::NoKey);
    }

    #[test]
    fn stale_and_future_signatures_are_rejected() {
        let cache = ReplayCache::default();
        let t = RequestTrace::parse("abc;hop=1").unwrap();
        let at = |ts| t.signed_with(&KEY, ts);
        assert_eq!(
            verify(
                &at(NOW - TRACE_SIGNATURE_MAX_AGE_SECS - 1),
                Some(&KEY),
                NOW,
                &cache
            ),
            Provenance::Stale
        );
        assert_eq!(
            verify(&at(NOW + MAX_FUTURE_SKEW_SECS + 1), Some(&KEY), NOW, &cache),
            Provenance::Stale
        );
        assert_eq!(
            verify(
                &at(NOW - TRACE_SIGNATURE_MAX_AGE_SECS),
                Some(&KEY),
                NOW,
                &cache
            ),
            Provenance::Verified
        );
    }

    #[test]
    fn a_full_replay_cache_refuses_and_expired_nonces_are_forgotten() {
        let cache = ReplayCache::default();
        for i in 0..REPLAY_CACHE_CAPACITY {
            assert!(cache.first_use(&format!("{i:032x}"), NOW, NOW));
        }
        // Full of live nonces: fail safe.
        assert!(!cache.first_use("new", NOW, NOW));
        // Once they have expired they make room.
        assert!(cache.first_use("new", NOW, NOW + TRACE_SIGNATURE_MAX_AGE_SECS + 1));
        assert_eq!(cache.seen.lock().len(), 1);
    }

    /// Through the installed key, as the server and proxy use it.
    #[tokio::test]
    async fn only_a_verified_inbound_header_runs_as_a_duplicate_hop() {
        install_trace_signing_key(KEY);
        let first = RequestTrace::parse("loop;hop=1").unwrap();
        let signed = first.signed_header_value();
        let second = RequestTrace::from_inbound_header(Some(&signed));
        assert_eq!((second.trace_id.as_str(), second.hop), ("loop", 2));
        with_outbound_trace(Some(second), async {
            assert!(is_duplicate_hop());
        })
        .await;
        // Replayed, stale, forged, unsigned, malformed and absent headers
        // start a fresh hop.
        let stale = first
            .signed_header_value_at(unix_now() - TRACE_SIGNATURE_MAX_AGE_SECS - 10)
            .unwrap();
        for header in [
            Some(signed.as_str()),
            Some(stale.as_str()),
            Some("loop;hop=7"),
            Some("garbage"),
            None,
        ] {
            let t = RequestTrace::from_inbound_header(header);
            assert_eq!(t.hop, 1, "{header:?}");
            assert!(!t.is_duplicate());
        }
        assert_eq!(
            RequestTrace::from_inbound_header(Some("loop;hop=7")).trace_id,
            "loop"
        );
    }
}
