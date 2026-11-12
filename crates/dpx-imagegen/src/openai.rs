//! OpenAI Images (ADR-45): `POST https://api.openai.com/v1/images/generations` with a GPT
//! image model.
//!
//! - Request: `Authorization: Bearer <key>`, JSON `model`, `prompt`, `size` ("WxH"),
//!   `quality`, `background` (`transparent` or `opaque`), `output_format` `png`, `n`.
//! - Answer: `data[].b64_json`, PNG pictures decoded with `dpx_formats::read_png`.
//! - Errors: OpenAI's `{"error": {"message", "type", "code"}}` mapped to [`GenError`]
//!   (401/403 auth, 429 rate limit or `insufficient_quota`, `moderation_blocked` content
//!   policy, 5xx server).
//! - The model id is a setting ([`OpenAiConfig::model`]); the default
//!   [`DEFAULT_MODEL`] was a current, non-deprecated model when [`MODELS_CHECKED`].
//! - Blocking HTTPS (ureq, native TLS with the system's root certificates) on a worker
//!   thread, with a connect timeout and an overall timeout. Cancelling cannot stop a request
//!   already sent: its answer is thrown away, and OpenAI may still bill it (unverified).

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use dpx_core::RgbaImage;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::provider::{
    CancelToken, Capabilities, GenError, GenerationRequest, ImageGenerationProvider,
    ProviderStatus, clean_message,
};
use crate::secret::ApiKeys;
use crate::size::{CustomSizes, SizeRule};

/// The provider's id (settings, MCP).
pub const ID: &str = "openai";
/// The default model: `gpt-image-2.6-flare`, OpenAI's "fast iteration" GPT image model and
/// one of the two replacements its deprecations page names for every image model shut down
/// in 2026.
pub const DEFAULT_MODEL: &str = "gpt-image-2.6-flare";
/// When the model list and the deprecations page were read (ADR-45).
pub const MODELS_CHECKED: &str = "2026-11-12";
/// The endpoint.
pub const ENDPOINT: &str = "https://api.openai.com/v1/images/generations";
/// The environment variable read as a fallback key source.
pub const ENV_KEY: &str = "OPENAI_API_KEY";
/// The keychain account of the key.
pub const KEY_ACCOUNT: &str = "openai";
/// Most pictures per request (`n`).
pub const MAX_VARIANTS: u8 = 10;
/// Longest prompt.
pub const MAX_PROMPT_CHARS: usize = 32_000;
/// Largest answer read (base64 PNGs of up to 10 pictures).
const MAX_RESPONSE_BYTES: u64 = 256 * 1024 * 1024;

/// What the GUI shows before generating (ASCII, the pixel UI's font).
pub const COST_NOTE: &str = "Sends your description over the internet to OpenAI, which \
    generates the picture. OpenAI bills every picture to your own API account (no free \
    tier; the price depends on model, quality and size, see OpenAI's pricing page). A \
    cancelled request may still be billed.";

/// Picture quality (and price): OpenAI's `quality`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    /// Cheapest and fastest; enough for quick drafts.
    Low,
    #[default]
    Medium,
    High,
    /// OpenAI decides.
    Auto,
}

impl Quality {
    pub fn as_str(self) -> &'static str {
        match self {
            Quality::Low => "low",
            Quality::Medium => "medium",
            Quality::High => "high",
            Quality::Auto => "auto",
        }
    }
}

/// The provider's settings (no key: keys live in [`ApiKeys`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenAiConfig {
    /// Model id, e.g. [`DEFAULT_MODEL`]. Models are shut down often, so it is a setting.
    pub model: String,
    pub quality: Quality,
    /// Whole request, from connecting to the last byte (OpenAI: complex prompts "may take
    /// up to 2 minutes").
    pub timeout_secs: u64,
    pub connect_timeout_secs: u64,
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_string(),
            quality: Quality::Medium,
            timeout_secs: 180,
            connect_timeout_secs: 20,
        }
    }
}

