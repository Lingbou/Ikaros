// SPDX-License-Identifier: GPL-3.0-only
//! Concrete execution environments and governed network egress adapters.

use ikaros_core::{IkarosError, Result, redact_secrets};
pub use ikaros_toolkit::{
    ExecutionEnv, FileMetadata, FileSystem, NetworkEgress, NetworkEgressRequest,
    NetworkEgressResponse, ProcessCwdScope, ProcessOutput, ProcessRequest, ProcessRunner,
};
use ikaros_toolkit::{Skill, SkillContext, SkillOutput};
use std::{
    collections::BTreeMap,
    fs,
    future::Future,
    io::Write,
    path::Component,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    time,
};

mod debug;
mod docker;
mod envs;
mod filesystem;
mod network;
mod network_policy;
mod process;

pub use self::{
    debug::{
        SandboxDebugReport, SandboxIsolationLevel, SandboxIsolationMatrixEntry,
        SandboxIsolationStatus, local_sandbox_debug_report, sandbox_isolation_matrix,
    },
    envs::{
        DockerExecutionEnv, DryRunExecutionEnv, LocalExecutionEnv, NetworkedExecutionEnv,
        WorkspaceExecutionEnv,
    },
    network_policy::{GovernedNetworkEgress, HttpNetworkEgress, NetworkEgressPolicy},
};
use self::{docker::*, process::*};
