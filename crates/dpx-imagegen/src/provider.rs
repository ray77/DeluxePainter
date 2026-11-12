//! The provider interface (ADR-45): what a picture generator offers, the request it takes,
//! the typed errors it gives and the token that cancels it.
//!
//! A provider is a real generator behind a network API or a local program. The editor never
//! pretends to generate: without a ready provider the GUI and MCP say so
//! ([`ProviderStatus::NotConfigured`]).

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use dpx_core::RgbaImage;
use dpx_core::resample;

use crate::size::SizeRule;

/// What to generate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationRequest {
    /// The picture described in words.
    pub prompt: String,
    /// The size to ask the provider for (one its [`Capabilities::sizes`] allow); the
    /// result is converted to the page afterwards (`dpx_core::import::convert`).
    pub size: (u32, u32),
    /// Pictures to make at once (variants), at least 1.
    pub variants: u8,
    /// Ask for a transparent background.
    pub transparent_background: bool,
    /// A fixed seed, for providers that take one.
    pub seed: Option<u64>,
}

impl GenerationRequest {
    /// One opaque picture of `size`, no seed.
    pub fn new(prompt: impl Into<String>, size: (u32, u32)) -> Self {
        Self {
            prompt: prompt.into(),
            size,
            variants: 1,
            transparent_background: false,
            seed: None,
        }
    }

    /// One picture for a `page` shown with `pixel_shape` (`DisplayMode::pixel_shape`): the
    /// provider's size nearest to the page's display proportions ([`SizeRule::choose`]), so
    /// the conversion has as little as possible to cut off.
    pub fn for_page(
        prompt: impl Into<String>,
        caps: &Capabilities,
        page: (u32, u32),
        pixel_shape: (u32, u32),
    ) -> Self {
        let aspect = resample::display_aspect(page, pixel_shape);
        Self::new(prompt, caps.sizes.choose(aspect))
    }
}

/// What a provider can do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// The picture sizes it makes.
    pub sizes: SizeRule,
    /// It can leave the background transparent.
    pub transparent_background: bool,
    /// Most pictures per request (1 = no variants).
    pub max_variants: u8,
    /// It takes a seed.
    pub seed: bool,
    /// Longest prompt in characters.
    pub max_prompt_chars: usize,
}

impl Capabilities {
    /// Checks a request against these capabilities before anything is sent.
    pub fn check(&self, r: &GenerationRequest) -> Result<(), GenError> {
        let invalid = |m: String| Err(GenError::InvalidRequest(m));
        let chars = r.prompt.chars().count();
        if r.prompt.trim().is_empty() {
            return invalid("the description is empty".into());
        }
        if chars > self.max_prompt_chars {
            return invalid(format!(
                "the description has {chars} characters, at most {}",
                self.max_prompt_chars
            ));
        }
        if !self.sizes.allows(r.size) {
            return invalid(format!(
                "{}x{} is not a size this generator makes",
                r.size.0, r.size.1
            ));
        }
        if r.variants == 0 || r.variants > self.max_variants {
            return invalid(format!(
                "{} pictures at once: this generator makes 1 to {}",
                r.variants, self.max_variants
            ));
        }
        if r.transparent_background && !self.transparent_background {
            return invalid("this generator cannot make transparent backgrounds".into());
        }
        if r.seed.is_some() && !self.seed {
            return invalid("this generator takes no seed".into());
        }
        Ok(())
    }
}

/// Whether a provider can be used now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderStatus {
    Ready,
    /// Not set up; the text says what is missing (e.g. no API key).
    NotConfigured(String),
}

/// A picture generator (ADR-45). Implementations are called on worker threads
/// ([`crate::JobManager`]), never on the GUI thread.
pub trait ImageGenerationProvider: Send + Sync {
    /// Stable identifier, e.g. `openai` (settings, MCP).
    fn id(&self) -> &str;
    /// Name shown to the user, e.g. `OpenAI Images`.
    fn display_name(&self) -> &str;
    /// Whether it sends the prompt over the network.
    fn needs_network(&self) -> bool;
    /// What using it costs and where the prompt goes, honestly and briefly (ASCII, shown
    /// before generating).
    fn cost_note(&self) -> &str;
    fn capabilities(&self) -> Capabilities;
    fn status(&self) -> ProviderStatus;
    /// Generates `request.variants` pictures. Blocks; returns [`GenError::Cancelled`] when
    /// `cancel` was set before the result arrived (a request already sent may still be
    /// billed by a cloud provider).
    fn generate(
        &self,
        request: &GenerationRequest,
        cancel: &CancelToken,
    ) -> Result<Vec<RgbaImage>, GenError>;
}

