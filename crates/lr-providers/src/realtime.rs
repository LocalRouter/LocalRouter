//! Native realtime origins and server-side credentials. No redirects or reconnects.
use lr_types::{AppError, AppResult};
use reqwest::header::HeaderMap;
use sha2::{Digest, Sha256};
#[derive(Clone)]
pub struct RealtimeTransport {
    base_url: reqwest::Url,
    headers: HeaderMap,
}
impl RealtimeTransport {
    pub fn new(base: &str, headers: HeaderMap) -> AppResult<Self> {
        let url = reqwest::Url::parse(base)
            .map_err(|_| AppError::Config("Invalid realtime provider origin".into()))?;
        if !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(AppError::Config(
                "Realtime origin requires HTTP(S) without credentials, query or fragment".into(),
            ));
        }
        if url.scheme() == "http"
            && !url.host_str().is_some_and(|host| {
                host == "localhost"
                    || host
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            })
        {
            return Err(AppError::Config(
                "Remote realtime providers require TLS".into(),
            ));
        }
        Ok(Self {
            base_url: url,
            headers,
        })
    }
    pub fn connection(&self, model: &str) -> AppResult<(String, HeaderMap)> {
        if model.is_empty()
            || model.len() > 512
            || model.chars().any(char::is_whitespace)
            || matches!(model, "auto" | "localrouter/auto")
        {
            return Err(AppError::InvalidParams(
                "Realtime requires an explicit native model".into(),
            ));
        }
        let mut url = self.base_url.clone();
        let path = format!("{}/realtime", url.path().trim_end_matches('/'));
        url.set_path(&path);
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme)
            .map_err(|_| AppError::Config("Invalid realtime WebSocket scheme".into()))?;
        url.query_pairs_mut().append_pair("model", model);
        Ok((url.into(), self.headers.clone()))
    }
    pub fn binding(&self, salt: &str) -> String {
        let mut hash = Sha256::new();
        hash.update(salt);
        hash.update([0]);
        hash.update(self.base_url.as_str());
        let mut headers: Vec<_> = self.headers.iter().collect();
        headers.sort_by(|a, b| {
            a.0.as_str()
                .cmp(b.0.as_str())
                .then_with(|| a.1.as_bytes().cmp(b.1.as_bytes()))
        });
        for (name, value) in headers {
            hash.update([0]);
            hash.update(name.as_str());
            hash.update([0]);
            hash.update(value.as_bytes());
        }
        format!("{:x}", hash.finalize())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn realtime_urls_encode_pinned_models_and_never_accept_remote_plaintext_or_embedded_credentials(
    ) {
        let transport =
            RealtimeTransport::new("https://api.example.com/v1", HeaderMap::new()).unwrap();
        let (url, _) = transport
            .connection("openai/gpt-realtime?api_key=not-a-parameter")
            .unwrap();
        let url = reqwest::Url::parse(&url).unwrap();
        assert_eq!(url.scheme(), "wss");
        assert_eq!(url.path(), "/v1/realtime");
        assert_eq!(url.query_pairs().count(), 1);
        assert_eq!(url.query_pairs().next().unwrap().0, "model");
        for base in [
            "http://remote.example/v1",
            "https://key:password@api.example.com/v1",
            "https://api.example.com/v1?token=x",
            "https://api.example.com/v1#fragment",
        ] {
            assert!(RealtimeTransport::new(base, HeaderMap::new()).is_err());
        }
        assert!(RealtimeTransport::new("http://127.0.0.1:1234/v1", HeaderMap::new()).is_ok());
    }
    #[test]
    fn origin_and_credential_changes_invalidate_native_realtime_bindings() {
        let a = RealtimeTransport::new("https://api.example.com/v1", HeaderMap::new()).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer fixture".parse().unwrap());
        let b = RealtimeTransport::new("https://api.example.com/v1", headers).unwrap();
        assert_ne!(a.binding("salt"), b.binding("salt"));
        assert_ne!(a.binding("salt"), a.binding("new-salt"));
    }
}
