// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::redact_secrets;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuardrailConfig {
    pub warnings_enabled: bool,
    pub hard_stop_enabled: bool,
    pub exact_failure_warn_after: u32,
    pub exact_failure_halt_after: u32,
    pub same_tool_failure_warn_after: u32,
    pub same_tool_failure_halt_after: u32,
    pub no_progress_warn_after: u32,
    pub no_progress_halt_after: u32,
}

impl Default for GuardrailConfig {
    fn default() -> Self {
        Self {
            warnings_enabled: true,
            hard_stop_enabled: false,
            exact_failure_warn_after: 3,
            exact_failure_halt_after: 5,
            same_tool_failure_warn_after: 3,
            same_tool_failure_halt_after: 5,
            no_progress_warn_after: 3,
            no_progress_halt_after: 5,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuardrailState {
    exact_failures: BTreeMap<String, u32>,
    same_tool_failure: Option<ConsecutiveCounter>,
    no_progress: Option<ConsecutiveCounter>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ConsecutiveCounter {
    skill: String,
    count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuardrailObservation {
    pub skill: String,
    pub ok: bool,
    pub summary: String,
    pub no_progress: bool,
}

impl GuardrailObservation {
    pub fn tool(skill: &str, ok: bool, summary: &str, output: &serde_json::Value) -> Self {
        Self {
            skill: redact_secrets(skill),
            ok,
            summary: redact_secrets(summary),
            no_progress: output_marks_no_progress(output),
        }
    }

    pub fn failure(skill: &str, summary: &str) -> Self {
        Self {
            skill: redact_secrets(skill),
            ok: false,
            summary: redact_secrets(summary),
            no_progress: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum GuardrailDecision {
    Continue,
    Warn(GuardrailSignal),
    Halt(GuardrailSignal),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuardrailSignal {
    pub kind: GuardrailSignalKind,
    pub skill: String,
    pub count: u32,
    pub threshold: u32,
    pub summary: String,
}

impl GuardrailSignal {
    pub fn message(&self) -> String {
        format!(
            "{:?} guardrail for {} reached {}/{}: {}",
            self.kind, self.skill, self.count, self.threshold, self.summary
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum GuardrailSignalKind {
    ExactFailure,
    SameToolFailure,
    NoProgress,
}

impl GuardrailState {
    pub fn observe(
        &mut self,
        config: &GuardrailConfig,
        observation: &GuardrailObservation,
    ) -> GuardrailDecision {
        let mut signals = Vec::new();
        if !observation.ok {
            signals.extend(self.observe_failure(observation));
        } else {
            self.same_tool_failure = None;
        }

        if observation.no_progress {
            signals.push(update_counter(
                &mut self.no_progress,
                &observation.skill,
                GuardrailSignalKind::NoProgress,
                observation,
            ));
        } else {
            self.no_progress = None;
        }

        choose_decision(config, signals)
    }

    fn observe_failure(&mut self, observation: &GuardrailObservation) -> Vec<GuardrailSignal> {
        let exact_key = format!("{}\n{}", observation.skill, observation.summary);
        let exact_count = self.exact_failures.entry(exact_key).or_insert(0);
        *exact_count += 1;
        let exact = GuardrailSignal {
            kind: GuardrailSignalKind::ExactFailure,
            skill: observation.skill.clone(),
            count: *exact_count,
            threshold: 0,
            summary: observation.summary.clone(),
        };
        let same_tool = update_counter(
            &mut self.same_tool_failure,
            &observation.skill,
            GuardrailSignalKind::SameToolFailure,
            observation,
        );
        vec![exact, same_tool]
    }
}

fn update_counter(
    counter: &mut Option<ConsecutiveCounter>,
    skill: &str,
    kind: GuardrailSignalKind,
    observation: &GuardrailObservation,
) -> GuardrailSignal {
    match counter {
        Some(current) if current.skill == skill => current.count += 1,
        _ => {
            *counter = Some(ConsecutiveCounter {
                skill: skill.into(),
                count: 1,
            });
        }
    }
    let count = counter.as_ref().map(|current| current.count).unwrap_or(1);
    GuardrailSignal {
        kind,
        skill: observation.skill.clone(),
        count,
        threshold: 0,
        summary: observation.summary.clone(),
    }
}

fn choose_decision(config: &GuardrailConfig, signals: Vec<GuardrailSignal>) -> GuardrailDecision {
    if config.hard_stop_enabled {
        if let Some(signal) = first_threshold_signal(&signals, config, ThresholdMode::Halt) {
            return GuardrailDecision::Halt(signal);
        }
    }
    if config.warnings_enabled {
        if let Some(signal) = first_threshold_signal(&signals, config, ThresholdMode::Warn) {
            return GuardrailDecision::Warn(signal);
        }
    }
    GuardrailDecision::Continue
}

fn first_threshold_signal(
    signals: &[GuardrailSignal],
    config: &GuardrailConfig,
    mode: ThresholdMode,
) -> Option<GuardrailSignal> {
    signals.iter().find_map(|signal| {
        let threshold = threshold_for(config, &signal.kind, mode);
        if threshold > 0 && signal.count >= threshold {
            let mut signal = signal.clone();
            signal.threshold = threshold;
            Some(signal)
        } else {
            None
        }
    })
}

#[derive(Debug, Clone, Copy)]
enum ThresholdMode {
    Warn,
    Halt,
}

fn threshold_for(config: &GuardrailConfig, kind: &GuardrailSignalKind, mode: ThresholdMode) -> u32 {
    match (kind, mode) {
        (GuardrailSignalKind::ExactFailure, ThresholdMode::Warn) => config.exact_failure_warn_after,
        (GuardrailSignalKind::ExactFailure, ThresholdMode::Halt) => config.exact_failure_halt_after,
        (GuardrailSignalKind::SameToolFailure, ThresholdMode::Warn) => {
            config.same_tool_failure_warn_after
        }
        (GuardrailSignalKind::SameToolFailure, ThresholdMode::Halt) => {
            config.same_tool_failure_halt_after
        }
        (GuardrailSignalKind::NoProgress, ThresholdMode::Warn) => config.no_progress_warn_after,
        (GuardrailSignalKind::NoProgress, ThresholdMode::Halt) => config.no_progress_halt_after,
    }
}

fn output_marks_no_progress(output: &serde_json::Value) -> bool {
    output
        .get("no_progress")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        || output
            .get("progress")
            .and_then(serde_json::Value::as_bool)
            .is_some_and(|progress| !progress)
}
