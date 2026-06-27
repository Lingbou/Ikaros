// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::debug) fn debug_state_db(
    args: DebugStateDbArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let output = debug_state_db_report(&args, paths, workspace, agent_override)?;
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

pub(crate) fn debug_state_db_json_line(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<String> {
    let args = DebugStateDbArgs {
        checkpoint: false,
        backup: None,
        repair: None,
        restore: None,
        prune_ended_before: None,
        vacuum: false,
    };
    let output = debug_state_db_report(&args, paths, workspace, agent_override)?;
    Ok(format!(
        "state_db_json: {}",
        serde_json::to_string(&redact_json(output))?
    ))
}

pub(in crate::debug) fn debug_state_db_report(
    args: &DebugStateDbArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Value> {
    let config = IkarosConfig::load(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let store = SqliteSessionStore::new(&agent.state_dir);
    let restore = args
        .restore
        .as_ref()
        .map(|path| store.restore_from(path))
        .transpose()?;
    if args.vacuum {
        store.vacuum()?;
    }
    let backup = args
        .backup
        .as_ref()
        .map(|path| store.backup_to(path))
        .transpose()?;
    let repair = args
        .repair
        .as_ref()
        .map(|path| store.repair_to(path))
        .transpose()?;
    let prune = args
        .prune_ended_before
        .as_deref()
        .map(parse_debug_state_db_prune_cutoff)
        .transpose()?
        .map(|cutoff| store.prune_ended_sessions_before(cutoff))
        .transpose()?;
    let report = store.operational_report()?;
    let wal_checkpoint = if args.checkpoint {
        store.checkpoint_wal()?
    } else {
        report.wal_checkpoint
    };
    let output = json!({
        "format": "ikaros-state-db-v1",
        "checkpoint_performed": args.checkpoint,
        "vacuum_performed": args.vacuum,
        "restore": restore,
        "backup": backup,
        "repair": repair,
        "prune": prune,
        "state_db": report.path.display().to_string(),
        "schema_version": report.schema_version,
        "journal_mode": report.journal_mode,
        "foreign_keys": report.foreign_keys,
        "integrity_check": report.integrity_check,
        "write_policy": report.write_policy,
        "wal_checkpoint": wal_checkpoint,
        "search_indexes": report.search_indexes,
    });
    Ok(output)
}

pub(in crate::debug) fn parse_debug_state_db_prune_cutoff(
    input: &str,
) -> Result<time::OffsetDateTime> {
    time::OffsetDateTime::parse(input, &time::format_description::well_known::Rfc3339)
        .map_err(|source| anyhow!("--prune-ended-before must be RFC3339: {source}"))
}
