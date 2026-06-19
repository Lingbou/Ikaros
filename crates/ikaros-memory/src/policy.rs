// SPDX-License-Identifier: GPL-3.0-only

use crate::{MemoryJournalAction, MemoryKind, MemoryPolicy, MemoryRecord, MemoryScore};
use std::{cmp::Ordering, collections::HashSet};

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryPolicyDecision {
    pub action: MemoryJournalAction,
    pub score: MemoryScore,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct MemoryPolicyEngine {
    policy: MemoryPolicy,
}

impl MemoryPolicyEngine {
    pub fn new(policy: MemoryPolicy) -> Self {
        Self { policy }
    }

    pub fn policy(&self) -> &MemoryPolicy {
        &self.policy
    }

    pub fn score_record(
        &self,
        record: &MemoryRecord,
        scope_records: &[MemoryRecord],
    ) -> MemoryScore {
        MemoryScore {
            recency: recency_score(record, scope_records),
            relevance: relevance_score(record),
            frequency: frequency_score(record, scope_records),
            source_strength: source_strength_score(record),
            confidence: confidence_score(record),
            sensitivity: sensitivity_score(record),
        }
    }

    pub fn classify_record(
        &self,
        record: &MemoryRecord,
        scope_records: &[MemoryRecord],
    ) -> Option<MemoryPolicyDecision> {
        let score = self.score_record(record, scope_records);
        let combined = score.combined();
        if combined <= self.policy.forget_threshold {
            return Some(MemoryPolicyDecision {
                action: MemoryJournalAction::Forget,
                score,
                reason: "policy score below forget threshold".into(),
            });
        }
        if combined <= self.policy.demote_threshold && !has_policy_tag(record, "policy-demoted") {
            return Some(MemoryPolicyDecision {
                action: MemoryJournalAction::Demote,
                score,
                reason: "policy score below demote threshold".into(),
            });
        }
        if combined >= self.policy.promote_threshold && !has_policy_tag(record, "policy-promoted") {
            return Some(MemoryPolicyDecision {
                action: MemoryJournalAction::Promote,
                score,
                reason: "policy score reached promote threshold".into(),
            });
        }
        None
    }

    pub fn quota_victims(
        &self,
        scope_records: &[MemoryRecord],
    ) -> Vec<(MemoryRecord, MemoryScore)> {
        let overflow = scope_records
            .len()
            .saturating_sub(self.policy.max_records_per_scope);
        if overflow == 0 {
            return Vec::new();
        }

        let mut scored = scope_records
            .iter()
            .cloned()
            .map(|record| {
                let score = self.score_record(&record, scope_records);
                (record, score)
            })
            .collect::<Vec<_>>();
        scored.sort_by(|(left_record, left_score), (right_record, right_score)| {
            left_score
                .combined()
                .partial_cmp(&right_score.combined())
                .unwrap_or(Ordering::Equal)
                .then_with(|| left_record.created_at.cmp(&right_record.created_at))
        });
        scored.into_iter().take(overflow).collect()
    }
}

impl Default for MemoryPolicyEngine {
    fn default() -> Self {
        Self::new(MemoryPolicy::default())
    }
}

pub fn add_policy_tag(tags: &[String], tag: &str) -> Vec<String> {
    let mut next = tags.to_vec();
    if !next.iter().any(|existing| existing == tag) {
        next.push(tag.to_owned());
    }
    next
}

pub fn has_policy_tag(record: &MemoryRecord, tag: &str) -> bool {
    record.tags.iter().any(|existing| existing == tag)
}

fn recency_score(record: &MemoryRecord, scope_records: &[MemoryRecord]) -> f32 {
    if scope_records.len() <= 1 {
        return 1.0;
    }
    let mut ordered = scope_records.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        effective_timestamp(left)
            .cmp(effective_timestamp(right))
            .then_with(|| left.id.cmp(&right.id))
    });
    let Some(index) = ordered
        .iter()
        .position(|candidate| candidate.id == record.id)
    else {
        return 0.5;
    };
    if ordered.len() == 1 {
        1.0
    } else {
        index as f32 / (ordered.len().saturating_sub(1)) as f32
    }
}

fn relevance_score(record: &MemoryRecord) -> f32 {
    let lower = record.content.to_ascii_lowercase();
    let mut score: f32 = match record.kind {
        MemoryKind::Relationship => 0.85,
        MemoryKind::User | MemoryKind::Project | MemoryKind::Knowledge => 0.65,
        MemoryKind::Persona => 0.60,
        MemoryKind::Task => 0.45,
    };
    if record.tags.iter().any(|tag| {
        matches!(
            tag.as_str(),
            "relationship" | "chat-learned" | "turn-summary" | "memory-lifecycle"
        )
    }) {
        score += 0.08;
    }
    if [
        "preference",
        "preferred name",
        "remember",
        "asked ikaros to remember",
        "用户",
        "偏好",
        "记住",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        score += 0.12;
    }
    if record.content.chars().count() < 12 {
        score -= 0.20;
    }
    score.clamp(0.0, 1.0)
}

fn frequency_score(record: &MemoryRecord, scope_records: &[MemoryRecord]) -> f32 {
    let current_terms = normalized_terms(&record.content);
    if current_terms.is_empty() {
        return 0.0;
    }
    let similar = scope_records
        .iter()
        .filter(|candidate| candidate.id != record.id)
        .filter(|candidate| candidate.kind == record.kind && candidate.scope == record.scope)
        .filter(|candidate| jaccard(&current_terms, &normalized_terms(&candidate.content)) >= 0.45)
        .count();
    match similar {
        0 => 0.0,
        1 => 0.55,
        2 => 0.75,
        _ => 1.0,
    }
}

fn source_strength_score(record: &MemoryRecord) -> f32 {
    let mut score: f32 = match record.source.as_deref() {
        Some("memory_lifecycle") => 0.65,
        Some("manual") => 0.80,
        Some(_) => 0.55,
        None => 0.45,
    };
    if record.source_ref.is_some() {
        score += 0.15;
    }
    if record.tags.iter().any(|tag| tag == "chat-learned") {
        score += 0.10;
    }
    score.clamp(0.0, 1.0)
}

fn confidence_score(record: &MemoryRecord) -> f32 {
    record
        .confidence
        .unwrap_or(match record.kind {
            MemoryKind::Relationship => 0.85,
            MemoryKind::User | MemoryKind::Project | MemoryKind::Knowledge => 0.65,
            MemoryKind::Persona => 0.60,
            MemoryKind::Task => 0.55,
        })
        .clamp(0.0, 1.0)
}

fn sensitivity_score(record: &MemoryRecord) -> f32 {
    if record.sensitive { 1.0 } else { 0.0 }
}

fn effective_timestamp(record: &MemoryRecord) -> &str {
    record.updated_at.as_deref().unwrap_or(&record.created_at)
}

fn normalized_terms(value: &str) -> HashSet<String> {
    value
        .split(|ch: char| !ch.is_alphanumeric())
        .map(str::to_ascii_lowercase)
        .filter(|term| term.chars().count() >= 3)
        .collect()
}

fn jaccard(left: &HashSet<String>, right: &HashSet<String>) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(right).count();
    let union = left.union(right).count();
    if union == 0 {
        0.0
    } else {
        intersection as f32 / union as f32
    }
}
