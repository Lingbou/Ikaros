// SPDX-License-Identifier: GPL-3.0-only

#![cfg(test)]

pub(super) use crate::*;
pub(super) use async_trait::async_trait;
pub(super) use ikaros_core::{AgentProfile, PolicyDecision, ResolvedAgentProfile, RiskLevel};
pub(super) use serde_json::json;
#[cfg(unix)]
pub(super) use std::os::unix::fs::PermissionsExt;
pub(super) use std::{
    collections::BTreeMap,
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(super) struct InterceptEnv {
    calls: Arc<AtomicUsize>,
}

#[cfg(unix)]
pub(super) struct SwapSymlinkOnWriteEnv {
    outside_target: PathBuf,
}

#[cfg(unix)]
impl FileSystem for SwapSymlinkOnWriteEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>> {
        LocalExecutionEnv.path_metadata(path)
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        LocalExecutionEnv.read_to_string(path)
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        LocalExecutionEnv.read_bytes(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            fs::remove_file(path).map_err(|source| IkarosError::io(path, source))?;
            std::os::unix::fs::symlink(&self.outside_target, path)
                .map_err(|source| IkarosError::io(path, source))?;
            LocalExecutionEnv.write_string(path, content).await
        })
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            fs::remove_file(path).map_err(|source| IkarosError::io(path, source))?;
            std::os::unix::fs::symlink(&self.outside_target, path)
                .map_err(|source| IkarosError::io(path, source))?;
            LocalExecutionEnv.write_bytes(path, content).await
        })
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.create_dir_all(path)
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        LocalExecutionEnv.read_dir(path)
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.remove_file(path)
    }
}

#[cfg(unix)]
impl ProcessRunner for SwapSymlinkOnWriteEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        LocalExecutionEnv.run_process(request)
    }
}

#[cfg(unix)]
impl NetworkEgress for SwapSymlinkOnWriteEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        LocalExecutionEnv.send_network_request(request)
    }
}

#[cfg(unix)]
impl ExecutionEnv for SwapSymlinkOnWriteEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        context: SkillContext,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        LocalExecutionEnv.execute_skill(skill, input, context)
    }
}

impl FileSystem for InterceptEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>> {
        LocalExecutionEnv.path_metadata(path)
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        LocalExecutionEnv.read_to_string(path)
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        LocalExecutionEnv.read_bytes(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.write_string(path, content)
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.write_bytes(path, content)
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.create_dir_all(path)
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        LocalExecutionEnv.read_dir(path)
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.remove_file(path)
    }
}

impl ProcessRunner for InterceptEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        LocalExecutionEnv.run_process(request)
    }
}

impl NetworkEgress for InterceptEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        LocalExecutionEnv.send_network_request(request)
    }
}

impl ExecutionEnv for InterceptEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        _context: SkillContext,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        let calls = self.calls.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(SkillOutput::new(
                format!("env executed {}", skill.name()),
                json!({"via_env": true, "input": input}),
            ))
        })
    }
}

pub(super) struct CoreReadSkill;
pub(super) struct RagDeferredSkill;
pub(super) struct HiddenExecutableSkill;

#[async_trait]
impl Skill for CoreReadSkill {
    fn name(&self) -> &'static str {
        "core_read_test"
    }

    fn description(&self) -> &'static str {
        "Core read test skill."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        Ok(SkillOutput::new("ok", json!({})))
    }
}

#[async_trait]
impl Skill for RagDeferredSkill {
    fn name(&self) -> &'static str {
        "rag_deferred_test"
    }

    fn description(&self) -> &'static str {
        "RAG deferred test skill."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        Ok(SkillOutput::new("ok", json!({})))
    }
}

#[async_trait]
impl Skill for HiddenExecutableSkill {
    fn name(&self) -> &'static str {
        "hidden_executable_test"
    }

    fn description(&self) -> &'static str {
        "Hidden executable test skill."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    fn descriptor(&self) -> SkillDescriptor {
        let mut descriptor = SkillDescriptor::from_skill(self);
        descriptor.disable_model_invocation = true;
        descriptor
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        Ok(SkillOutput::new("ok", json!({})))
    }
}

pub(super) fn audit_event_at(kind: &str, at: &str) -> AuditEvent {
    let mut event =
        AuditEvent::new(kind, None, format!("{kind} audit event"), json!({})).expect("audit event");
    event.at = at.into();
    event
}

pub(super) fn compressed_audit_archives(dir: &Path) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(dir)
        .expect("read dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "gz"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

mod registry;

pub(super) struct CountingNetworkTransport {
    pub(super) calls: Arc<AtomicUsize>,
}

impl NetworkEgress for CountingNetworkTransport {
    fn send_network_request<'a>(
        &'a self,
        _request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        let calls = self.calls.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(NetworkEgressResponse {
                status: 200,
                headers: Default::default(),
                body: "{\"ok\":true}".into(),
                body_bytes: None,
            })
        })
    }
}
mod approval;
mod audit;
mod execution_session;
mod local_env;
mod network;
mod sandbox_rotation;
