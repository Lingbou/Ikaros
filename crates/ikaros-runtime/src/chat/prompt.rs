// SPDX-License-Identifier: GPL-3.0-only

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
    let relationship = if context.relationship_context.is_empty() {
        "none".into()
    } else {
        context.relationship_context.join("\n")
    };
    let references = if context.reference_context.is_empty() {
        "none".into()
    } else {
        context.reference_context.join("\n")
    };
    let history = if context.chat_history_context.is_empty() {
        "none".into()
    } else {
        context.chat_history_context.join("\n")
    };
    let memory = if context.memory_context.is_empty() {
        "none".into()
    } else {
        context.memory_context.join("\n")
    };
    let rag = if context.rag_context.is_empty() {
        "none".into()
    } else {
        context.rag_context.join("\n")
    };
    let compression_notice = context
        .context_continuation_prompt
        .as_ref()
        .map(|prompt| format!("\n\nContext compression notice:\n{prompt}"))
        .unwrap_or_default();
    redact_secrets(&format!(
        "{}\n\nLocal relationship context:\n{}\n\nLocal reference context:\n{}\n\nLocal chat history context:\n{}\n\nLocal memory context:\n{}\n\nLocal RAG context:\n{}{}\n\nUse local context when relevant. Do not reveal secrets. Tool use and writes must remain behind the harness.",
        context.persona_context, relationship, references, history, memory, rag, compression_notice
    ))
}
