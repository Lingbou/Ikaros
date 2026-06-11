// SPDX-License-Identifier: GPL-3.0-only

use super::BodyDashboard;
use anyhow::{Context, Result};
use ikaros_body::{BodyKind, DashboardRenderOptions, WebDashboardAdapter};
use ikaros_core::IkarosPaths;
use ikaros_runtime::current_body_frame;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

pub(super) fn write_dashboard(
    args: BodyDashboard,
    paths: &IkarosPaths,
    workspace: &Path,
) -> Result<()> {
    paths.ensure()?;
    let frame = current_body_frame(paths, args.events, BodyKind::Web)?;
    let output = dashboard_output_path(paths, args.output)?;
    let snapshot_output = args
        .snapshot_output
        .map(|path| ikaros_home_output_path(paths, path));
    let snapshot_output = snapshot_output.transpose()?;
    let snapshot_link = snapshot_output
        .as_ref()
        .map(|path| dashboard_snapshot_href(&paths.home, &output, path))
        .transpose()?;
    let render_options = DashboardRenderOptions {
        refresh_seconds: args.refresh_seconds,
        snapshot_path: snapshot_link,
    };
    let html = WebDashboardAdapter.render_frame_with_options(&frame, &render_options);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&output, html).with_context(|| format!("failed to write {}", output.display()))?;
    if let Some(snapshot_output) = snapshot_output {
        if let Some(parent) = snapshot_output.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&snapshot_output, serde_json::to_string_pretty(&frame)?)
            .with_context(|| format!("failed to write {}", snapshot_output.display()))?;
        println!("snapshot: {}", snapshot_output.display());
    }
    println!("dashboard: {}", output.display());
    if let Some(seconds) = args.refresh_seconds {
        println!("refresh_seconds: {}", seconds.max(1));
    }
    println!("workspace: {}", workspace.display());
    Ok(())
}

pub(super) fn dashboard_output_path(
    paths: &IkarosPaths,
    output: Option<PathBuf>,
) -> Result<PathBuf> {
    let relative = output.unwrap_or_else(|| PathBuf::from("dashboard.html"));
    ikaros_home_output_path(paths, relative)
}

pub(super) fn ikaros_home_output_path(paths: &IkarosPaths, relative: PathBuf) -> Result<PathBuf> {
    if relative.is_absolute() {
        anyhow::bail!("dashboard output must be relative to IKAROS_HOME");
    }
    for component in relative.components() {
        match component {
            Component::Normal(part) if part == ".temp" => {
                anyhow::bail!("dashboard output cannot target .temp")
            }
            Component::Normal(_) | Component::CurDir => {}
            _ => anyhow::bail!("dashboard output must stay inside IKAROS_HOME"),
        }
    }
    Ok(paths.home.join(relative))
}

pub(super) fn dashboard_snapshot_href(
    home: &Path,
    dashboard_output: &Path,
    snapshot_output: &Path,
) -> Result<String> {
    let dashboard_dir = dashboard_output
        .parent()
        .ok_or_else(|| anyhow::anyhow!("dashboard output has no parent directory"))?;
    let from = dashboard_dir
        .strip_prefix(home)
        .with_context(|| "dashboard output must stay under IKAROS_HOME")?;
    let target = snapshot_output
        .strip_prefix(home)
        .with_context(|| "snapshot output must stay under IKAROS_HOME")?;
    let from_components = href_components(from);
    let target_components = href_components(target);
    let common = from_components
        .iter()
        .zip(target_components.iter())
        .take_while(|(left, right)| left == right)
        .count();
    let mut href = PathBuf::new();
    for _ in common..from_components.len() {
        href.push("..");
    }
    for component in &target_components[common..] {
        href.push(component);
    }
    Ok(href.to_string_lossy().replace('\\', "/"))
}

fn href_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect()
}
