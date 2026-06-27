// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    chat::{
        context::{
            context_lookup_is_safe_read, extract_projection_context,
            extract_retrieved_memory_context, extract_working_memory_context,
        },
        types::{ChatContext, ChatRunOptions},
    },
    relationship_context_lines, relationship_snapshot_from_session,
};
use ikaros_core::{ResolvedAgentProfile, Result};
use ikaros_harness::{ExecutionSession, SkillRegistry};
use serde_json::json;

pub(super) async fn assemble_memory_context(
    context: &mut ChatContext,
    input: &str,
    agent: &ResolvedAgentProfile,
    session: &ExecutionSession,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
) -> Result<()> {
    if !agent.profile.memory_context || options.memory_limit == 0 {
        return Ok(());
    }
    let relationship = relationship_snapshot_from_session(
        session,
        registry,
        options.scope.as_deref(),
        options.memory_limit,
    )
    .await?;
    context.relationship = relationship_context_lines(&relationship, options.memory_limit);

    if context_lookup_is_safe_read(registry, "memory_projection") {
        let mut projection_input = json!({
            "user_scope": "default",
        });
        let mut projection_audit_input = json!({
            "user_scope": "default",
        });
        if let Some(scope) = &options.scope {
            projection_input["project_scope"] = json!(scope);
            projection_audit_input["project_scope"] = json!(scope);
        }
        let result = session
            .execute_read_skill_with_audit_input(
                registry,
                "memory_projection",
                projection_input,
                projection_audit_input,
            )
            .await?;
        if result.ok {
            context
                .memory_projection
                .extend(extract_projection_context(&result.output));
        }
    }

    if let Some(session_id) = &options.session_id
        && context_lookup_is_safe_read(registry, "working_memory_list")
    {
        let result = session
            .execute_read_skill_with_audit_input(
                registry,
                "working_memory_list",
                json!({
                    "session_id": session_id,
                    "limit": options.memory_limit,
                }),
                json!({
                    "session_id": "<redacted chat session>",
                    "limit": options.memory_limit,
                }),
            )
            .await?;
        if result.ok {
            context
                .working_memory
                .extend(extract_working_memory_context(
                    &result.output,
                    options.memory_limit,
                ));
        }
    }

    if options.memory_search_limit > 0 {
        let mut memory_input = json!({
            "query": input,
            "limit": options.memory_search_limit,
        });
        let mut memory_audit_input = json!({
            "query": "<redacted chat query>",
            "limit": options.memory_search_limit,
        });
        if let Some(scope) = &options.scope {
            memory_input["scope"] = json!(scope);
            memory_audit_input["scope"] = json!(scope);
        }
        let result = session
            .execute_read_skill_with_audit_input(
                registry,
                "memory_search",
                memory_input,
                memory_audit_input,
            )
            .await?;
        if result.ok {
            context
                .retrieved_memory
                .extend(extract_retrieved_memory_context(
                    &result.output,
                    options.memory_search_limit,
                ));
        }
    }
    Ok(())
}
