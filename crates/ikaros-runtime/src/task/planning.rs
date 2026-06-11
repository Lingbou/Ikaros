// SPDX-License-Identifier: GPL-3.0-only

use super::RuntimeTaskPlan;
use ikaros_core::{Plan, PlanStep, RiskLevel};
use ikaros_harness::ExecutablePlanStep;
use serde_json::json;

pub fn task_steps(task_text: &str, task_title: &str, task_id: &str) -> Vec<ExecutablePlanStep> {
    build_task_plan(task_text, task_title, task_id).executable_steps
}

pub fn build_task_plan(task_text: &str, task_title: &str, task_id: &str) -> RuntimeTaskPlan {
    let steps = vec![
        planned_skill_step(
            "Search local memory for task context.",
            "memory_search",
            json!({"query": task_text, "limit": 3}),
            RiskLevel::SafeRead,
        ),
        planned_skill_step(
            "Search local RAG index for task context.",
            "rag_search",
            json!({"query": task_text, "top_k": 3}),
            RiskLevel::SafeRead,
        ),
        planned_skill_step(
            "Summarize the task through a safe harness skill.",
            "task_summarize",
            json!({"task": task_title}),
            RiskLevel::SafeRead,
        ),
        planned_skill_step(
            "Persist safe task completion memory.",
            "memory_append",
            task_memory_input(task_title, task_id),
            RiskLevel::DatabaseWrite,
        ),
    ];
    let (plan_steps, executable_steps): (Vec<_>, Vec<_>) = steps.into_iter().unzip();
    RuntimeTaskPlan {
        plan: Plan {
            task_id: task_id.into(),
            steps: plan_steps,
        },
        executable_steps,
    }
}

fn planned_skill_step(
    description: impl Into<String>,
    skill: impl Into<String>,
    input: serde_json::Value,
    risk: RiskLevel,
) -> (PlanStep, ExecutablePlanStep) {
    let step = ExecutablePlanStep::new(description, skill, input, risk);
    let plan_step = PlanStep {
        id: step.id.clone(),
        description: step.description.clone(),
        risk: step.risk.clone(),
        tool: Some(step.skill.clone()),
    };
    (plan_step, step)
}

fn task_memory_input(task_title: &str, task_id: &str) -> serde_json::Value {
    json!({
        "kind": "task",
        "scope": task_id,
        "content": format!("Task completed: {task_title}"),
        "tags": ["task_run", "completed"],
    })
}
