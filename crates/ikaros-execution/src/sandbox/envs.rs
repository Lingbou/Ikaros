// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[derive(Debug, Clone, Copy, Default)]
pub struct LocalExecutionEnv;

#[derive(Clone)]
pub struct DockerExecutionEnv {
    workspace_root: PathBuf,
    image: String,
    inner: Arc<dyn ExecutionEnv>,
}

#[derive(Clone)]
pub struct WorkspaceExecutionEnv {
    workspace_root: PathBuf,
    inner: Arc<dyn ExecutionEnv>,
}

#[derive(Clone)]
pub struct NetworkedExecutionEnv {
    inner: Arc<dyn ExecutionEnv>,
    network: Arc<dyn NetworkEgress>,
}

#[derive(Clone)]
pub struct DryRunExecutionEnv {
    inner: Arc<dyn ExecutionEnv>,
}

impl DockerExecutionEnv {
    pub fn new(workspace_root: impl Into<PathBuf>, image: impl Into<String>) -> Self {
        Self {
            workspace_root: normalize_path(&absolute_path(workspace_root.into())),
            image: image.into(),
            inner: Arc::new(LocalExecutionEnv),
        }
    }

    pub fn with_inner(
        workspace_root: impl Into<PathBuf>,
        image: impl Into<String>,
        inner: Arc<dyn ExecutionEnv>,
    ) -> Self {
        Self {
            workspace_root: normalize_path(&absolute_path(workspace_root.into())),
            image: image.into(),
            inner,
        }
    }
}

impl DryRunExecutionEnv {
    pub fn new(inner: Arc<dyn ExecutionEnv>) -> Self {
        Self { inner }
    }
}

impl NetworkedExecutionEnv {
    pub fn new(inner: Arc<dyn ExecutionEnv>, network: Arc<dyn NetworkEgress>) -> Self {
        Self { inner, network }
    }
}

impl WorkspaceExecutionEnv {
    pub fn new(workspace_root: impl Into<PathBuf>, inner: Arc<dyn ExecutionEnv>) -> Self {
        Self {
            workspace_root: normalize_path(&absolute_path(workspace_root.into())),
            inner,
        }
    }

    pub fn local(workspace_root: impl Into<PathBuf>) -> Self {
        Self::new(workspace_root, Arc::new(LocalExecutionEnv))
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    fn resolve_path(&self, path: &Path) -> PathBuf {
        normalize_path(&if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        })
    }

    fn ensure_lexically_in_workspace(&self, path: &Path) -> Result<PathBuf> {
        let resolved = self.resolve_path(path);
        if resolved.starts_with(&self.workspace_root) {
            return Ok(resolved);
        }
        #[cfg(windows)]
        {
            let canonical_workspace = fs::canonicalize(&self.workspace_root)
                .unwrap_or_else(|_| self.workspace_root.clone());
            if let Ok(canonical) = fs::canonicalize(&resolved)
                && canonical.starts_with(&canonical_workspace)
            {
                return Ok(resolved);
            }
        }
        Err(IkarosError::OutOfScope(resolved))
    }

    fn ensure_write_path(&self, path: &Path) -> Result<PathBuf> {
        let resolved = self.ensure_lexically_in_workspace(path)?;
        if let Ok(canonical) = fs::canonicalize(&resolved) {
            self.ensure_canonical_in_workspace(&resolved, &canonical)?;
            return Ok(resolved);
        }
        let parent = resolved
            .parent()
            .ok_or_else(|| IkarosError::OutOfScope(resolved.clone()))?;
        self.ensure_existing_anchor_in_workspace(parent)?;
        Ok(resolved)
    }

    fn ensure_create_dir_path(&self, path: &Path) -> Result<PathBuf> {
        let resolved = self.ensure_lexically_in_workspace(path)?;
        if let Ok(canonical) = fs::canonicalize(&resolved) {
            self.ensure_canonical_in_workspace(&resolved, &canonical)?;
            return Ok(resolved);
        }
        self.ensure_existing_anchor_in_workspace(&resolved)?;
        Ok(resolved)
    }

    fn ensure_existing_workspace_path(&self, path: &Path) -> Result<PathBuf> {
        let resolved = self.ensure_lexically_in_workspace(path)?;
        let canonical =
            fs::canonicalize(&resolved).map_err(|source| IkarosError::io(&resolved, source))?;
        self.ensure_canonical_in_workspace(&resolved, &canonical)?;
        Ok(resolved)
    }

    fn ensure_existing_anchor_in_workspace(&self, path: &Path) -> Result<()> {
        if let Ok(canonical) = fs::canonicalize(path) {
            self.ensure_canonical_in_workspace(path, &canonical)?;
            return Ok(());
        }
        let mut ancestor = path;
        while let Some(parent) = ancestor.parent() {
            if parent == self.workspace_root {
                return Ok(());
            }
            if parent.starts_with(&self.workspace_root)
                && let Ok(canonical) = fs::canonicalize(parent)
            {
                self.ensure_canonical_in_workspace(parent, &canonical)?;
                return Ok(());
            }
            ancestor = parent;
        }
        Err(IkarosError::OutOfScope(path.to_path_buf()))
    }

    fn ensure_canonical_in_workspace(&self, requested: &Path, canonical: &Path) -> Result<()> {
        let canonical_workspace =
            fs::canonicalize(&self.workspace_root).unwrap_or_else(|_| self.workspace_root.clone());
        if !canonical.starts_with(&canonical_workspace) {
            return Err(IkarosError::OutOfScope(requested.to_path_buf()));
        }
        Ok(())
    }

