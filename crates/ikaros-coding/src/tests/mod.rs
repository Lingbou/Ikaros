// SPDX-License-Identifier: GPL-3.0-only

#![cfg(test)]

pub(super) use super::*;

pub(super) use ikaros_core::{IkarosError, SelfModifyCheckProfileConfig, SelfModifyConfig};
pub(super) use ikaros_sandbox::{
    FileMetadata, FileSystem as ExecutionFileSystem, LocalExecutionEnv, ProcessOutput,
    ProcessRequest, ProcessRunner,
};
pub(super) use std::collections::BTreeMap;
#[cfg(unix)]
pub(super) use std::os::unix::fs::symlink;
pub(super) use std::{
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(super) fn canonical_or_original(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[derive(Debug, Default)]
pub(super) struct SelfModifyTrackingEnv {
    read_to_string_calls: Arc<AtomicUsize>,
    read_bytes_calls: Arc<AtomicUsize>,
    write_string_calls: Arc<AtomicUsize>,
    write_bytes_calls: Arc<AtomicUsize>,
    remove_file_calls: Arc<AtomicUsize>,
    process_calls: Arc<AtomicUsize>,
}

impl SelfModifyTrackingEnv {
    fn read_bytes_count(&self) -> usize {
        self.read_bytes_calls.load(Ordering::SeqCst)
    }

    fn write_string_count(&self) -> usize {
        self.write_string_calls.load(Ordering::SeqCst)
    }

    fn write_bytes_count(&self) -> usize {
        self.write_bytes_calls.load(Ordering::SeqCst)
    }

    fn remove_file_count(&self) -> usize {
        self.remove_file_calls.load(Ordering::SeqCst)
    }

    fn process_count(&self) -> usize {
        self.process_calls.load(Ordering::SeqCst)
    }
}

impl ExecutionFileSystem for SelfModifyTrackingEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<FileMetadata>> + Send + 'a>> {
        LocalExecutionEnv.path_metadata(path)
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<String>> + Send + 'a>> {
        self.read_to_string_calls.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.read_to_string(path)
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<Vec<u8>>> + Send + 'a>> {
        self.read_bytes_calls.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.read_bytes(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<()>> + Send + 'a>> {
        self.write_string_calls.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.write_string(path, content)
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<()>> + Send + 'a>> {
        self.write_bytes_calls.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.write_bytes(path, content)
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<()>> + Send + 'a>> {
        LocalExecutionEnv.create_dir_all(path)
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<Vec<String>>> + Send + 'a>> {
        LocalExecutionEnv.read_dir(path)
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<()>> + Send + 'a>> {
        self.remove_file_calls.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.remove_file(path)
    }
}

#[derive(Debug)]
pub(super) struct FailingRemoveFileSystem {
    fail_path: PathBuf,
    failures_left: Arc<AtomicUsize>,
}

impl FailingRemoveFileSystem {
    fn new(fail_path: PathBuf) -> Self {
        Self {
            fail_path: canonical_or_original(&fail_path),
            failures_left: Arc::new(AtomicUsize::new(1)),
        }
    }
}

impl ExecutionFileSystem for FailingRemoveFileSystem {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<FileMetadata>> + Send + 'a>> {
        LocalExecutionEnv.path_metadata(path)
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<String>> + Send + 'a>> {
        LocalExecutionEnv.read_to_string(path)
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<Vec<u8>>> + Send + 'a>> {
        LocalExecutionEnv.read_bytes(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<()>> + Send + 'a>> {
        LocalExecutionEnv.write_string(path, content)
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<()>> + Send + 'a>> {
        LocalExecutionEnv.write_bytes(path, content)
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<()>> + Send + 'a>> {
        LocalExecutionEnv.create_dir_all(path)
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<Vec<String>>> + Send + 'a>> {
        LocalExecutionEnv.read_dir(path)
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let comparable_path = canonical_or_original(path);
            if comparable_path == self.fail_path
                && self
                    .failures_left
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                        (remaining > 0).then_some(remaining - 1)
                    })
                    .is_ok()
            {
                return Err(IkarosError::Message("forced remove failure".into()));
            }
            LocalExecutionEnv.remove_file(path).await
        })
    }
}

impl ProcessRunner for SelfModifyTrackingEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<ProcessOutput>> + Send + 'a>> {
        self.process_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            assert_eq!(request.command, "cargo");
            assert_eq!(request.args, vec!["check"]);
            Ok(ProcessOutput {
                status: 0,
                stdout: "checked through execution env".into(),
                stderr: String::new(),
            })
        })
    }
}

#[derive(Debug)]
pub(super) struct GitStatusProcessEnv {
    calls: Arc<AtomicUsize>,
}

impl ProcessRunner for GitStatusProcessEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<ProcessOutput>> + Send + 'a>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            assert_eq!(request.command, "git");
            assert_eq!(request.args, vec!["status", "--porcelain=v1", "--branch"]);
            Ok(ProcessOutput {
                status: 0,
                stdout: "## main\n M tracked.rs\n?? new.rs\n".into(),
                stderr: String::new(),
            })
        })
    }
}

mod analysis;
mod context;
mod guarded_patch;
mod runtime;
mod self_modify;
