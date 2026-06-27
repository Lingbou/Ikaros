// SPDX-License-Identifier: GPL-3.0-only

use super::types::ContextAssembleInput;
use ikaros_context::{ContextBudget, ContextTokenEstimator, ContextTokenizerKind, TokenEstimator};
use ikaros_models::{ModelContextProfile, ModelTokenizerKind};

pub(super) fn context_budget_for_input(
    input: &ContextAssembleInput<'_>,
    estimator: impl Into<String>,
) -> ContextBudget {
    let requested = input.options.context_token_budget;
    let Some(model_context) = input.model_context else {
        return ContextBudget::new(requested, estimator);
    };
    let available = model_context
        .available_context_tokens(input.reserved_system_tokens)
        .max(1) as usize;
    let max_tokens = if requested == 0 {
        available
    } else {
        requested.min(available)
    };
    ContextBudget::new(max_tokens, estimator).with_model_window(
        requested,
        model_context.context_window,
        model_context.default_output_tokens,
        input.reserved_system_tokens,
        model_context.source.clone(),
    )
}

pub fn context_estimator_for_model(
    model_context: Option<&ModelContextProfile>,
) -> ContextTokenEstimator {
    context_tokenizer_for_model(model_context).estimator()
}

pub fn context_tokenizer_for_model(
    model_context: Option<&ModelContextProfile>,
) -> ContextTokenizerKind {
    match model_context.map(|context| context.tokenizer) {
        Some(ModelTokenizerKind::OpenAiCompatible) => ContextTokenizerKind::OpenAiCompatible,
        Some(ModelTokenizerKind::Anthropic) => ContextTokenizerKind::AnthropicFallback,
        Some(ModelTokenizerKind::Ollama) => ContextTokenizerKind::OllamaFallback,
        Some(ModelTokenizerKind::Mock) => ContextTokenizerKind::Mock,
        Some(ModelTokenizerKind::Heuristic) | None => ContextTokenizerKind::Heuristic,
    }
}

pub(super) fn context_estimator_for_input(
    input: &ContextAssembleInput<'_>,
) -> ContextTokenEstimator {
    context_estimator_for_model(input.model_context)
}

pub(super) fn cap_reference_context(
    references: &mut Vec<String>,
    budget: &ContextBudget,
    estimator: &dyn TokenEstimator,
) {
    if budget.is_unbounded() || references.is_empty() {
        return;
    }
    let max_reference_tokens = (budget.max_tokens / 2).max(1);
    let reference_tokens = references
        .iter()
        .map(|reference| estimator.estimate_tokens(reference))
        .sum::<usize>();
    if reference_tokens <= max_reference_tokens {
        return;
    }

    let mut remaining = max_reference_tokens;
    let mut capped = Vec::with_capacity(references.len());
    let mut omitted = 0usize;
    for reference in references.iter() {
        if remaining == 0 {
            omitted += 1;
            continue;
        }
        let next = truncate_reference_for_budget(reference, remaining, estimator);
        remaining = remaining.saturating_sub(estimator.estimate_tokens(&next));
        capped.push(next);
    }
    if omitted > 0 && remaining > 0 {
        let marker = format!(
            "[reference] omitted {omitted} reference(s): explicit references capped at 50% context budget"
        );
        let marker = truncate_reference_for_budget(&marker, remaining, estimator);
        capped.push(marker);
    }
    *references = capped;
}

fn truncate_reference_for_budget(
    reference: &str,
    max_tokens: usize,
    estimator: &dyn TokenEstimator,
) -> String {
    let marker = "[reference] truncated: explicit references capped at 50% context budget";
    if max_tokens == 0 || estimator.estimate_tokens(marker) >= max_tokens {
        return marker.into();
    }

    let mut output = String::new();
    for line in reference.lines() {
        let candidate = if output.is_empty() {
            line.to_owned()
        } else {
            format!("{output}\n{line}")
        };
        let with_marker = format!("{candidate}\n{marker}");
        if estimator.estimate_tokens(&with_marker) > max_tokens {
            break;
        }
        output = candidate;
    }
    if output.is_empty() {
        marker.into()
    } else {
        format!("{output}\n{marker}")
    }
}