impl OpenAiConfig {
    fn check_model(&self) -> Result<(), GenError> {
        let m = &self.model;
        if m.is_empty() || m.len() > 64 || !m.chars().all(|c| c.is_ascii_graphic()) {
            return Err(GenError::NotConfigured(format!(
                "'{}' is no OpenAI model id",
                clean_message(m)
            )));
        }
        Ok(())
    }
}

/// OpenAI Images behind [`ImageGenerationProvider`].
pub struct OpenAiProvider {
    config: OpenAiConfig,
    keys: Arc<ApiKeys>,
    endpoint: String,
    /// Timeouts (from the config, shorter in tests).
    timeout: Duration,
    connect_timeout: Duration,
    /// Use the HTTP(S)_PROXY environment (not in tests against a local server).
    env_proxy: bool,
}

impl OpenAiProvider {
    pub fn new(config: OpenAiConfig, keys: Arc<ApiKeys>) -> Self {
        let timeout = Duration::from_secs(config.timeout_secs.clamp(5, 900));
        let connect_timeout = Duration::from_secs(config.connect_timeout_secs.clamp(1, 120));
        Self {
            config,
            keys,
            endpoint: ENDPOINT.to_string(),
            timeout,
            connect_timeout,
            env_proxy: true,
        }
    }

    pub fn config(&self) -> &OpenAiConfig {
        &self.config
    }

    /// A local test server instead of OpenAI (tests only).
    #[cfg(test)]
    fn with_test_server(mut self, url: &str, timeout: Duration) -> Self {
        self.endpoint = url.to_string();
        self.timeout = timeout;
        self.connect_timeout = timeout;
        self.env_proxy = false;
        self
    }

    fn agent(&self) -> ureq::Agent {
        use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
        let tls = TlsConfig::builder()
            .provider(TlsProvider::NativeTls)
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        let mut config = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .timeout_connect(Some(self.connect_timeout))
            .http_status_as_error(false)
            .tls_config(tls)
            .user_agent("DeluxePainter");
        if !self.env_proxy {
            config = config.proxy(None);
        }
        config.build().into()
    }
}

impl ImageGenerationProvider for OpenAiProvider {
    fn id(&self) -> &str {
        ID
    }

    fn display_name(&self) -> &str {
        "OpenAI Images"
    }

    fn needs_network(&self) -> bool {
        true
    }

    fn cost_note(&self) -> &str {
        COST_NOTE
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            sizes: SizeRule::Custom(CustomSizes::OPENAI),
            transparent_background: true,
            max_variants: MAX_VARIANTS,
            seed: false,
            max_prompt_chars: MAX_PROMPT_CHARS,
        }
    }

    fn status(&self) -> ProviderStatus {
        if let Err(GenError::NotConfigured(m)) = self.config.check_model() {
            return ProviderStatus::NotConfigured(m);
        }
        match self.keys.key(KEY_ACCOUNT, ENV_KEY) {
            Some(_) => ProviderStatus::Ready,
            None => ProviderStatus::NotConfigured("no OpenAI API key".into()),
        }
    }

    fn generate(
        &self,
        request: &GenerationRequest,
        cancel: &CancelToken,
    ) -> Result<Vec<RgbaImage>, GenError> {
        self.capabilities().check(request)?;
        self.config.check_model()?;
        let (key, _) = self
            .keys
            .key(KEY_ACCOUNT, ENV_KEY)
            .ok_or_else(|| GenError::NotConfigured("no OpenAI API key".into()))?;
        cancel.check()?;
        let body = request_body(&self.config, request).to_string();
        let sent = self
            .agent()
            .post(&self.endpoint)
            .header("Authorization", &format!("Bearer {}", key.expose()))
            .header("Content-Type", "application/json")
            .send(body.as_bytes());
        cancel.check()?;
        let mut response = sent.map_err(transport_error)?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_vec()
            .map_err(transport_error)?;
        cancel.check()?;
        let images = parse_response(status, retry_after.as_deref(), &bytes)?;
        cancel.check()?;
        Ok(images)
    }
}

