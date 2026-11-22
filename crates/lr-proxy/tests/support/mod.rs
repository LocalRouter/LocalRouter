//! Shared fixtures for the end-to-end suites: synthetic conversations of any
//! size, generated chunk by chunk so that a 300 MiB request never exists in
//! memory or on disk, and a request body with an exact length for them.

#![allow(dead_code)]

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body::{Body, Frame, SizeHint};

/// The request shape a synthetic conversation is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Api {
    /// OpenAI Responses, with the top-level keys in the order Codex's
    /// `ResponsesApiRequest` declares them.
    Responses,
    /// Anthropic Messages.
    Messages,
}

/// A deterministic conversation of roughly `target_bytes`: alternating user
/// and assistant turns of ordinary words, with JSON escapes and multi-byte
/// characters in every turn. Nothing in it looks like a credential.
#[derive(Clone, Debug)]
pub struct SyntheticConversation {
    pub api: Api,
    /// Turns are added until the body is at least this long.
    pub target_bytes: u64,
    pub model: &'static str,
    /// Write `model` as the last top-level key instead of the first.
    pub model_last: bool,
    /// Text placed verbatim at the end of the turn with this index.
    pub marker: Option<(usize, String)>,
}

impl SyntheticConversation {
    pub fn responses(target_bytes: u64) -> Self {
        Self {
            api: Api::Responses,
            target_bytes,
            model: "synthetic-model",
            model_last: false,
            marker: None,
        }
    }

    pub fn messages(target_bytes: u64) -> Self {
        Self {
            api: Api::Messages,
            ..Self::responses(target_bytes)
        }
    }

    pub fn model_last(mut self) -> Self {
        self.model_last = true;
        self
    }

    pub fn with_marker(mut self, turn: usize, text: &str) -> Self {
        self.marker = Some((turn, text.to_string()));
        self
    }

    /// The body as a sequence of chunks, one per turn. Their concatenation
    /// is one valid JSON document.
    pub fn chunks(&self) -> Chunks {
        Chunks {
            conversation: self.clone(),
            stage: Stage::Prefix,
            turn: 0,
            written: 0,
        }
    }

    /// Exact length of the body in bytes.
    pub fn byte_len(&self) -> u64 {
        self.chunks().map(|chunk| chunk.len() as u64).sum()
    }

    /// Number of turns the body holds.
    pub fn turns(&self) -> usize {
        // The prefix and the suffix are the two chunks that are not turns.
        self.chunks().count() - 2
    }

    /// The whole body in memory. Only for small conversations.
    pub fn to_vec(&self) -> Vec<u8> {
        self.chunks().flat_map(|chunk| chunk.to_vec()).collect()
    }

    /// The body as a request body of known length, so a client sends it with
    /// `Content-Length`.
    pub fn body(&self) -> SizedBody<Chunks> {
        SizedBody::new(self.chunks(), self.byte_len())
    }

    fn prefix(&self) -> String {
        let model = if self.model_last {
            String::new()
        } else {
            format!("\"model\":\"{}\",", self.model)
        };
        match self.api {
            Api::Responses => format!(
                "{{{model}\"instructions\":\"You are a synthetic test fixture.\",\"input\":["
            ),
            Api::Messages => format!("{{{model}\"max_tokens\":1024,\"messages\":["),
        }
    }

    fn suffix(&self) -> String {
        let model = if self.model_last {
            format!(",\"model\":\"{}\"", self.model)
        } else {
            String::new()
        };
        match self.api {
            Api::Responses => format!(
                "],\"tools\":[],\"tool_choice\":\"auto\",\"parallel_tool_calls\":false,\
                 \"reasoning\":null,\"store\":false,\"stream\":true,\"include\":[]{model}}}"
            ),
            Api::Messages => format!(
                "],\"system\":\"You are a synthetic test fixture.\",\"stream\":true{model}}}"
            ),
        }
    }

    fn turn(&self, index: usize) -> String {
        let user = index.is_multiple_of(2);
        let mut text = turn_text(index);
        if let Some((turn, marker)) = &self.marker {
            if *turn == index {
                text.push(' ');
                text.push_str(marker);
            }
        }
        let text = serde_json::to_string(&text).expect("a string serializes");
        let separator = if index == 0 { "" } else { "," };
        match (self.api, user) {
            (Api::Responses, true) => format!(
                "{separator}{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":{text}}}]}}"
            ),
            (Api::Responses, false) => format!(
                "{separator}{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":{text}}}]}}"
            ),
            (Api::Messages, true) => {
                format!("{separator}{{\"role\":\"user\",\"content\":{text}}}")
            }
            (Api::Messages, false) => {
                format!("{separator}{{\"role\":\"assistant\",\"content\":{text}}}")
            }
        }
    }
}

const WORDS: &[&str] = &[
    "the",
    "quick",
    "review",
    "of",
    "module",
    "layout",
    "shows",
    "that",
    "each",
    "handler",
    "returns",
    "early",
    "when",
    "input",
    "is",
    "empty",
    "and",
    "otherwise",
    "walks",
    "list",
    "once",
    "before",
    "writing",
    "result",
    "back",
    "to",
    "caller",
    "without",
    "copying",
    "it",
    "twice",
    "which",
    "keeps",
    "memory",
    "flat",
    "for",
    "long",
    "sessions",
    "with",
    "many",
    "turns",
    "please",
    "check",
    "tests",
    "again",
    "after",
    "rename",
    "then",
    "report",
    "what",
];

