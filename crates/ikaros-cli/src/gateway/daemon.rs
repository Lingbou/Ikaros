// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::gateway) async fn run_message_worker(
    args: MessageWorker,
    store: &LocalGatewayStore,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    if args.interval_seconds == 0 {
        anyhow::bail!("message worker interval must be greater than zero");
    }
    if args.limit == 0 {
        anyhow::bail!("message worker limit must be greater than zero");
    }
    let worker_lock = acquire_message_worker_lock(&paths.gateway_dir)?;
    if let Some(recovery) = worker_lock.stale_recovery() {
        println!(
            "message_worker_lock_recovered: stale=true lock={} archived={} owner={}",
            recovery.lock_path.display(),
            recovery.archived_path.display(),
            recovery.owner
        );
    }
    let _worker_lock = worker_lock;
    let mut forensics = MessageWorkerForensics::start(&paths.gateway_dir, args.limit, args.once)?;
    println!("message_worker: started");
    println!(
        "message_worker_lock: {}",
        paths.gateway_dir.join(MESSAGE_WORKER_LOCK_FILE).display()
    );
    println!("message_worker_events: {}", forensics.path().display());
    println!("interval_seconds: {}", args.interval_seconds);
    println!("limit: {}", args.limit);
    println!("gateway_inbox: {}", store.inbox_path().display());
    println!("gateway_outbox: {}", store.outbox_path().display());
    loop {
        if let Some(reason) = take_message_worker_stop_request(&paths.gateway_dir)? {
            println!(
                "message_worker: stop_requested reason={}",
                redact_secrets(&reason)
            );
            forensics.finish("stopped", "stop requested")?;
            break;
        }
        let report = match run_gateway_worker_tick(
            store,
            args.limit,
            paths,
            workspace,
            agent_override,
        )
        .await
        {
            Ok(report) => report,
            Err(error) => {
                let reason = error.to_string();
                forensics.finish("failed", &reason)?;
                return Err(error.into());
            }
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
        if args.once {
            break;
        }
        sleep(Duration::from_secs(args.interval_seconds)).await;
    }
    forensics.finish("completed", "worker loop exited")?;
    Ok(())
}

pub(in crate::gateway) fn start_message_daemon(
    args: MessageDaemonStart,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    if args.interval_seconds == 0 {
        anyhow::bail!("message daemon interval must be greater than zero");
    }
    if args.limit == 0 {
        anyhow::bail!("message daemon limit must be greater than zero");
    }
    fs::create_dir_all(&paths.gateway_dir)
        .with_context(|| format!("failed to create {}", paths.gateway_dir.display()))?;
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    let lock_path = gateway_worker_lock_path(&store);
    if let Ok(owner) = fs::read_to_string(&lock_path)
        && !message_worker_lock_is_stale(&owner)
    {
        anyhow::bail!(
            "message daemon already running: lock={} owner={}",
            lock_path.display(),
            redacted_message_worker_lock_owner(&owner)
        );
    }
    clear_message_worker_stop_request(&paths.gateway_dir)?;

    let log_path = message_daemon_log_path(&paths.gateway_dir);
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("failed to open {}", log_path.display()))?;
    let stderr_log = log
        .try_clone()
        .with_context(|| format!("failed to clone {}", log_path.display()))?;
    let exe = std::env::current_exe().with_context(|| "failed to locate current executable")?;
    let mut command = Command::new(exe);
    command.arg("--ikaros-home").arg(&paths.home).arg(workspace);
    if let Some(agent) = agent_override {
        command.arg("--agent").arg(agent);
    }
    command
        .args(["message", "worker"])
        .arg("--interval-seconds")
        .arg(args.interval_seconds.to_string())
        .arg("--limit")
        .arg(args.limit.to_string())
        .env_remove("IKAROS_RUN_LIVE_MODEL_TESTS")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr_log));
    let mut child = command
        .spawn()
        .with_context(|| "failed to start message daemon worker")?;
    let pid = child.id();
    let started = wait_for_daemon_start(&mut child, &lock_path, &log_path)?;
    println!("message_daemon: started");
    println!("message_daemon_pid: {pid}");
    println!("message_daemon_started: {started}");
    println!("message_daemon_lock: {}", lock_path.display());
    println!("message_daemon_log: {}", log_path.display());
    println!(
        "message_daemon_worker: interval_seconds={} limit={}",
        args.interval_seconds, args.limit
    );
    Ok(())
}

pub(in crate::gateway) fn wait_for_daemon_start(
    child: &mut std::process::Child,
    lock_path: &Path,
    log_path: &Path,
) -> Result<&'static str> {
    let started_at = Instant::now();
    loop {
        if lock_path.exists() {
            return Ok("lock_acquired");
        }
        if let Some(status) = child
            .try_wait()
            .with_context(|| "failed to poll message daemon worker")?
        {
            let log_tail = fs::read_to_string(log_path)
                .ok()
                .map(|log| redact_secrets(&log))
                .unwrap_or_else(|| "unreadable".into());
            anyhow::bail!("message daemon worker exited during startup: {status}; log={log_tail}");
        }
        if started_at.elapsed() >= Duration::from_secs(3) {
            anyhow::bail!(
                "message daemon worker did not acquire lock within startup timeout: lock={} log={}",
                lock_path.display(),
                log_path.display()
            );
        }
        thread::sleep(Duration::from_millis(25));
    }
}

pub(in crate::gateway) fn request_message_daemon_stop(
    args: MessageDaemonStop,
    paths: &IkarosPaths,
) -> Result<()> {
    let payload = write_message_worker_stop_request(&args.reason, &paths.gateway_dir)?;
    let reason = payload
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none");
    println!(
        "message_daemon_stop: requested=true path={} reason={}",
        paths.gateway_dir.join(MESSAGE_WORKER_STOP_FILE).display(),
        reason
    );
    Ok(())
}

pub(in crate::gateway) fn restart_message_daemon(
    args: MessageDaemonStart,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    let status = message_daemon_status_label(&store);
    let stop_requested = status == "running";
    if stop_requested {
        write_message_worker_stop_request("operator requested restart", &paths.gateway_dir)?;
        wait_for_daemon_stop(
            &store,
            Duration::from_secs(args.interval_seconds.max(1) + 3),
        )?;
    }
    clear_message_worker_stop_request(&paths.gateway_dir)?;
    println!(
        "message_daemon_restart: starting=true stop_requested={stop_requested} previous_status={status}"
    );
    start_message_daemon(args, paths, workspace, agent_override)?;
    Ok(())
}

pub(in crate::gateway) fn wait_for_daemon_stop(
    store: &LocalGatewayStore,
    timeout: Duration,
) -> Result<()> {
    let started_at = Instant::now();
    loop {
        let status = message_daemon_status_label(store);
        if matches!(status, "stopped" | "stale" | "unknown") {
            return Ok(());
        }
        if started_at.elapsed() >= timeout {
            anyhow::bail!("message daemon did not stop within {:?}", timeout);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub(in crate::gateway) fn request_message_worker_stop(
    args: MessageWorkerStop,
    paths: &IkarosPaths,
) -> Result<()> {
    let payload = write_message_worker_stop_request(&args.reason, &paths.gateway_dir)?;
    let reason = payload
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none");
    println!(
        "message_worker_stop: requested=true path={} reason={}",
        paths.gateway_dir.join(MESSAGE_WORKER_STOP_FILE).display(),
        reason
    );
    Ok(())
}
