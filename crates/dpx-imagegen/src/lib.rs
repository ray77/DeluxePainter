//! DeluxePainter AI image generation, core part (ADR-45). No GUI code: the GUI (Picture ▸
//! Generate..) and MCP tools build on it.
//!
//! - [`ImageGenerationProvider`]: the interface of a real picture generator (cloud or local),
//!   with its [`Capabilities`], honest cost and network notes, and typed [`GenError`]s.
//! - [`OpenAiProvider`]: OpenAI Images, the first real provider (owner decision).
//! - [`ApiKeys`]: API keys in the OS keychain, for this session only when there is none, and
//!   the provider's environment variable as a read-only fallback.
//! - [`JobManager`]: generations on worker threads with ids, polling and cancelling.
//! - [`SizeRule`]: which size to ask a provider for, from the page's display proportions.
//!
//! The pictures a provider returns are true colour; `dpx_core::import::convert` turns them
//! into a page of exactly the target's size and colour limits, and
//! `dpx_core::Command::ReplacePicture` makes one the whole picture in one undo step.
//!
//! There is no fake generation: [`fixture::TestPattern`] draws a fixed test pattern for tests
//! and debug snapshots only and is never in [`user_providers`].

pub mod fixture;
pub mod jobs;
pub mod openai;
pub mod provider;
pub mod secret;
pub mod size;

use std::sync::Arc;

pub use jobs::{JobId, JobManager, JobState, JobStatus};
pub use openai::{OpenAiConfig, OpenAiProvider, Quality};
pub use provider::{
    CancelToken, Capabilities, GenError, GenerationRequest, ImageGenerationProvider, ProviderStatus,
};
pub use secret::{ApiKeys, KeySource, KeyStatus, Secret, SecretError, SecretStore};
pub use size::{CustomSizes, SizeRule};

/// The ids of the providers a user can choose, in the order they are offered.
pub const USER_PROVIDER_IDS: [&str; 1] = [openai::ID];

/// The providers a user can choose (GUI list, MCP): real generators only.
pub fn user_providers(
    keys: Arc<ApiKeys>,
    openai: OpenAiConfig,
) -> Vec<Arc<dyn ImageGenerationProvider>> {
    vec![Arc::new(OpenAiProvider::new(openai, keys))]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_generators_are_offered() {
        let list = user_providers(Arc::new(ApiKeys::in_memory()), OpenAiConfig::default());
        let ids: Vec<&str> = list.iter().map(|p| p.id()).collect();
        assert_eq!(ids, USER_PROVIDER_IDS);
        assert!(!ids.contains(&fixture::TEST_PATTERN_ID));
        for p in &list {
            assert!(p.cost_note().is_ascii() && p.display_name().is_ascii());
            // Without a key nothing is ready, and the GUI says what is missing.
            assert!(matches!(p.status(), ProviderStatus::NotConfigured(_)));
        }
    }

    #[test]
    fn providers_are_shareable_across_threads() {
        fn check<T: Send + Sync + ?Sized>() {}
        check::<dyn ImageGenerationProvider>();
        check::<ApiKeys>();
    }
}
