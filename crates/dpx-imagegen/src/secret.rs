//! API keys (ADR-45): kept in the OS keychain, never in a settings or project file.
//!
//! - [`KeychainStore`]: the system keychain through the `keyring` crate (macOS Keychain,
//!   Windows Credential Manager, the Secret Service on Linux and the BSDs).
//! - [`MemoryStore`]: for this session only. [`ApiKeys::system`] falls back to it when there
//!   is no keychain (e.g. Linux without a Secret Service daemon) or the keychain refuses to
//!   store, and keeps the reason, so the GUI can say "kept for this session only".
//! - The environment variable (e.g. `OPENAI_API_KEY`) is read as a last, read-only source:
//!   a key typed into DeluxePainter wins; clearing the key in DeluxePainter cannot clear the
//!   variable.
//!
//! [`Secret`] never shows its value in `Debug` output. Tests use [`MemoryStore`] and an
//! injected environment only; they never touch the real keychain or environment.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

/// The keychain service name entries are stored under.
pub const KEYCHAIN_SERVICE: &str = "DeluxePainter";

/// A secret value (an API key); `Debug` shows only that it is set.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// The key without surrounding whitespace; `None` if that is empty or holds anything but
    /// printable ASCII without spaces (no API key does, and it goes into an HTTP header; a
    /// pasted line break is trimmed).
    pub fn new(value: &str) -> Option<Self> {
        let v = value.trim();
        (!v.is_empty() && v.chars().all(|c| c.is_ascii_graphic())).then(|| Self(v.to_string()))
    }

    /// The value, for the one place that sends it (an HTTP header).
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret(<{} characters>)", self.0.chars().count())
    }
}

/// Why a store failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretError {
    /// There is no keychain on this system (or it cannot be reached).
    Unavailable(String),
    /// The keychain is locked or access was refused.
    Denied(String),
    /// Anything else the keychain reported.
    Failed(String),
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretError::Unavailable(m) => write!(f, "no system keychain: {m}"),
            SecretError::Denied(m) => write!(f, "the system keychain refused access: {m}"),
            SecretError::Failed(m) => write!(f, "the system keychain failed: {m}"),
        }
    }
}

impl std::error::Error for SecretError {}

/// What kind of store a [`SecretStore`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreKind {
    /// Kept by the operating system across sessions.
    Keychain,
    /// Kept in memory until DeluxePainter quits.
    Session,
}

/// Where secrets are kept, by account name (e.g. `openai`).
pub trait SecretStore: Send + Sync {
    fn kind(&self) -> StoreKind;
    fn get(&self, account: &str) -> Result<Option<Secret>, SecretError>;
    fn set(&self, account: &str, secret: &Secret) -> Result<(), SecretError>;
    /// Removes the secret; nothing to remove is fine.
    fn delete(&self, account: &str) -> Result<(), SecretError>;
}

/// Secrets in memory, for this session only.
#[derive(Default)]
pub struct MemoryStore {
    secrets: Mutex<HashMap<String, Secret>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemoryStore {
    fn kind(&self) -> StoreKind {
        StoreKind::Session
    }

    fn get(&self, account: &str) -> Result<Option<Secret>, SecretError> {
        Ok(lock(&self.secrets).get(account).cloned())
    }

    fn set(&self, account: &str, secret: &Secret) -> Result<(), SecretError> {
        lock(&self.secrets).insert(account.to_string(), secret.clone());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        lock(&self.secrets).remove(account);
        Ok(())
    }
}

/// The OS keychain (service [`KEYCHAIN_SERVICE`]).
pub struct KeychainStore {
    service: String,
}

impl KeychainStore {
    /// The system keychain, if there is one this process can use.
    pub fn open(service: &str) -> Result<Self, SecretError> {
        match keyring::Entry::store_status() {
            Ok(()) => Ok(Self {
                service: service.to_string(),
            }),
            Err(e) => Err(SecretError::Unavailable(e.to_string())),
        }
    }

