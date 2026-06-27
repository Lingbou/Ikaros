// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::message) async fn run_message_worker(
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
    let _worker_lock = acquire_message_worker_lock(paths)?;
    let mut forensics = MessageWorkerForensics::start(paths, args.limit, args.once)?;
    println!("message_worker: started");
    println!(
        "message_worker_lock: {}",
        paths.gateway_dir.join(MESSAGE_WORKER_LOCK_FILE).display()
    );
    println!("message_worker_events: {}", forensics.path.display());
    println!("interval_seconds: {}", args.interval_seconds);
    println!("limit: {}", args.limit);
    println!("gateway_inbox: {}", store.inbox_path().display());
    println!("gateway_outbox: {}", store.outbox_path().display());
    loop {
        if let Some(reason) = take_message_worker_stop_request(paths)? {
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

pub(in crate::message) fn start_message_daemon(
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
    clear_message_worker_stop_request(paths)?;

    let log_path = message_daemon_log_path(paths);
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
    command
        .arg("--ikaros-home")
        .arg(&paths.home)
        .arg("--workspace")
        .arg(workspace);
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

pub(in crate::message) fn wait_for_daemon_start(
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

pub(in crate::message) fn request_message_daemon_stop(
    args: MessageDaemonStop,
    paths: &IkarosPaths,
) -> Result<()> {
    let payload = write_message_worker_stop_request(&args.reason, paths)?;
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

pub(in crate::message) fn restart_message_daemon(
    args: MessageDaemonStart,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    let status = message_daemon_status_label(&store);
    let stop_requested = status == "running";
    if stop_requested {
        write_message_worker_stop_request("operator requested restart", paths)?;
        wait_for_daemon_stop(
            &store,
            Duration::from_secs(args.interval_seconds.max(1) + 3),
        )?;
    }
    clear_message_worker_stop_request(paths)?;
    println!(
        "message_daemon_restart: starting=true stop_requested={stop_requested} previous_status={status}"
    );
    start_message_daemon(args, paths, workspace, agent_override)?;
    Ok(())
}

pub(in crate::message) fn wait_for_daemon_stop(
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

pub(in crate::message) const MESSAGE_WORKER_LOCK_FILE: &str = "message-worker.lock";
pub(in crate::message) const MESSAGE_WORKER_EVENTS_FILE: &str = "message-worker-events.jsonl";
pub(in crate::message) const MESSAGE_WORKER_STOP_FILE: &str = "message-worker.stop";

pub(in crate::message) struct MessageWorkerLock {
    path: PathBuf,
    body: String,
}

impl Drop for MessageWorkerLock {
    fn drop(&mut self) {
        let Ok(current) = fs::read_to_string(&self.path) else {
            return;
        };
        if current == self.body {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(in crate::message) fn acquire_message_worker_lock(
    paths: &IkarosPaths,
) -> Result<MessageWorkerLock> {
    fs::create_dir_all(&paths.gateway_dir)
        .with_context(|| format!("failed to create {}", paths.gateway_dir.display()))?;
    let path = paths.gateway_dir.join(MESSAGE_WORKER_LOCK_FILE);
    let started_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".into());
    let body = format!("pid={}\nstarted_at={started_at}\n", std::process::id());
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            if let Err(error) = file.write_all(body.as_bytes()) {
                let _ = fs::remove_file(&path);
                return Err(error).with_context(|| format!("failed to write {}", path.display()));
            }
            Ok(MessageWorkerLock { path, body })
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let current = fs::read_to_string(&path)
                .unwrap_or_else(|read_error| format!("unreadable: {read_error}"));
            if message_worker_lock_is_stale(&current) {
                let archived = path.with_file_name(format!(
                    "{MESSAGE_WORKER_LOCK_FILE}.stale.{}",
                    OffsetDateTime::now_utc().unix_timestamp_nanos()
                ));
                fs::rename(&path, &archived).with_context(|| {
                    format!(
                        "failed to archive stale message worker lock {}",
                        path.display()
                    )
                })?;
                println!(
                    "message_worker_lock_recovered: stale=true lock={} archived={} owner={}",
                    path.display(),
                    archived.display(),
                    redacted_message_worker_lock_owner(&current)
                );
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .with_context(|| {
                        format!(
                            "failed to create {} after stale lock recovery",
                            path.display()
                        )
                    })?;
                if let Err(error) = file.write_all(body.as_bytes()) {
                    let _ = fs::remove_file(&path);
                    return Err(error)
                        .with_context(|| format!("failed to write {}", path.display()));
                }
                return Ok(MessageWorkerLock { path, body });
            }
            let current = redacted_message_worker_lock_owner(&current);
            anyhow::bail!(
                "message worker already running: lock={} owner={}",
                path.display(),
                current
            );
        }
        Err(error) => Err(error).with_context(|| format!("failed to create {}", path.display())),
    }
}

pub(crate) fn message_worker_lock_is_stale(contents: &str) -> bool {
    let Some(pid) = contents
        .lines()
        .find_map(|line| line.trim().strip_prefix("pid="))
        .and_then(|pid| pid.parse::<u32>().ok())
    else {
        return false;
    };
    !pid_is_running(pid)
}

pub(crate) fn message_worker_lock_is_stale_label(contents: &str) -> &'static str {
    if message_worker_lock_is_stale(contents) {
        "true"
    } else {
        "false"
    }
}

pub(in crate::message) fn pid_is_running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        true
    }
}

pub(in crate::message) struct MessageWorkerForensics {
    path: PathBuf,
    run_id: String,
    finished: bool,
}

impl MessageWorkerForensics {
    pub(in crate::message) fn start(paths: &IkarosPaths, limit: usize, once: bool) -> Result<Self> {
        fs::create_dir_all(&paths.gateway_dir)
            .with_context(|| format!("failed to create {}", paths.gateway_dir.display()))?;
        let path = paths.gateway_dir.join(MESSAGE_WORKER_EVENTS_FILE);
        let started_at = now_rfc3339();
        let run_id = format!(
            "{}-{}",
            std::process::id(),
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        );
        append_message_worker_event(
            &path,
            serde_json::json!({
                "schema": "ikaros-message-worker-forensics-v1",
                "version": 1,
                "run_id": run_id,
                "event": "started",
                "status": "running",
                "at": started_at,
                "pid": std::process::id(),
                "limit": limit,
                "once": once,
            }),
        )?;
        Ok(Self {
            path,
            run_id,
            finished: false,
        })
    }

    pub(in crate::message) fn finish(&mut self, status: &str, reason: &str) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        append_message_worker_event(
            &self.path,
            serde_json::json!({
                "schema": "ikaros-message-worker-forensics-v1",
                "version": 1,
                "run_id": self.run_id,
                "event": "stopped",
                "status": status,
                "at": now_rfc3339(),
                "pid": std::process::id(),
                "reason": redact_secrets(reason),
            }),
        )?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for MessageWorkerForensics {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let _ = append_message_worker_event(
            &self.path,
            serde_json::json!({
                "schema": "ikaros-message-worker-forensics-v1",
                "version": 1,
                "run_id": self.run_id,
                "event": "stopped",
                "status": "aborted",
                "at": now_rfc3339(),
                "pid": std::process::id(),
                "reason": "dropped_before_finish",
            }),
        );
    }
}

pub(in crate::message) fn append_message_worker_event(
    path: &Path,
    value: serde_json::Value,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let value = redact_json(value);
    let line = serde_json::to_string(&value).with_context(|| "failed to encode worker event")?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    writeln!(file, "{line}").with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub(in crate::message) fn request_message_worker_stop(
    args: MessageWorkerStop,
    paths: &IkarosPaths,
) -> Result<()> {
    let payload = write_message_worker_stop_request(&args.reason, paths)?;
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

pub(in crate::message) fn write_message_worker_stop_request(
    reason: &str,
    paths: &IkarosPaths,
) -> Result<serde_json::Value> {
    fs::create_dir_all(&paths.gateway_dir)
        .with_context(|| format!("failed to create {}", paths.gateway_dir.display()))?;
    let path = paths.gateway_dir.join(MESSAGE_WORKER_STOP_FILE);
    let payload = redact_json(serde_json::json!({
        "schema": "ikaros-message-worker-stop-v1",
        "version": 1,
        "at": now_rfc3339(),
        "pid": std::process::id(),
        "reason": reason,
    }));
    let encoded =
        serde_json::to_string(&payload).with_context(|| "failed to encode worker stop request")?;
    fs::write(&path, format!("{encoded}\n"))
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(payload)
}

pub(in crate::message) fn clear_message_worker_stop_request(paths: &IkarosPaths) -> Result<()> {
    let path = paths.gateway_dir.join(MESSAGE_WORKER_STOP_FILE);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove {}", path.display())),
    }
}

pub(in crate::message) fn take_message_worker_stop_request(
    paths: &IkarosPaths,
) -> Result<Option<String>> {
    let path = paths.gateway_dir.join(MESSAGE_WORKER_STOP_FILE);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))?;
    let reason = serde_json::from_str::<serde_json::Value>(&contents)
        .ok()
        .and_then(|value| {
            value
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "stop requested".into());
    Ok(Some(reason))
}

pub(in crate::message) fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
}