/// The JSON body for `request`.
pub(crate) fn request_body(config: &OpenAiConfig, request: &GenerationRequest) -> Value {
    json!({
        "model": config.model,
        "prompt": request.prompt,
        "size": format!("{}x{}", request.size.0, request.size.1),
        "quality": config.quality.as_str(),
        "background": if request.transparent_background { "transparent" } else { "opaque" },
        "output_format": "png",
        "n": request.variants,
    })
}

/// A ureq failure as a [`GenError`].
fn transport_error(e: ureq::Error) -> GenError {
    match e {
        ureq::Error::Timeout(_) => GenError::Timeout,
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => GenError::Timeout,
        ureq::Error::HostNotFound => {
            GenError::Network("the server was not found (is there an internet connection?)".into())
        }
        ureq::Error::ConnectionFailed => GenError::Network("the connection failed".into()),
        ureq::Error::BodyExceedsLimit(_) => GenError::BadResponse("the answer is too large".into()),
        ureq::Error::Tls(_) | ureq::Error::NativeTls(_) => GenError::Network(format!(
            "the secure connection failed: {}",
            clean_message(&e.to_string())
        )),
        other => GenError::Network(clean_message(&other.to_string())),
    }
}

/// OpenAI's error object.
#[derive(Debug, Default, Deserialize)]
struct ApiError {
    #[serde(default)]
    message: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    code: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    error: ApiError,
}

#[derive(Debug, Deserialize)]
struct Image {
    #[serde(default)]
    b64_json: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ImagesBody {
    data: Vec<Image>,
}

/// The pictures of an answer with HTTP `status`, or the error it reports.
pub(crate) fn parse_response(
    status: u16,
    retry_after: Option<&str>,
    body: &[u8],
) -> Result<Vec<RgbaImage>, GenError> {
    if !(200..300).contains(&status) {
        return Err(status_error(status, retry_after, body));
    }
    let parsed: ImagesBody = serde_json::from_slice(body)
        .map_err(|e| GenError::BadResponse(format!("no picture list ({e})")))?;
    if parsed.data.is_empty() {
        return Err(GenError::BadResponse("the answer holds no picture".into()));
    }
    parsed
        .data
        .iter()
        .enumerate()
        .map(|(i, image)| {
            let b64 = image.b64_json.as_deref().ok_or_else(|| {
                GenError::BadResponse(format!("picture {} has no image data", i + 1))
            })?;
            decode_png(b64).map_err(|m| GenError::BadResponse(format!("picture {}: {m}", i + 1)))
        })
        .collect()
}

/// A base64 PNG as RGBA.
fn decode_png(b64: &str) -> Result<RgbaImage, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("bad base64 ({e})"))?;
    match dpx_formats::read_png(&bytes).map_err(|e| e.summary())? {
        dpx_formats::PngImage::TrueColor(img) => Ok(img),
        dpx_formats::PngImage::Indexed(pic) => {
            Ok(dpx_core::import::rgba_of(&pic.image, &pic.palette))
        }
    }
}