/// Cancels a running generation; cheap to clone, shared with the worker.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// `Err(Cancelled)` once cancelled.
    pub fn check(&self) -> Result<(), GenError> {
        if self.is_cancelled() {
            Err(GenError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Why a generation failed. Messages from a service are cleaned ([`clean_message`]) before
/// they get here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenError {
    /// The provider is not set up (no API key, no model).
    NotConfigured(String),
    /// The request was refused before or by the service as malformed (size, prompt …).
    InvalidRequest(String),
    /// The service refused the key (HTTP 401) or the account (403, e.g. an organisation
    /// that must be verified first).
    Auth { status: u16, message: String },
    /// Too many requests or the account's quota is used up (HTTP 429).
    RateLimited {
        quota_exhausted: bool,
        retry_after: Option<Duration>,
        message: String,
    },
    /// The prompt or the picture was blocked by the service's content policy.
    ContentPolicy(String),
    /// No connection (DNS, connect, TLS, a connection that broke off).
    Network(String),
    /// No complete answer in time.
    Timeout,
    /// Cancelled by the user (or MCP).
    Cancelled,
    /// The service answered something that is no picture.
    BadResponse(String),
    /// The service failed (HTTP 5xx).
    Server { status: u16, message: String },
    /// Too many requests are in flight already (cancelled ones count until they end).
    Busy { running: usize },
}

impl GenError {
    /// Stable code for MCP answers.
    pub fn code(&self) -> &'static str {
        match self {
            GenError::NotConfigured(_) => "NOT_CONFIGURED",
            GenError::InvalidRequest(_) => "INVALID_REQUEST",
            GenError::Auth { .. } => "AUTH_FAILED",
            GenError::RateLimited {
                quota_exhausted: true,
                ..
            } => "QUOTA_EXCEEDED",
            GenError::RateLimited { .. } => "RATE_LIMITED",
            GenError::ContentPolicy(_) => "CONTENT_POLICY",
            GenError::Network(_) => "NETWORK",
            GenError::Timeout => "TIMEOUT",
            GenError::Cancelled => "CANCELLED",
            GenError::BadResponse(_) => "BAD_RESPONSE",
            GenError::Server { .. } => "SERVER_ERROR",
            GenError::Busy { .. } => "BUSY",
        }
    }

    /// Whether trying again later may work without changing anything.
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            GenError::RateLimited {
                quota_exhausted: false,
                ..
            } | GenError::Network(_)
                | GenError::Timeout
                | GenError::Server { .. }
                | GenError::Busy { .. }
        )
    }
}

impl fmt::Display for GenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GenError::NotConfigured(m) => write!(f, "Image generator not set up: {m}"),
            GenError::InvalidRequest(m) => write!(f, "Request refused: {m}"),
            GenError::Auth { status: 401, .. } => {
                write!(f, "The API key was refused (check or replace it)")
            }
            GenError::Auth { status, message } => {
                write!(f, "Access refused (HTTP {status}): {message}")
            }
            GenError::RateLimited {
                quota_exhausted: true,
                message,
                ..
            } => write!(f, "The account's quota or credit is used up: {message}"),
            GenError::RateLimited {
                retry_after: Some(d),
                ..
            } => write!(
                f,
                "Too many requests; try again in {} s",
                d.as_secs().max(1)
            ),
            GenError::RateLimited { .. } => write!(f, "Too many requests; try again later"),
            GenError::ContentPolicy(m) => {
                write!(f, "Blocked by the service's content policy: {m}")
            }
            GenError::Network(m) => write!(f, "No connection to the service: {m}"),
            GenError::Timeout => write!(f, "The service did not answer in time"),
            GenError::Cancelled => write!(f, "Cancelled"),
            GenError::BadResponse(m) => write!(f, "Unexpected answer from the service: {m}"),
            GenError::Server { status, message } => {
                write!(f, "The service failed (HTTP {status}): {message}")
            }
            GenError::Busy { running } => write!(
                f,
                "{running} requests are still waiting for the image generator (cancelled \
                 ones too, until it answers); try again when one has ended"
            ),
        }
    }
}

impl std::error::Error for GenError {}

/// Longest message kept from a service.
pub const MAX_MESSAGE_CHARS: usize = 200;