    fn ensure_plugin_process_request(&self, request: &ProcessRequest) -> Result<PathBuf> {
        if request.use_shell {
            return Err(IkarosError::OutOfScope(request.cwd.clone()));
        }
        let cwd = fs::canonicalize(&request.cwd)
            .map_err(|source| IkarosError::io(&request.cwd, source))?;
        if !cwd.is_dir() {
            return Err(IkarosError::OutOfScope(request.cwd.clone()));
        }
        let command = Path::new(&request.command);
        let command = if command.is_absolute() {
            command.to_path_buf()
        } else {
            cwd.join(command)
        };
        let command =
            fs::canonicalize(&command).map_err(|source| IkarosError::io(&command, source))?;
        if !command.starts_with(&cwd) {
            return Err(IkarosError::OutOfScope(command));
        }
        Ok(cwd)
    }
}
impl FileSystem for WorkspaceExecutionEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>> {
        Box::pin(async move {
            let resolved = self.ensure_existing_workspace_path(path)?;
            self.inner.path_metadata(&resolved).await
        })
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            let resolved = self.ensure_existing_workspace_path(path)?;
            self.inner.read_to_string(&resolved).await
        })
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        Box::pin(async move {
            let resolved = self.ensure_existing_workspace_path(path)?;
            self.inner.read_bytes(&resolved).await
        })
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let resolved = self.ensure_write_path(path)?;
            self.inner.write_string(&resolved, content).await
        })
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let resolved = self.ensure_write_path(path)?;
            self.inner.write_bytes(&resolved, content).await
        })
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let resolved = self.ensure_create_dir_path(path)?;
            self.inner.create_dir_all(&resolved).await
        })
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        Box::pin(async move {
            let resolved = self.ensure_existing_workspace_path(path)?;
            self.inner.read_dir(&resolved).await
        })
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let resolved = self.ensure_existing_workspace_path(path)?;
            self.inner.remove_file(&resolved).await
        })
    }
}

impl ProcessRunner for WorkspaceExecutionEnv {
    fn run_process<'a>(
        &'a self,
        mut request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        Box::pin(async move {
            request.cwd = match request.cwd_scope {
                ProcessCwdScope::Workspace => self.ensure_existing_workspace_path(&request.cwd)?,
                ProcessCwdScope::Plugin => self.ensure_plugin_process_request(&request)?,
            };
            self.inner.run_process(request).await
        })
    }
}

impl NetworkEgress for WorkspaceExecutionEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        self.inner.send_network_request(request)
    }
}

impl ExecutionEnv for WorkspaceExecutionEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        context: SkillContext,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        self.inner.execute_skill(skill, input, context)
    }
}

impl FileSystem for NetworkedExecutionEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>> {
        self.inner.path_metadata(path)
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        self.inner.read_to_string(path)
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        self.inner.read_bytes(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.inner.write_string(path, content)
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.inner.write_bytes(path, content)
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.inner.create_dir_all(path)
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        self.inner.read_dir(path)
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.inner.remove_file(path)
    }
}

impl ProcessRunner for NetworkedExecutionEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        self.inner.run_process(request)
    }
}

impl NetworkEgress for NetworkedExecutionEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        self.network.send_network_request(request)
    }
}

impl ExecutionEnv for NetworkedExecutionEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        context: SkillContext,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        self.inner.execute_skill(skill, input, context)
    }
}

impl FileSystem for DryRunExecutionEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>> {
        self.inner.path_metadata(path)
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        self.inner.read_to_string(path)
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        self.inner.read_bytes(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        _content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let path = path.to_path_buf();
        Box::pin(async move {
            let _ = path;
            Ok(())
        })
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        _content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let path = path.to_path_buf();
        Box::pin(async move {
            let _ = path;
            Ok(())
        })
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let path = path.to_path_buf();
        Box::pin(async move {
            let _ = path;
            Ok(())
        })
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        self.inner.read_dir(path)
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let path = path.to_path_buf();
        Box::pin(async move {
            let _ = path;
            Ok(())
        })
    }
}

impl ProcessRunner for DryRunExecutionEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        Box::pin(async move {
            Ok(ProcessOutput {
                status: 0,
                stdout: format!("dry-run: skipped command {}", request.command),
                stderr: String::new(),
            })
        })
    }
}

impl NetworkEgress for DryRunExecutionEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        Box::pin(async move {
            Ok(NetworkEgressResponse {
                status: 200,
                headers: Default::default(),
                body: format!(
                    "{{\"dry_run\":true,\"url\":{}}}",
                    serde_json::to_string(&request.url)?
                ),
                body_bytes: None,
            })
        })
    }
}

impl ExecutionEnv for DryRunExecutionEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        context: SkillContext,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        self.inner.execute_skill(skill, input, context)
    }
}

impl FileSystem for DockerExecutionEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>> {
        self.inner.path_metadata(path)
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        self.inner.read_to_string(path)
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        self.inner.read_bytes(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.inner.write_string(path, content)
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.inner.write_bytes(path, content)
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.inner.create_dir_all(path)
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        self.inner.read_dir(path)
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.inner.remove_file(path)
    }
}

impl ProcessRunner for DockerExecutionEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        Box::pin(async move {
            let docker_request =
                docker_process_request(&self.workspace_root, &self.image, request)?;
            LocalExecutionEnv.run_process(docker_request).await
        })
    }
}

impl NetworkEgress for DockerExecutionEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        self.inner.send_network_request(request)
    }
}

impl ExecutionEnv for DockerExecutionEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        context: SkillContext,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        self.inner.execute_skill(skill, input, context)
    }
}
impl ExecutionEnv for LocalExecutionEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        context: SkillContext,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        Box::pin(async move { skill.execute(input, context).await })
    }
}