/// The error an HTTP status (with OpenAI's error body) stands for.
fn status_error(status: u16, retry_after: Option<&str>, body: &[u8]) -> GenError {
    let error = serde_json::from_slice::<ErrorBody>(body)
        .map(|b| b.error)
        .unwrap_or_default();
    let message = error
        .message
        .as_deref()
        .map(clean_message)
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| format!("HTTP {status}"));
    let code = match &error.code {
        Some(Value::String(s)) => s.as_str(),
        _ => "",
    };
    let kind = error.kind.as_deref().unwrap_or("");
    let is = |name: &str| code == name || kind == name;
    match status {
        401 | 403 => GenError::Auth { status, message },
        429 => GenError::RateLimited {
            quota_exhausted: is("insufficient_quota") || is("billing_hard_limit_reached"),
            retry_after: retry_after
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|s| s.is_finite() && *s >= 0.0)
                .map(|s| Duration::from_secs_f64(s.min(86_400.0))),
            message,
        },
        _ if is("moderation_blocked") || is("content_policy_violation") => {
            GenError::ContentPolicy(message)
        }
        408 => GenError::Timeout,
        400..=499 => GenError::InvalidRequest(message),
        500..=599 => GenError::Server { status, message },
        _ => GenError::BadResponse(format!("HTTP {status}: {message}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::Secret;
    use dpx_core::Rgba8;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    fn rgba_png(w: u32, h: u32, seed: u8) -> (RgbaImage, Vec<u8>) {
        let mut img = RgbaImage::new(w, h, Rgba8::default());
        let mut raw = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let p = Rgba8::new(seed, x as u8 * 40, y as u8 * 60, 255 - x as u8);
                img.set(x, y, p);
                raw.extend_from_slice(&[p.r, p.g, p.b, p.a]);
            }
        }
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().unwrap();
            writer.write_image_data(&raw).unwrap();
        }
        (img, out)
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn images_json(pngs: &[&[u8]]) -> Vec<u8> {
        let data: Vec<Value> = pngs.iter().map(|p| json!({ "b64_json": b64(p) })).collect();
        json!({ "created": 1, "data": data, "size": "1024x768" })
            .to_string()
            .into_bytes()
    }

    fn error_json(message: &str, kind: &str, code: Option<&str>) -> Vec<u8> {
        json!({ "error": { "message": message, "type": kind, "code": code, "param": null } })
            .to_string()
            .into_bytes()
    }

    #[test]
    fn the_request_body_has_every_field() {
        let mut r = GenerationRequest::new("a castle", (1024, 768));
        r.variants = 3;
        let body = request_body(&OpenAiConfig::default(), &r);
        assert_eq!(
            body,
            json!({
                "model": DEFAULT_MODEL,
                "prompt": "a castle",
                "size": "1024x768",
                "quality": "medium",
                "background": "opaque",
                "output_format": "png",
                "n": 3,
            })
        );
        r.transparent_background = true;
        let low = OpenAiConfig {
            quality: Quality::Low,
            model: "gpt-image-2.5-flare".into(),
            ..OpenAiConfig::default()
        };
        let body = request_body(&low, &r);
        assert_eq!(body["background"], "transparent");
        assert_eq!(body["quality"], "low");
        assert_eq!(body["model"], "gpt-image-2.5-flare");
    }

    #[test]
    fn answers_decode_to_pictures() {
        let (a, pa) = rgba_png(4, 3, 10);
        let (b, pb) = rgba_png(2, 5, 200);
        let got = parse_response(200, None, &images_json(&[&pa, &pb])).unwrap();
        assert_eq!(got, vec![a, b]);
        let bad = |body: &[u8]| match parse_response(200, None, body) {
            Err(GenError::BadResponse(m)) => m,
            other => panic!("{other:?}"),
        };
        assert!(bad(b"<html>").contains("no picture list"));
        assert!(bad(br#"{"data":[]}"#).contains("no picture"));
        assert!(bad(br#"{"data":[{"url":"x"}]}"#).contains("no image data"));
        assert!(bad(br#"{"data":[{"b64_json":"%%%"}]}"#).contains("bad base64"));
        let not_png = json!({ "data": [{ "b64_json": b64(b"GIF89a....") }] }).to_string();
        assert!(bad(not_png.as_bytes()).starts_with("picture 1:"));
    }

    #[test]
    fn errors_are_typed() {
        let e = parse_response(
            401,
            None,
            &error_json(
                "Incorrect API key provided: sk-proj-abc*****xyz.",
                "invalid_request_error",
                Some("invalid_api_key"),
            ),
        )
        .unwrap_err();
        assert_eq!(
            e,
            GenError::Auth {
                status: 401,
                message: "Incorrect API key provided: [key]".into()
            }
        );
        let e = parse_response(
            403,
            None,
            &error_json(
                "Your organization must be verified",
                "invalid_request_error",
                None,
            ),
        )
        .unwrap_err();
        assert!(matches!(e, GenError::Auth { status: 403, .. }));
        let e = parse_response(
            429,
            Some("7"),
            &error_json(
                "Rate limit reached",
                "requests",
                Some("rate_limit_exceeded"),
            ),
        )
        .unwrap_err();
        assert_eq!(
            e,
            GenError::RateLimited {
                quota_exhausted: false,
                retry_after: Some(Duration::from_secs(7)),
                message: "Rate limit reached".into()
            }
        );
        let e = parse_response(
            429,
            None,
            &error_json(
                "You exceeded your current quota",
                "insufficient_quota",
                Some("insufficient_quota"),
            ),
        )
        .unwrap_err();
        assert_eq!(e.code(), "QUOTA_EXCEEDED");
        let e = parse_response(
            400,
            None,
            &error_json(
                "Your request was rejected by the safety system",
                "image_generation_user_error",
                Some("moderation_blocked"),
            ),
        )
        .unwrap_err();
        assert!(matches!(e, GenError::ContentPolicy(_)));
        let e = parse_response(
            400,
            None,
            &error_json("Invalid size", "invalid_request_error", None),
        )
        .unwrap_err();
        assert_eq!(e, GenError::InvalidRequest("Invalid size".into()));
        let e = parse_response(
            404,
            None,
            &error_json("The model does not exist", "x", None),
        )
        .unwrap_err();
        assert!(matches!(e, GenError::InvalidRequest(_)));
        let e = parse_response(503, None, b"upstream connect error").unwrap_err();
        assert_eq!(
            e,
            GenError::Server {
                status: 503,
                message: "HTTP 503".into()
            }
        );
        assert_eq!(
            parse_response(408, None, b"").unwrap_err(),
            GenError::Timeout
        );
    }

    /// Serves one canned answer on 127.0.0.1 and returns the request it got.
    fn serve_once(answer: Vec<u8>, delay: Duration) -> (String, thread::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/v1/images/generations",
            listener.local_addr().unwrap()
        );
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            thread::sleep(delay);
            let _ = stream.write_all(&answer);
            let _ = stream.flush();
            request
        });
        (url, handle)
    }

    fn read_request(stream: &mut TcpStream) -> Vec<u8> {
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
                let len = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if buf.len() >= end + 4 + len {
                    break;
                }
            }
        }
        buf
    }

    fn http(status: &str, extra: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n{extra}\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    fn keys_with(key: &str) -> Arc<ApiKeys> {
        let keys = ApiKeys::in_memory();
        keys.set(KEY_ACCOUNT, &Secret::new(key).unwrap());
        Arc::new(keys)
    }

    fn provider(url: &str, keys: Arc<ApiKeys>, timeout: Duration) -> OpenAiProvider {
        OpenAiProvider::new(OpenAiConfig::default(), keys).with_test_server(url, timeout)
    }

    #[test]
    fn a_whole_request_against_a_local_server() {
        let (a, pa) = rgba_png(4, 3, 1);
        let (b, pb) = rgba_png(4, 3, 2);
        let (url, server) = serve_once(
            http("200 OK", "", &images_json(&[&pa, &pb])),
            Duration::ZERO,
        );
        let p = provider(&url, keys_with("sk-test-123"), Duration::from_secs(10));
        assert_eq!(p.status(), ProviderStatus::Ready);
        let mut r = GenerationRequest::for_page("a castle", &p.capabilities(), (320, 240), (1, 1));
        r.variants = 2;
        let got = p.generate(&r, &CancelToken::new()).unwrap();
        assert_eq!(got, vec![a, b]);
        let request = server.join().unwrap();
        let text = String::from_utf8_lossy(&request);
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        assert!(
            head.starts_with("POST /v1/images/generations HTTP/1.1"),
            "{head}"
        );
        let head = head.to_ascii_lowercase();
        assert!(head.contains("authorization: bearer sk-test-123"), "{head}");
        assert!(head.contains("content-type: application/json"), "{head}");
        assert!(head.contains("user-agent: deluxepainter"), "{head}");
        let sent: Value = serde_json::from_str(body).unwrap();
        assert_eq!(sent, request_body(&OpenAiConfig::default(), &r));
        assert_eq!(sent["size"], "1024x768");
    }

    #[test]
    fn server_errors_reach_the_caller_typed() {
        let (url, server) = serve_once(
            http(
                "429 Too Many Requests",
                "Retry-After: 3\r\n",
                &error_json("Slow down", "requests", Some("rate_limit_exceeded")),
            ),
            Duration::ZERO,
        );
        let p = provider(&url, keys_with("sk-x"), Duration::from_secs(10));
        let r = GenerationRequest::new("x", (1024, 1024));
        let e = p.generate(&r, &CancelToken::new()).unwrap_err();
        assert_eq!(
            e,
            GenError::RateLimited {
                quota_exhausted: false,
                retry_after: Some(Duration::from_secs(3)),
                message: "Slow down".into()
            }
        );
        server.join().unwrap();
        let (url, server) = serve_once(
            http(
                "401 Unauthorized",
                "",
                &error_json(
                    "Incorrect API key provided: sk-x.",
                    "invalid_request_error",
                    None,
                ),
            ),
            Duration::ZERO,
        );
        let p = provider(&url, keys_with("sk-x"), Duration::from_secs(10));
        let e = p.generate(&r, &CancelToken::new()).unwrap_err();
        assert!(matches!(e, GenError::Auth { status: 401, .. }), "{e:?}");
        server.join().unwrap();
    }

    #[test]
    fn a_silent_server_times_out_and_a_closed_port_is_a_network_error() {
        let (url, server) = serve_once(Vec::new(), Duration::from_secs(2));
        let p = provider(&url, keys_with("sk-x"), Duration::from_millis(300));
        let r = GenerationRequest::new("x", (1024, 1024));
        let started = std::time::Instant::now();
        assert_eq!(p.generate(&r, &CancelToken::new()), Err(GenError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
        // A port nobody listens on.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let url = format!("http://127.0.0.1:{port}/v1/images/generations");
        let p = provider(&url, keys_with("sk-x"), Duration::from_secs(5));
        let e = p.generate(&r, &CancelToken::new()).unwrap_err();
        assert!(matches!(e, GenError::Network(_)), "{e:?}");
    }

    #[test]
    fn nothing_is_sent_without_a_key_a_model_or_after_cancelling() {
        // A port nobody listens on: sending anything would be a network error.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let url = format!("http://127.0.0.1:{port}/");
        let r = GenerationRequest::new("x", (1024, 1024));
        let none = provider(&url, Arc::new(ApiKeys::in_memory()), Duration::from_secs(5));
        assert_eq!(
            none.status(),
            ProviderStatus::NotConfigured("no OpenAI API key".into())
        );
        assert!(matches!(
            none.generate(&r, &CancelToken::new()),
            Err(GenError::NotConfigured(_))
        ));
        let p = provider(&url, keys_with("sk-x"), Duration::from_secs(5));
        let cancel = CancelToken::new();
        cancel.cancel();
        assert_eq!(p.generate(&r, &cancel), Err(GenError::Cancelled));
        // A size OpenAI does not make is refused before sending.
        let odd = GenerationRequest::new("x", (320, 240));
        assert!(matches!(
            p.generate(&odd, &CancelToken::new()),
            Err(GenError::InvalidRequest(_))
        ));
        let mut bad_model = OpenAiProvider::new(
            OpenAiConfig {
                model: "two words".into(),
                ..OpenAiConfig::default()
            },
            keys_with("sk-x"),
        );
        bad_model.endpoint = url;
        assert!(matches!(
            bad_model.status(),
            ProviderStatus::NotConfigured(_)
        ));
    }

    #[test]
    fn config_round_trips_and_fills_in_defaults() {
        let c: OpenAiConfig = serde_json::from_str(r#"{"quality":"high"}"#).unwrap();
        assert_eq!(c.model, DEFAULT_MODEL);
        assert_eq!(c.quality, Quality::High);
        let back: OpenAiConfig = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        assert!(COST_NOTE.is_ascii());
    }
}