/// A service's message made safe to show and log: printable ASCII only (the pixel UI's
/// font), whitespace collapsed, anything that looks like an API key (`sk-…`) replaced, at
/// most [`MAX_MESSAGE_CHARS`] characters.
pub fn clean_message(raw: &str) -> String {
    let ascii: String = raw
        .chars()
        .map(|c| if c.is_ascii_graphic() { c } else { ' ' })
        .collect();
    let words: Vec<String> = ascii
        .split_whitespace()
        .map(|w| {
            if w.contains("sk-") {
                "[key]".to_string()
            } else {
                w.to_string()
            }
        })
        .collect();
    let mut out = words.join(" ");
    if out.len() > MAX_MESSAGE_CHARS {
        out.truncate(MAX_MESSAGE_CHARS - 3);
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::size::CustomSizes;

    fn caps() -> Capabilities {
        Capabilities {
            sizes: SizeRule::Custom(CustomSizes::OPENAI),
            transparent_background: true,
            max_variants: 4,
            seed: false,
            max_prompt_chars: 10,
        }
    }

    #[test]
    fn requests_are_checked_against_the_capabilities() {
        let c = caps();
        let ok = GenerationRequest::new("castle", (1024, 768));
        c.check(&ok).unwrap();
        let bad = |r: GenerationRequest| match c.check(&r) {
            Err(GenError::InvalidRequest(m)) => m,
            other => panic!("{other:?}"),
        };
        assert!(bad(GenerationRequest::new("  ", (1024, 768))).contains("empty"));
        assert!(bad(GenerationRequest::new("a castle by the sea", (1024, 768))).contains("19"));
        assert!(bad(GenerationRequest::new("castle", (1000, 768))).contains("1000x768"));
        let many = GenerationRequest {
            variants: 5,
            ..ok.clone()
        };
        assert!(bad(many).contains("1 to 4"));
        let none = GenerationRequest {
            variants: 0,
            ..ok.clone()
        };
        bad(none);
        let seeded = GenerationRequest {
            seed: Some(1),
            ..ok.clone()
        };
        assert!(bad(seeded).contains("seed"));
        let opaque_only = Capabilities {
            transparent_background: false,
            ..c.clone()
        };
        let clear = GenerationRequest {
            transparent_background: true,
            ..ok
        };
        assert!(opaque_only.check(&clear).is_err());
    }

    #[test]
    fn for_page_picks_the_display_proportions() {
        let c = caps();
        assert_eq!(
            GenerationRequest::for_page("x", &c, (320, 240), (1, 1)).size,
            (1024, 768)
        );
        // HiRes pixels are twice as tall: 640x256 shows as 5:4.
        assert_eq!(
            GenerationRequest::for_page("x", &c, (640, 256), (1, 2)).size,
            (1040, 832)
        );
    }

    #[test]
    fn cancel_token_is_shared() {
        let t = CancelToken::new();
        let u = t.clone();
        assert!(t.check().is_ok());
        u.cancel();
        assert_eq!(t.check(), Err(GenError::Cancelled));
    }

    #[test]
    fn codes_retries_and_messages() {
        let quota = GenError::RateLimited {
            quota_exhausted: true,
            retry_after: None,
            message: "x".into(),
        };
        assert_eq!(quota.code(), "QUOTA_EXCEEDED");
        assert!(!quota.retryable());
        let rate = GenError::RateLimited {
            quota_exhausted: false,
            retry_after: Some(Duration::from_secs(20)),
            message: "x".into(),
        };
        assert_eq!(rate.code(), "RATE_LIMITED");
        assert!(rate.retryable());
        assert_eq!(rate.to_string(), "Too many requests; try again in 20 s");
        assert!(GenError::Timeout.retryable());
        assert!(!GenError::ContentPolicy("x".into()).retryable());
        let auth = GenError::Auth {
            status: 401,
            message: "Incorrect API key".into(),
        };
        assert_eq!(auth.code(), "AUTH_FAILED");
        assert!(auth.to_string().contains("API key was refused"));
        // Every message is ASCII (the pixel UI's font).
        for e in [quota, rate, auth, GenError::Busy { running: 2 }] {
            assert!(e.to_string().is_ascii());
        }
    }

    #[test]
    fn messages_are_cleaned() {
        assert_eq!(
            clean_message("Incorrect API key provided: sk-abc***xyz.\n You can find…"),
            "Incorrect API key provided: [key] You can find"
        );
        let long = "word ".repeat(100);
        let c = clean_message(&long);
        assert_eq!(c.len(), MAX_MESSAGE_CHARS);
        assert!(c.ends_with("..."));
        assert_eq!(clean_message("Größe\tfalsch"), "Gr e falsch");
    }
}