/// About 4 KiB of ordinary prose, different for every turn.
fn turn_text(index: usize) -> String {
    let mut state = (index as u64)
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let mut text = format!("Turn {index}: she said \"continue\"\nthen — naïve café, 日本語, 😀.\t");
    while text.len() < 4096 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        text.push_str(WORDS[(state >> 33) as usize % WORDS.len()]);
        text.push(' ');
    }
    text
}

#[derive(Clone, Copy, Debug)]
enum Stage {
    Prefix,
    Turns,
    Done,
}

/// Iterator over the chunks of a [`SyntheticConversation`].
#[derive(Clone, Debug)]
pub struct Chunks {
    conversation: SyntheticConversation,
    stage: Stage,
    turn: usize,
    written: u64,
}

impl Iterator for Chunks {
    type Item = Bytes;

    fn next(&mut self) -> Option<Bytes> {
        let chunk = match self.stage {
            Stage::Prefix => {
                self.stage = Stage::Turns;
                self.conversation.prefix()
            }
            Stage::Turns if self.written < self.conversation.target_bytes => {
                let turn = self.conversation.turn(self.turn);
                self.turn += 1;
                turn
            }
            Stage::Turns => {
                self.stage = Stage::Done;
                self.conversation.suffix()
            }
            Stage::Done => return None,
        };
        self.written += chunk.len() as u64;
        Some(Bytes::from(chunk))
    }
}

/// `total` bytes as cheap chunks of at most `chunk` bytes, for uploads whose
/// content does not matter.
pub fn filler(total: u64, chunk: usize) -> impl Iterator<Item = Bytes> + Send + Unpin + 'static {
    let block = Bytes::from(vec![b'u'; chunk]);
    let mut remaining = total;
    std::iter::from_fn(move || {
        if remaining == 0 {
            return None;
        }
        let take = remaining.min(block.len() as u64) as usize;
        remaining -= take as u64;
        Some(block.slice(..take))
    })
}

/// A request body produced chunk by chunk whose total length is known up
/// front, so hyper sends `Content-Length` instead of chunking it.
pub struct SizedBody<I> {
    chunks: I,
    remaining: u64,
}

impl<I> SizedBody<I> {
    pub fn new(chunks: I, len: u64) -> Self {
        Self {
            chunks,
            remaining: len,
        }
    }
}

impl<I: Iterator<Item = Bytes> + Unpin> Body for SizedBody<I> {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        let Some(chunk) = self.chunks.next() else {
            assert_eq!(self.remaining, 0, "body ended short of its declared length");
            return Poll::Ready(None);
        };
        self.remaining = self
            .remaining
            .checked_sub(chunk.len() as u64)
            .expect("body exceeds its declared length");
        Poll::Ready(Some(Ok(Frame::data(chunk))))
    }

    fn is_end_stream(&self) -> bool {
        self.remaining == 0
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.remaining)
    }
}

// ---------------------------------------------------------------------------
// TLS upstreams and raw HTTP helpers
// ---------------------------------------------------------------------------

use std::sync::Arc;

use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
use rustls::{RootCertStore, ServerConfig};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// A throwaway CA that signs the test upstream's certificate.
pub struct TestCa {
    issuer: rcgen::Issuer<'static, KeyPair>,
    pub ca_der: CertificateDer<'static>,
}

pub fn make_ca() -> TestCa {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(Vec::new()).unwrap();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "Test Upstream CA");
    params.distinguished_name = dn;
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let cert = params.self_signed(&key).unwrap();
    let ca_der = cert.der().clone();
    let issuer = rcgen::Issuer::from_ca_cert_pem(&cert.pem(), key).unwrap();
    TestCa { issuer, ca_der }
}

fn make_leaf(ca: &TestCa, host: &str) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let key = KeyPair::generate().unwrap();
    let params = CertificateParams::new(vec![host.to_string()]).unwrap();
    let cert = params.signed_by(&key, &ca.issuer).unwrap();
    let der = cert.der().clone();
    let pk = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
    (der, pk)
}

/// A TLS listener for `host` on an OS-assigned port, speaking HTTP/1.1.
pub async fn tls_listener(ca: &TestCa, host: &str) -> (TcpListener, TlsAcceptor, u16) {
    // Tests build this listener before the proxy, and rustls cannot pick a
    // provider on its own when feature unification enables more than one.
    lr_proxy::tls::ensure_crypto_provider();
    let (leaf, key) = make_leaf(ca, host);
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![leaf], key)
        .unwrap();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, TlsAcceptor::from(Arc::new(config)), port)
}

/// Root store holding the certificates of a PEM bundle.
pub fn root_store(pem: &str) -> RootCertStore {
    let mut roots = RootCertStore::empty();
    let mut reader = std::io::BufReader::new(pem.as_bytes());
    for cert in rustls_pemfile::certs(&mut reader) {
        roots.add(cert.unwrap()).unwrap();
    }
    roots
}

/// Read an HTTP head from a raw stream. Returns the head lowercased, and
/// whatever body bytes arrived in the same reads.
pub async fn read_head<S: AsyncRead + Unpin>(stream: &mut S) -> (String, Vec<u8>) {
    let mut seen = Vec::new();
    let mut buffer = [0u8; 16 * 1024];
    loop {
        if let Some(end) = seen.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = seen.split_off(end + 4);
            return (String::from_utf8_lossy(&seen).to_ascii_lowercase(), rest);
        }
        let n = stream.read(&mut buffer).await.unwrap();
        assert!(n > 0, "connection closed before the end of the HTTP head");
        seen.extend_from_slice(&buffer[..n]);
    }
}
