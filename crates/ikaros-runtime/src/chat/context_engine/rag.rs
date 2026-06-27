// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::{
    context::{context_lookup_is_safe_read, extract_rag_context},
    types::{ChatContext, ChatRunOptions},
};
use ikaros_core::{ResolvedAgentProfile, Result};
use ikaros_harness::{ExecutionSession, SkillRegistry};
use serde_json::json;

pub(super) async fn assemble_rag_context(
    context: &mut ChatContext,
    input: &str,
    agent: &ResolvedAgentProfile,
    session: &ExecutionSession,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
) -> Result<()> {
    if !agent.profile.rag_context
        || options.rag_top_k == 0
        || !context_lookup_is_safe_read(registry, "rag_search")
    {
        return Ok(());
    }
    let mut rag_input = json!({
        "query": input,
        "top_k": options.rag_top_k,
    });
    let mut rag_audit_input = json!({
        "query": "<redacted chat query>",
        "top_k": options.rag_top_k,
    });
    if let Some(scope) = &options.scope {
        rag_input["scope"] = json!(scope);
        rag_audit_input["scope"] = json!(scope);
    }
    let result = session
        .execute_read_skill_with_audit_input(registry, "rag_search", rag_input, rag_audit_input)
        .await?;
    if result.ok {
        context.rag = extract_rag_context(&result.output, options.rag_top_k);
    }
    Ok(())
}
