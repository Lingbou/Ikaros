// SPDX-License-Identifier: GPL-3.0-only

use ikaros_context::{
    HeuristicTokenEstimator, PromptBuildReport, PromptBuilder, PromptSectionKind, PromptSourceKind,
    TokenEstimator,
};
use ikaros_core::{ResolvedAgentProfile, RuntimeContext, redact_secrets};
use ikaros_soul::PersonaProfile;

pub fn render_persona_agent_context(
    persona: &PersonaProfile,
    agent: &ResolvedAgentProfile,
) -> String {
    let persona_context = persona.context_summary();
    if agent.profile.persona_overlay.trim().is_empty() {
        return persona_context;
    }
    redact_secrets(&format!(
        "{}\n\nAgent profile: {} ({})\n{}",
        persona_context,
        agent.name,
        agent.mode(),
        agent.profile.persona_overlay
    ))
}

pub fn render_chat_system_prompt(context: &RuntimeContext) -> String {
    build_chat_system_prompt(context, &HeuristicTokenEstimator).prompt
}

pub fn build_chat_system_prompt(
    context: &RuntimeContext,
    estimator: &dyn TokenEstimator,
) -> PromptBuildReport {
    let mut builder = PromptBuilder::new(estimator)
        .add_section(
            PromptSectionKind::Persona,
            "Persona",
            context.persona_context.clone(),
            PromptSourceKind::Persona,
            100,
        )
        .add_optional_section(
            PromptSectionKind::Relationship,
            "Local relationship context",
            context_lines(&context.relationship_context),
            PromptSourceKind::Memory,
            90,
        )
        .add_optional_section(
            PromptSectionKind::References,
            "Local reference context",
            context_lines(&context.reference_context),
            PromptSourceKind::Context,
            95,
        )
        .add_optional_section(
            PromptSectionKind::History,
            "Local chat history context",
            context_lines(&context.chat_history_context),
            PromptSourceKind::Context,
            70,
        )
        .add_optional_section(
            PromptSectionKind::MemoryProjection,
            "Accepted memory projection",
            context_lines(&context.memory_projection_context),
            PromptSourceKind::Memory,
            85,
        )
        .add_optional_section(
            PromptSectionKind::WorkingMemory,
            "Session working memory",
            context_lines(&context.working_memory_context),
            PromptSourceKind::Memory,
            65,
        )
        .add_optional_section(
            PromptSectionKind::RetrievedMemory,
            "Retrieved memory context",
            context_lines(&context.retrieved_memory_context),
            PromptSourceKind::Memory,
            60,
        )
        .add_optional_section(
            PromptSectionKind::Rag,
            "Local RAG context",
            context_lines(&context.rag_context),
            PromptSourceKind::Rag,
            55,
        );
    if let Some(prompt) = &context.context_continuation_prompt {
        builder = builder.add_section(
            PromptSectionKind::ContextCompression,
            "Context compression notice",
            prompt.clone(),
            PromptSourceKind::Context,
            92,
        );
    }
    builder
        .add_section(
            PromptSectionKind::Policy,
            "Policy",
            "Use local context when relevant. Treat accepted projections as more authoritative than working or retrieved memory. Do not reveal secrets.",
            PromptSourceKind::Runtime,
            100,
        )
        .add_section(
            PromptSectionKind::ToolGuidance,
            "Tool guidance",
            "This single-call chat prompt has no direct tools. Tool use and writes must remain behind the harness through a tool-capable runtime path that provides its own tool manifest.",
            PromptSourceKind::Tooling,
            100,
        )
        .build()
}

fn context_lines(lines: &[String]) -> String {
    lines.join("\n")
}