    fn entry(&self, account: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(&self.service, account).map_err(keychain_error)
    }
}

fn keychain_error(e: keyring::Error) -> SecretError {
    match e {
        keyring::Error::NoStorageAccess(_) => SecretError::Denied(e.to_string()),
        keyring::Error::NoDefaultStore => SecretError::Unavailable(e.to_string()),
        _ => SecretError::Failed(e.to_string()),
    }
}

impl SecretStore for KeychainStore {
    fn kind(&self) -> StoreKind {
        StoreKind::Keychain
    }

    fn get(&self, account: &str) -> Result<Option<Secret>, SecretError> {
        match self.entry(account)?.get_password() {
            Ok(v) => Ok(Secret::new(&v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keychain_error(e)),
        }
    }

    fn set(&self, account: &str, secret: &Secret) -> Result<(), SecretError> {
        self.entry(account)?
            .set_password(secret.expose())
            .map_err(keychain_error)
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keychain_error(e)),
        }
    }
}

/// Where a key came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeySource {
    /// The system keychain.
    Keychain,
    /// Typed in this session and kept in memory only.
    Session,
    /// The environment variable.
    Environment,
}

/// What the GUI shows about a key (never the key).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyStatus {
    pub source: Option<KeySource>,
    /// Why new keys are kept for this session only, if they are.
    pub session_only: Option<String>,
    /// The environment variable that is read.
    pub env_var: String,
}

impl KeyStatus {
    /// One ASCII line for the pixel UI.
    pub fn describe(&self) -> String {
        match self.source {
            Some(KeySource::Keychain) => "API key saved in the system keychain".into(),
            Some(KeySource::Session) => {
                "API key kept for this session only (no system keychain)".into()
            }
            Some(KeySource::Environment) => {
                format!("API key from the environment variable {}", self.env_var)
            }
            None => "No API key set up".into(),
        }
    }
}

/// Environment lookup (injectable for tests).
pub type EnvReader = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The API keys of all providers: a persistent store (the keychain), a session store and the
/// environment, looked up in the order session (a key typed but not storable), persistent
/// store, environment.
pub struct ApiKeys {
    persistent: Option<Box<dyn SecretStore>>,
    session: MemoryStore,
    /// Why keys are kept for this session only (no or a failing keychain).
    session_reason: Mutex<Option<String>>,
    /// Keys read from the persistent store (each read may prompt the user on macOS).
    cache: Mutex<HashMap<String, Option<Secret>>>,
    env: EnvReader,
}

impl fmt::Debug for ApiKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiKeys")
            .field("persistent", &self.persistent.as_ref().map(|s| s.kind()))
            .field("session_reason", &*lock(&self.session_reason))
            .finish_non_exhaustive()
    }
}

impl ApiKeys {
    /// The system keychain, or for this session only when there is none (the reason is
    /// kept), and the process environment.
    pub fn system() -> Self {
        let env: EnvReader = Arc::new(|name: &str| std::env::var(name).ok());
        match KeychainStore::open(KEYCHAIN_SERVICE) {
            Ok(store) => Self::new(Some(Box::new(store)), None, env),
            Err(e) => Self::new(None, Some(e.to_string()), env),
        }
    }

    /// Keys in `persistent` (none: session only, for `reason`), with `env` as the
    /// environment.
    pub fn new(
        persistent: Option<Box<dyn SecretStore>>,
        reason: Option<String>,
        env: EnvReader,
    ) -> Self {
        let reason = match (&persistent, reason) {
            (None, None) => Some("no system keychain".to_string()),
            (_, r) => r,
        };
        Self {
            persistent,
            session: MemoryStore::new(),
            session_reason: Mutex::new(reason),
            cache: Mutex::new(HashMap::new()),
            env,
        }
    }

    /// Session keys only and no environment (tests, debug runs).
    pub fn in_memory() -> Self {
        Self::new(
            None,
            Some("test store".into()),
            Arc::new(|_: &str| None::<String>),
        )
    }

