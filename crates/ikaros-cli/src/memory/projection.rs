// SPDX-License-Identifier: GPL-3.0-only

use super::journal::append_projection_rendered_journal;
use super::{MemoryProjectionArgs, MemoryProjectionCommand};
use anyhow::Result;
use ikaros_core::IkarosPaths;
use ikaros_host::memory_projection_stores;
use ikaros_state::memory::{
    LocalMemoryStore, MemoryProjectionFileStore, MemoryProjectionInput, MemoryQuery, MemoryStore,
    ProjectionRenderer,
};
use serde_json::json;

pub(super) fn memory_projection_command(
    command: MemoryProjectionCommand,
    paths: &IkarosPaths,
) -> Result<()> {
    let stores = memory_projection_stores(paths)?;
    let store = stores.memory;
    let file_store = stores.projection_files;
    match command {
        MemoryProjectionCommand::Render(args) => {
            let projection = render_projection(&store, &args)?;
            let written = file_store.write(&projection, args.scope.as_deref())?;
            append_projection_rendered_journal(paths, args.scope.as_deref())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "summary": "projection rendered",
                    "directory": file_store.dir(),
                    "files": written,
                }))?
            );
        }
        MemoryProjectionCommand::Show(args) => {
            let projection = file_store.read(args.scope.as_deref())?;
            println!("{}", projection.user.trim_end());
            println!();
            println!("{}", projection.project.trim_end());
            println!();
            println!("{}", projection.general.trim_end());
        }
    }
    Ok(())
}

pub(super) fn refresh_default_projection(
    store: &LocalMemoryStore,
    paths: &IkarosPaths,
    project_scope: &str,
) -> Result<()> {
    let args = MemoryProjectionArgs {
        user_scope: "default".into(),
        scope: Some(project_scope.to_owned()),
    };
    let projection = render_projection(store, &args)?;
    MemoryProjectionFileStore::new(&paths.memory_dir).write(&projection, Some(project_scope))?;
    Ok(())
}

fn render_projection(
    store: &LocalMemoryStore,
    args: &MemoryProjectionArgs,
) -> ikaros_core::Result<ikaros_state::memory::MemoryProjection> {
    let records = store.list(MemoryQuery {
        limit: Some(usize::MAX),
        ..MemoryQuery::default()
    })?;
    ProjectionRenderer::default().render(MemoryProjectionInput {
        user_scope: args.user_scope.clone(),
        project_scope: args.scope.clone(),
        perspective: None,
        records,
    })
}
