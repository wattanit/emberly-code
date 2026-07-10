//! How a provider client authenticates its HTTP requests (P-8, Tech Spec
//! §4.5). The scheme is data so any endpoint speaking a wire format an adapter
//! already parses is reachable by configuration alone.
//!
//! `Auth` never reads a secret itself: the key value is resolved from config
//! (env / `keys.toml`) before it is handed here, so no secret-loading logic
//! lives in the provider layer.

/// An authentication scheme plus the resolved key it carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    /// No authentication header — e.g. a local Ollama/vLLM server.
    None,
    /// `Authorization: Bearer <key>`. An empty key sends no header (the local
    /// server path), matching the pre-profile OpenAI behavior.
    Bearer(String),
    /// `x-api-key: <key>` — the Anthropic Messages default.
    XApiKey(String),
    /// A custom header carrying the key verbatim, e.g. name `api-key`.
    Header { name: String, value: String },
}

impl Auth {
    /// Attach this scheme's header (if any) to an outgoing request.
    pub(crate) fn apply(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Auth::None => builder,
            Auth::Bearer(key) if key.is_empty() => builder,
            Auth::Bearer(key) => builder.bearer_auth(key),
            Auth::XApiKey(key) => builder.header("x-api-key", key),
            Auth::Header { name, value } => builder.header(name.as_str(), value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a request with `auth` applied and return it for header inspection.
    /// (No `unwrap`/`expect` — the crate denies them; `match` + `panic!`.)
    fn apply(auth: &Auth) -> reqwest::Request {
        // Building a reqwest client configures rustls, which needs the
        // process-default crypto provider installed (as the live tests do).
        use std::sync::Once;
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let _ = rustls_rustcrypto::provider().install_default();
        });
        let builder = reqwest::Client::new().post("http://localhost/");
        match auth.apply(builder).build() {
            Ok(req) => req,
            Err(e) => panic!("failed to build request: {e}"),
        }
    }

    /// A header's value as a string, or `None` if absent/non-ASCII.
    fn header(req: &reqwest::Request, name: &str) -> Option<String> {
        req.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    #[test]
    fn none_sets_no_auth_headers() {
        let req = apply(&Auth::None);
        assert_eq!(header(&req, "authorization"), None);
        assert_eq!(header(&req, "x-api-key"), None);
    }

    #[test]
    fn bearer_sets_authorization() {
        let req = apply(&Auth::Bearer("sk-123".into()));
        assert_eq!(header(&req, "authorization").as_deref(), Some("Bearer sk-123"));
    }

    #[test]
    fn empty_bearer_sets_no_header() {
        // The local-server path: an empty key must not send an auth header.
        let req = apply(&Auth::Bearer(String::new()));
        assert_eq!(header(&req, "authorization"), None);
    }

    #[test]
    fn x_api_key_sets_header_verbatim() {
        let req = apply(&Auth::XApiKey("key-abc".into()));
        assert_eq!(header(&req, "x-api-key").as_deref(), Some("key-abc"));
        // x-api-key is sent even when empty (Anthropic default is unconditional).
        let empty = apply(&Auth::XApiKey(String::new()));
        assert_eq!(header(&empty, "x-api-key").as_deref(), Some(""));
    }

    #[test]
    fn custom_header_carries_the_key() {
        let req = apply(&Auth::Header {
            name: "api-key".into(),
            value: "v1".into(),
        });
        assert_eq!(header(&req, "api-key").as_deref(), Some("v1"));
    }
}