    /// The key for `account`, falling back to the environment variable `env_var`.
    pub fn key(&self, account: &str, env_var: &str) -> Option<(Secret, KeySource)> {
        if let Ok(Some(s)) = self.session.get(account) {
            return Some((s, KeySource::Session));
        }
        if let Some(s) = self.stored(account) {
            return Some((s, KeySource::Keychain));
        }
        (self.env)(env_var)
            .and_then(|v| Secret::new(&v))
            .map(|s| (s, KeySource::Environment))
    }

    fn stored(&self, account: &str) -> Option<Secret> {
        let store = self.persistent.as_ref()?;
        if let Some(cached) = lock(&self.cache).get(account) {
            return cached.clone();
        }
        // A failing read counts as no key (the GUI offers to enter one); not cached.
        let read = store.get(account).ok()?;
        lock(&self.cache).insert(account.to_string(), read.clone());
        read
    }

    /// What the GUI shows.
    pub fn status(&self, account: &str, env_var: &str) -> KeyStatus {
        KeyStatus {
            source: self.key(account, env_var).map(|(_, s)| s),
            session_only: lock(&self.session_reason).clone(),
            env_var: env_var.to_string(),
        }
    }

    /// Stores a key: in the keychain, or, when there is none or it refuses, for this session
    /// only (the reason then shows in [`Self::status`]). Returns where it went.
    pub fn set(&self, account: &str, secret: &Secret) -> KeySource {
        if let Some(store) = &self.persistent {
            match store.set(account, secret) {
                Ok(()) => {
                    let _ = self.session.delete(account);
                    lock(&self.cache).insert(account.to_string(), Some(secret.clone()));
                    // The keychain works (again): new keys are no longer session-only.
                    *lock(&self.session_reason) = None;
                    return KeySource::Keychain;
                }
                Err(e) => *lock(&self.session_reason) = Some(e.to_string()),
            }
        }
        let _ = self.session.set(account, secret);
        KeySource::Session
    }

