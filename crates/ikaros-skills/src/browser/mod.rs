// SPDX-License-Identifier: GPL-3.0-only

use async_trait::async_trait;
use ikaros_core::{Result, RiskLevel, redact_secrets};
use ikaros_toolkit::{PolicyRequest, Skill, SkillContext, SkillOutput};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

mod actions;
mod cdp;
mod http;
mod util;

use actions::{
    BrowserHttpAction, BrowserWsAction, browser_action_is_write, browser_action_name,
    browser_input_schema, browser_ws_action_is_write, browser_ws_action_name,
    browser_ws_input_schema,
};
use cdp::execute_browser_ws_action;
use http::execute_browser_http_action;
use util::{browser_endpoint, browser_policy_command, browser_ws_policy_command};

const DEFAULT_CDP_ENDPOINT: &str = "http://127.0.0.1:9222";

#[derive(Debug, Clone)]
pub struct BrowserStatusSkill;

#[derive(Debug, Clone)]
pub struct BrowserListSkill;

#[derive(Debug, Clone)]
pub struct BrowserNewTargetSkill;

#[derive(Debug, Clone)]
pub struct BrowserActivateTargetSkill;

#[derive(Debug, Clone)]
pub struct BrowserCloseTargetSkill;

#[derive(Debug, Clone)]
pub struct BrowserNavigateSkill;

#[derive(Debug, Clone)]
pub struct BrowserSnapshotSkill;

#[derive(Debug, Clone)]
pub struct BrowserClickSkill;

#[derive(Debug, Clone)]
pub struct BrowserTypeSkill;

#[derive(Debug, Clone)]
pub struct BrowserScrollSkill;

#[derive(Debug, Clone)]
pub struct BrowserScreenshotSkill;

#[derive(Debug, Clone)]
pub struct BrowserCdpSkill;

macro_rules! impl_browser_http_skill {
    ($ty:ident, $name:literal, $description:literal, $action:expr) => {
        #[async_trait]
        impl Skill for $ty {
            fn name(&self) -> &'static str {
                $name
            }

            fn description(&self) -> &'static str {
                $description
            }

            fn input_schema(&self) -> Value {
                browser_input_schema($action)
            }

            fn risk_level(&self) -> RiskLevel {
                RiskLevel::Network
            }

            fn policy_request(
                &self,
                input: &Value,
                _workspace_root: &Path,
            ) -> PolicyRequest {
                PolicyRequest {
                    action: self.name().into(),
                    risk: RiskLevel::Network,
                    path: None,
                    command: Some(browser_policy_command($action, input)),
                    is_write: browser_action_is_write($action),
                }
            }

            fn approval_context(
                &self,
                input: &Value,
                _workspace_root: &Path,
            ) -> Option<Value> {
                Some(json!({
                    "kind": "browser_cdp_http",
                    "skill": self.name(),
                    "action": browser_action_name($action),
                    "endpoint": browser_endpoint(input),
                    "target_id": input.get("target_id").and_then(Value::as_str).map(redact_secrets),
                    "url": input.get("url").and_then(Value::as_str).map(redact_secrets),
                    "network_egress": true,
                }))
            }

            async fn execute(&self, input: Value, ctx: SkillContext) -> Result<SkillOutput> {
                execute_browser_http_action($action, input, ctx).await
            }
        }
    };
}

impl_browser_http_skill!(
    BrowserStatusSkill,
    "browser_status",
    "Read Chrome DevTools Protocol version/status from a governed local or remote browser endpoint.",
    BrowserHttpAction::Status
);
impl_browser_http_skill!(
    BrowserListSkill,
    "browser_list",
    "List Chrome DevTools Protocol targets from a governed browser endpoint.",
    BrowserHttpAction::List
);
impl_browser_http_skill!(
    BrowserNewTargetSkill,
    "browser_new_target",
    "Create a new browser target through a governed Chrome DevTools Protocol endpoint.",
    BrowserHttpAction::New
);
impl_browser_http_skill!(
    BrowserActivateTargetSkill,
    "browser_activate_target",
    "Activate an existing browser target through a governed Chrome DevTools Protocol endpoint.",
    BrowserHttpAction::Activate
);
impl_browser_http_skill!(
    BrowserCloseTargetSkill,
    "browser_close_target",
    "Close an existing browser target through a governed Chrome DevTools Protocol endpoint.",
    BrowserHttpAction::Close
);

macro_rules! impl_browser_ws_skill {
    ($ty:ident, $name:literal, $description:literal, $action:expr) => {
        #[async_trait]
        impl Skill for $ty {
            fn name(&self) -> &'static str {
                $name
            }

            fn description(&self) -> &'static str {
                $description
            }

            fn input_schema(&self) -> Value {
                browser_ws_input_schema($action)
            }

            fn risk_level(&self) -> RiskLevel {
                RiskLevel::Network
            }

            fn policy_request(
                &self,
                input: &Value,
                _workspace_root: &Path,
            ) -> PolicyRequest {
                PolicyRequest {
                    action: self.name().into(),
                    risk: RiskLevel::Network,
                    path: input
                        .get("output_path")
                        .and_then(Value::as_str)
                        .map(PathBuf::from),
                    command: Some(browser_ws_policy_command($action, input)),
                    is_write: browser_ws_action_is_write($action)
                        || input.get("output_path").and_then(Value::as_str).is_some(),
                }
            }

            fn approval_context(
                &self,
                input: &Value,
                _workspace_root: &Path,
            ) -> Option<Value> {
                Some(json!({
                    "kind": "browser_cdp_websocket",
                    "skill": self.name(),
                    "action": browser_ws_action_name($action),
                    "endpoint": browser_endpoint(input),
                    "target_id": input.get("target_id").and_then(Value::as_str).map(redact_secrets),
                    "url": input.get("url").and_then(Value::as_str).map(redact_secrets),
                    "output_path": input.get("output_path").and_then(Value::as_str).map(redact_secrets),
                    "network_egress": true,
                }))
            }

            async fn execute(&self, input: Value, ctx: SkillContext) -> Result<SkillOutput> {
                execute_browser_ws_action($action, input, ctx).await
            }
        }
    };
}

impl_browser_ws_skill!(
    BrowserNavigateSkill,
    "browser_navigate",
    "Navigate an existing Chrome DevTools target to a URL through a governed browser skill.",
    BrowserWsAction::Navigate
);
impl_browser_ws_skill!(
    BrowserSnapshotSkill,
    "browser_snapshot",
    "Read a lightweight DOM snapshot from an existing Chrome DevTools target.",
    BrowserWsAction::Snapshot
);
impl_browser_ws_skill!(
    BrowserClickSkill,
    "browser_click",
    "Dispatch a mouse click in an existing Chrome DevTools target.",
    BrowserWsAction::Click
);
impl_browser_ws_skill!(
    BrowserTypeSkill,
    "browser_type",
    "Insert text into the focused element in an existing Chrome DevTools target.",
    BrowserWsAction::Type
);
impl_browser_ws_skill!(
    BrowserScrollSkill,
    "browser_scroll",
    "Dispatch a scroll gesture in an existing Chrome DevTools target.",
    BrowserWsAction::Scroll
);
impl_browser_ws_skill!(
    BrowserScreenshotSkill,
    "browser_screenshot",
    "Capture a screenshot from an existing Chrome DevTools target and optionally save it through ExecutionEnv.",
    BrowserWsAction::Screenshot
);
impl_browser_ws_skill!(
    BrowserCdpSkill,
    "browser_cdp",
    "Send one explicit Chrome DevTools Protocol command to an existing target.",
    BrowserWsAction::Cdp
);
