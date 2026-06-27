// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(super) fn docker_process_request(
    workspace_root: &Path,
    image: &str,
    request: ProcessRequest,
) -> Result<ProcessRequest> {
    let image = image.trim();
    if image.is_empty() {
        return Err(IkarosError::Message(
            "docker sandbox image must not be empty".into(),
        ));
    }
    if request.cwd_scope == ProcessCwdScope::Plugin && request.use_shell {
        return Err(IkarosError::Message(
            "docker sandbox does not support shell-based plugin commands".into(),
        ));
    }

    let workspace_root = fs::canonicalize(workspace_root)
        .map_err(|source| IkarosError::io(workspace_root, source))?;
    let host_cwd =
        fs::canonicalize(&request.cwd).map_err(|source| IkarosError::io(&request.cwd, source))?;
    let mount = docker_mount_for_request(&workspace_root, &host_cwd, request.cwd_scope)?;
    let container_root = if mount.kind == DockerMountKind::Workspace {
        "/workspace"
    } else {
        "/plugin"
    };
    let container_cwd = docker_container_path(&mount.source, &host_cwd, container_root)?;
    let mut args = vec![
        "run".to_owned(),
        "--rm".to_owned(),
        "--network".to_owned(),
        "none".to_owned(),
        "--workdir".to_owned(),
        container_cwd,
        "--mount".to_owned(),
        format!(
            "type=bind,source={},target={container_root},rw",
            mount.source.display()
        ),
        "--security-opt".to_owned(),
        "no-new-privileges".to_owned(),
        "--cap-drop".to_owned(),
        "ALL".to_owned(),
    ];
    if let Some(user) = docker_user_from_environment() {
        args.push("--user".to_owned());
        args.push(user);
    }
    for (name, value) in &request.env {
        args.push("--env".to_owned());
        args.push(format!("{name}={value}"));
    }
    args.push(image.to_owned());
    if request.use_shell {
        args.push("sh".to_owned());
        args.push("-lc".to_owned());
        args.push(request.command);
    } else {
        args.push(docker_container_command(
            &mount.source,
            container_root,
            &request.command,
        )?);
        args.extend(request.args);
    }

    let mut docker = ProcessRequest::program("docker", args, workspace_root);
    docker.stdin = request.stdin;
    docker.timeout_ms = request.timeout_ms;
    docker.max_output_bytes = request.max_output_bytes;
    Ok(docker)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DockerMountKind {
    Workspace,
    Plugin,
}

#[derive(Debug, Clone)]
pub(super) struct DockerMount {
    source: PathBuf,
    kind: DockerMountKind,
}

pub(super) fn docker_mount_for_request(
    workspace_root: &Path,
    host_cwd: &Path,
    scope: ProcessCwdScope,
) -> Result<DockerMount> {
    if host_cwd.starts_with(workspace_root) {
        return Ok(DockerMount {
            source: workspace_root.to_path_buf(),
            kind: DockerMountKind::Workspace,
        });
    }
    if scope == ProcessCwdScope::Plugin {
        return Ok(DockerMount {
            source: host_cwd.to_path_buf(),
            kind: DockerMountKind::Plugin,
        });
    }
    Err(IkarosError::OutOfScope(host_cwd.to_path_buf()))
}

pub(super) fn docker_container_command(
    mount_source: &Path,
    container_root: &str,
    command: &str,
) -> Result<String> {
    let command_path = Path::new(command);
    if command_path.is_absolute() && command_path.starts_with(mount_source) {
        return docker_container_path(mount_source, command_path, container_root);
    }
    Ok(command.to_owned())
}

pub(super) fn docker_container_path(
    mount_source: &Path,
    host_path: &Path,
    container_root: &str,
) -> Result<String> {
    let relative = host_path.strip_prefix(mount_source).map_err(|_| {
        IkarosError::Message(format!(
            "docker sandbox path is outside mounted source: {}",
            host_path.display()
        ))
    })?;
    let suffix = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(
                part.to_str()
                    .ok_or_else(|| {
                        IkarosError::Message(format!(
                            "docker sandbox path is not valid UTF-8: {}",
                            host_path.display()
                        ))
                    })
                    .map(str::to_owned),
            ),
            Component::CurDir => None,
            _ => Some(Err(IkarosError::Message(format!(
                "docker sandbox path contains unsupported component: {}",
                host_path.display()
            )))),
        })
        .collect::<Result<Vec<_>>>()?;
    if suffix.is_empty() {
        return Ok(container_root.to_owned());
    }
    Ok(format!("{container_root}/{}", suffix.join("/")))
}

pub(super) fn docker_user_from_environment() -> Option<String> {
    let uid = std::env::var("UID").ok()?;
    let gid = std::env::var("GID").ok()?;
    if uid.chars().all(|ch| ch.is_ascii_digit()) && gid.chars().all(|ch| ch.is_ascii_digit()) {
        Some(format!("{uid}:{gid}"))
    } else {
        None
    }
}