    /// Forgets the key in DeluxePainter (keychain and session). The environment variable
    /// cannot be cleared from here.
    pub fn clear(&self, account: &str) -> Result<(), SecretError> {
        let _ = self.session.delete(account);
        lock(&self.cache).remove(account);
        match &self.persistent {
            Some(store) => store.delete(account),
            None => Ok(()),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A keychain stand-in that may refuse.
    #[derive(Default)]
    struct FakeKeychain {
        inner: MemoryStore,
        refuse: Arc<std::sync::atomic::AtomicBool>,
        reads: Arc<Mutex<usize>>,
    }

    impl SecretStore for FakeKeychain {
        fn kind(&self) -> StoreKind {
            StoreKind::Keychain
        }
        fn get(&self, account: &str) -> Result<Option<Secret>, SecretError> {
            *lock(&*self.reads) += 1;
            self.inner.get(account)
        }
        fn set(&self, account: &str, secret: &Secret) -> Result<(), SecretError> {
            if self.refuse.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(SecretError::Denied("locked".into()));
            }
            self.inner.set(account, secret)
        }
        fn delete(&self, account: &str) -> Result<(), SecretError> {
            self.inner.delete(account)
        }
    }

    fn env(value: Option<&'static str>) -> EnvReader {
        Arc::new(move |name: &str| {
            assert_eq!(name, "TEST_KEY");
            value.map(str::to_string)
        })
    }

    fn key(s: &str) -> Secret {
        Secret::new(s).unwrap()
    }

    #[test]
    fn secrets_are_trimmed_checked_and_never_shown() {
        assert_eq!(key("  sk-abc\n").expose(), "sk-abc");
        assert!(Secret::new("   ").is_none());
        assert!(Secret::new("sk a").is_none());
        assert!(Secret::new("sk\tb").is_none());
        let s = key("sk-very-secret");
        assert_eq!(format!("{s:?}"), "Secret(<14 characters>)");
        let keys = ApiKeys::in_memory();
        keys.set("openai", &s);
        assert!(!format!("{keys:?}").contains("very-secret"));
    }

    #[test]
    fn keychain_then_environment() {
        let keys = ApiKeys::new(
            Some(Box::new(FakeKeychain::default())),
            None,
            env(Some("sk-env")),
        );
        assert_eq!(
            keys.key("openai", "TEST_KEY"),
            Some((key("sk-env"), KeySource::Environment))
        );
        assert_eq!(keys.set("openai", &key("sk-mine")), KeySource::Keychain);
        assert_eq!(
            keys.key("openai", "TEST_KEY"),
            Some((key("sk-mine"), KeySource::Keychain))
        );
        let status = keys.status("openai", "TEST_KEY");
        assert_eq!(status.source, Some(KeySource::Keychain));
        assert_eq!(status.session_only, None);
        assert_eq!(status.describe(), "API key saved in the system keychain");
        // Clearing leaves the read-only environment.
        keys.clear("openai").unwrap();
        assert_eq!(
            keys.key("openai", "TEST_KEY").map(|(_, s)| s),
            Some(KeySource::Environment)
        );
        assert_eq!(
            keys.status("openai", "TEST_KEY").describe(),
            "API key from the environment variable TEST_KEY"
        );
    }

    #[test]
    fn keychain_reads_are_cached() {
        let chain = FakeKeychain::default();
        chain.inner.set("openai", &key("sk-old")).unwrap();
        let reads = chain.reads.clone();
        let keys = ApiKeys::new(Some(Box::new(chain)), None, env(None));
        for _ in 0..3 {
            assert_eq!(keys.key("openai", "TEST_KEY").unwrap().0, key("sk-old"));
        }
        assert_eq!(*lock(&reads), 1);
        // A new key replaces the cached one without another read.
        keys.set("openai", &key("sk-new"));
        assert_eq!(keys.key("openai", "TEST_KEY").unwrap().0, key("sk-new"));
        assert_eq!(*lock(&reads), 1);
    }

    #[test]
    fn without_a_keychain_keys_last_the_session_and_say_so() {
        let keys = ApiKeys::new(None, Some("no Secret Service".into()), env(None));
        assert_eq!(keys.status("openai", "TEST_KEY").source, None);
        assert_eq!(
            keys.status("openai", "TEST_KEY").describe(),
            "No API key set up"
        );
        assert_eq!(keys.set("openai", &key("sk-1")), KeySource::Session);
        let status = keys.status("openai", "TEST_KEY");
        assert_eq!(status.source, Some(KeySource::Session));
        assert_eq!(status.session_only.as_deref(), Some("no Secret Service"));
        assert!(status.describe().contains("this session only"));
        keys.clear("openai").unwrap();
        assert_eq!(keys.key("openai", "TEST_KEY"), None);
    }

    #[test]
    fn a_refusing_keychain_falls_back_to_the_session() {
        let chain = FakeKeychain::default();
        let refuse = chain.refuse.clone();
        refuse.store(true, std::sync::atomic::Ordering::SeqCst);
        let keys = ApiKeys::new(Some(Box::new(chain)), None, env(Some("sk-env")));
        assert_eq!(keys.status("openai", "TEST_KEY").session_only, None);
        assert_eq!(keys.set("openai", &key("sk-typed")), KeySource::Session);
        // The typed key wins over the environment.
        assert_eq!(
            keys.key("openai", "TEST_KEY"),
            Some((key("sk-typed"), KeySource::Session))
        );
        let status = keys.status("openai", "TEST_KEY");
        assert!(status.session_only.unwrap().contains("refused"));
        // Unlocked again: the next key goes to the keychain and nothing says session-only.
        refuse.store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(keys.set("openai", &key("sk-later")), KeySource::Keychain);
        let status = keys.status("openai", "TEST_KEY");
        assert_eq!(status.source, Some(KeySource::Keychain));
        assert_eq!(status.session_only, None);
    }
}
