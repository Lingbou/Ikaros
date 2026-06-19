// SPDX-License-Identifier: GPL-3.0-only

use crate::{TokenEstimator, estimate_tokens_heuristic};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContextTokenizerKind {
    #[default]
    Heuristic,
    OpenAiCompatible,
    AnthropicFallback,
    OllamaFallback,
    Mock,
}

impl ContextTokenizerKind {
    pub fn estimator(self) -> ContextTokenEstimator {
        ContextTokenEstimator { kind: self }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ContextTokenEstimator {
    kind: ContextTokenizerKind,
}

impl ContextTokenEstimator {
    pub fn new(kind: ContextTokenizerKind) -> Self {
        Self { kind }
    }

    pub fn kind(&self) -> ContextTokenizerKind {
        self.kind
    }
}

impl TokenEstimator for ContextTokenEstimator {
    fn name(&self) -> &'static str {
        match self.kind {
            ContextTokenizerKind::Heuristic => "heuristic-v1",
            ContextTokenizerKind::OpenAiCompatible => "openai-compatible-chatml-v1",
            ContextTokenizerKind::AnthropicFallback => "anthropic-fallback-heuristic-v1",
            ContextTokenizerKind::OllamaFallback => "ollama-fallback-heuristic-v1",
            ContextTokenizerKind::Mock => "mock-tokenizer-v1",
        }
    }

    fn estimate_tokens(&self, text: &str) -> usize {
        match self.kind {
            ContextTokenizerKind::Heuristic
            | ContextTokenizerKind::AnthropicFallback
            | ContextTokenizerKind::OllamaFallback => estimate_tokens_heuristic(text),
            ContextTokenizerKind::OpenAiCompatible => estimate_openai_compatible_tokens(text),
            ContextTokenizerKind::Mock => estimate_mock_tokens(text),
        }
    }
}

fn estimate_openai_compatible_tokens(text: &str) -> usize {
    let mut tokens = 0usize;
    let mut ascii_run = 0usize;
    for ch in text.chars() {
        if ch.is_whitespace() {
            tokens += finish_ascii_run(&mut ascii_run);
            continue;
        }
        if is_cjk_like(ch) {
            tokens += finish_ascii_run(&mut ascii_run);
            tokens += 1;
            continue;
        }
        if ch.is_ascii_alphanumeric() || ch == '_' {
            ascii_run += 1;
            continue;
        }
        tokens += finish_ascii_run(&mut ascii_run);
        tokens += 1;
    }
    tokens += finish_ascii_run(&mut ascii_run);
    tokens.max(1)
}

fn estimate_mock_tokens(text: &str) -> usize {
    text.split_whitespace().count().max(1)
}

fn finish_ascii_run(run: &mut usize) -> usize {
    let tokens = run.div_ceil(4);
    *run = 0;
    tokens
}

fn is_cjk_like(ch: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&ch)
        || ('\u{3040}'..='\u{30ff}').contains(&ch)
        || ('\u{ac00}'..='\u{d7af}').contains(&ch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_compatible_estimator_counts_punctuation_boundaries() {
        let heuristic = ContextTokenizerKind::Heuristic.estimator();
        let openai = ContextTokenizerKind::OpenAiCompatible.estimator();

        assert!(
            openai.estimate_tokens("hello, world!") > heuristic.estimate_tokens("hello, world!")
        );
        assert_eq!(openai.name(), "openai-compatible-chatml-v1");
    }

    #[test]
    fn fallback_estimators_are_explicitly_named() {
        assert_eq!(
            ContextTokenizerKind::AnthropicFallback.estimator().name(),
            "anthropic-fallback-heuristic-v1"
        );
        assert_eq!(
            ContextTokenizerKind::OllamaFallback.estimator().name(),
            "ollama-fallback-heuristic-v1"
        );
    }

    #[test]
    fn mock_estimator_is_stable_for_tests() {
        let mock = ContextTokenizerKind::Mock.estimator();

        assert_eq!(mock.estimate_tokens("one two three"), 3);
        assert_eq!(mock.name(), "mock-tokenizer-v1");
    }
}
