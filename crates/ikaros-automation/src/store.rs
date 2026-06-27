// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    ScheduleDeliveryTarget, ScheduleJobOptions, ScheduleRetryPolicy, ScheduleRunHistoryEntry,
    ScheduleRunUpdate, ScheduledJob,
};
use ikaros_core::{IkarosError, Result, now_rfc3339, redact_secrets};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

const SCHEDULE_HISTORY_LIMIT: usize = 20;

#[derive(Debug, Clone)]
pub struct LocalScheduleStore {
    path: PathBuf,
}

impl LocalScheduleStore {
    pub fn new(automation_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: automation_dir.into().join("schedules.jsonl"),
        }
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn add(
        &self,
        task: impl Into<String>,
        run_at: impl Into<String>,
        interval_seconds: Option<u64>,
        agent: Option<String>,
    ) -> Result<ScheduledJob> {
        self.add_with_deliveries(
            task,
            run_at,
            interval_seconds,
            agent,
            ScheduleDeliveryTarget::default_targets(),
        )
    }

    pub fn add_with_deliveries(
        &self,
        task: impl Into<String>,
        run_at: impl Into<String>,
        interval_seconds: Option<u64>,
        agent: Option<String>,
        deliveries: Vec<ScheduleDeliveryTarget>,
    ) -> Result<ScheduledJob> {
        self.add_with_options(
            task,
            run_at,
            ScheduleJobOptions {
                interval_seconds,
                agent,
                deliveries,
                ..ScheduleJobOptions::default()
            },
        )
    }

    pub fn add_with_options(
        &self,
        task: impl Into<String>,
        run_at: impl Into<String>,
        mut options: ScheduleJobOptions,
    ) -> Result<ScheduledJob> {
        validate_schedule_options(&options)?;
        normalize_timezone(&mut options)?;
        let run_at = normalize_schedule_time(&run_at.into())?;
        let mut jobs = self.read_all()?;
        let job = ScheduledJob::new_with_options(task, run_at, options)?;
        jobs.push(job.clone());
        self.write_all(&jobs)?;
        Ok(job)
    }

    pub fn list(&self) -> Result<Vec<ScheduledJob>> {
        let mut jobs = self.read_all()?;
        sort_jobs(&mut jobs);
        Ok(jobs)
    }

    pub fn due_now(&self) -> Result<Vec<ScheduledJob>> {
        let now = OffsetDateTime::now_utc();
        let mut jobs = self
            .read_all()?
            .into_iter()
            .filter(|job| job.enabled && is_due(job, now))
            .collect::<Vec<_>>();
        sort_jobs(&mut jobs);
        Ok(jobs)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<Option<ScheduledJob>> {
        let mut jobs = self.read_all()?;
        let now = now_rfc3339()?;
        let mut updated = None;
        for job in &mut jobs {
            if job.id == id {
                job.enabled = enabled;
                job.updated_at = now.clone();
                updated = Some(job.clone());
                break;
            }
        }
        self.write_all(&jobs)?;
        Ok(updated)
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        let jobs = self.read_all()?;
        let before = jobs.len();
        let retained = jobs
            .into_iter()
            .filter(|job| job.id != id)
            .collect::<Vec<_>>();
        self.write_all(&retained)?;
        Ok(retained.len() != before)
    }

    pub fn record_run(
        &self,
        id: &str,
        status: impl Into<String>,
        summary: impl Into<String>,
    ) -> Result<Option<ScheduleRunUpdate>> {
        let mut jobs = self.read_all()?;
        let now = OffsetDateTime::now_utc();
        let ran_at = format_time(now)?;
        let status = redact_secrets(&status.into());
        let summary = redact_secrets(&summary.into());
        let mut update = None;
        for job in &mut jobs {
            if job.id == id {
                let failed_attempt = failed_attempt_for_status(job, &status);
                let retrying_after_failure =
                    failed_attempt.is_some_and(|attempt| should_retry_failed_job(job, attempt));
                let next_run_at = next_run_at(job, now, &status)?;
                job.last_run_at = Some(ran_at.clone());
                job.last_status = Some(status.clone());
                job.last_summary = Some(summary.clone());
                job.updated_at = ran_at.clone();
                job.enabled = next_run_at.is_some();
                if let Some(next_run_at) = &next_run_at {
                    job.run_at = next_run_at.clone();
                }
                job.retry_attempts = next_retry_attempts(
                    &status,
                    failed_attempt,
                    retrying_after_failure,
                    next_run_at.is_none(),
                );
                append_history(
                    job,
                    ScheduleRunHistoryEntry {
                        ran_at: ran_at.clone(),
                        status: status.clone(),
                        summary: summary.clone(),
                        next_run_at: next_run_at.clone(),
                        enabled: job.enabled,
                        attempt: failed_attempt.unwrap_or(1),
                    },
                );
                update = Some(ScheduleRunUpdate {
                    ran_at: ran_at.clone(),
                    status: status.clone(),
                    summary: summary.clone(),
                    next_run_at,
                    enabled: job.enabled,
                });
                break;
            }
        }
        self.write_all(&jobs)?;
        Ok(update)
    }

    pub fn deliveries_dir(&self) -> PathBuf {
        self.path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("deliveries")
    }

    pub fn write_local_delivery(
        &self,
        job_id: &str,
        run_id: &str,
        content: impl Into<String>,
    ) -> Result<PathBuf> {
        let path = self
            .deliveries_dir()
            .join(safe_path_fragment(job_id))
            .join(format!("{}.md", safe_path_fragment(run_id)));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        fs::write(&path, redact_secrets(&content.into()))
            .map_err(|source| IkarosError::io(&path, source))?;
        Ok(path)
    }

    fn ensure_parent(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        Ok(())
    }

    fn read_all(&self) -> Result<Vec<ScheduledJob>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file =
            fs::File::open(&self.path).map_err(|source| IkarosError::io(&self.path, source))?;
        let reader = BufReader::new(file);
        let mut jobs = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|source| IkarosError::io(&self.path, source))?;
            if !line.trim().is_empty() {
                jobs.push(serde_json::from_str(&line)?);
            }
        }
        Ok(jobs)
    }

    fn write_all(&self, jobs: &[ScheduledJob]) -> Result<()> {
        self.ensure_parent()?;
        let lines = jobs
            .iter()
            .map(serde_json::to_string)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let temp_path = schedule_temp_path(&self.path);
        let write_result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp_path)
                .map_err(|source| IkarosError::io(&temp_path, source))?;
            for line in &lines {
                writeln!(file, "{line}").map_err(|source| IkarosError::io(&temp_path, source))?;
            }
            file.flush()
                .map_err(|source| IkarosError::io(&temp_path, source))?;
            file.sync_all()
                .map_err(|source| IkarosError::io(&temp_path, source))?;
            Ok(())
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temp_path);
            return Err(error);
        }
        if let Err(source) = fs::rename(&temp_path, &self.path) {
            let _ = fs::remove_file(&temp_path);
            return Err(IkarosError::io(&self.path, source));
        }
        sync_parent_dir(&self.path);
        Ok(())
    }
}

fn schedule_temp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("schedules.jsonl");
    path.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()))
}

fn sync_parent_dir(path: &Path) {
    #[cfg(unix)]
    if let Some(parent) = path.parent()
        && let Ok(dir) = fs::File::open(parent)
    {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn safe_path_fragment(value: &str) -> String {
    let mut fragment = value
        .chars()
        .map(|ch| match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' => ch,
            _ => '-',
        })
        .collect::<String>();
    if fragment.is_empty() {
        fragment = "delivery".into();
    }
    fragment.truncate(80);
    fragment
}

pub fn normalize_schedule_time(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("now") {
        return format_time(OffsetDateTime::now_utc());
    }
    let parsed = parse_time(trimmed)?;
    format_time(parsed)
}

fn is_due(job: &ScheduledJob, now: OffsetDateTime) -> bool {
    let Ok(run_at) = parse_time(&job.run_at) else {
        return false;
    };
    if run_at > now {
        return false;
    }
    if let Some(grace_period_seconds) = job.grace_period_seconds {
        let Ok(grace) = schedule_interval_duration(grace_period_seconds) else {
            return false;
        };
        let Some(deadline) = run_at.checked_add(grace) else {
            return false;
        };
        if now > deadline {
            return false;
        }
    }
    true
}

fn next_run_at(job: &ScheduledJob, now: OffsetDateTime, status: &str) -> Result<Option<String>> {
    if let Some(attempt) = failed_attempt_for_status(job, status)
        && should_retry_failed_job(job, attempt)
    {
        let backoff = schedule_interval_duration(job.retry.backoff_seconds)?;
        let next = now.checked_add(backoff).ok_or_else(|| {
            IkarosError::Message(
                "schedule retry backoff moves next run outside supported time range".into(),
            )
        })?;
        return Ok(Some(format_time(next)?));
    }
    let Some(interval_seconds) = job.interval_seconds else {
        return Ok(None);
    };
    let mut next = parse_time(&job.run_at).unwrap_or(now);
    let interval = schedule_interval_duration(interval_seconds)?;
    while next <= now {
        next = next.checked_add(interval).ok_or_else(|| {
            IkarosError::Message(
                "schedule interval moves next run outside supported time range".into(),
            )
        })?;
    }
    Ok(Some(format_time(next)?))
}

fn failed_attempt_for_status(job: &ScheduledJob, status: &str) -> Option<u32> {
    is_failed_status(status).then(|| job.retry_attempts.saturating_add(1))
}

fn should_retry_failed_job(job: &ScheduledJob, attempt: u32) -> bool {
    attempt < job.retry.max_attempts
}

fn next_retry_attempts(
    status: &str,
    failed_attempt: Option<u32>,
    retrying_after_failure: bool,
    terminal: bool,
) -> u32 {
    if !is_failed_status(status) {
        return 0;
    }
    let failed_attempt = failed_attempt.unwrap_or(1);
    if retrying_after_failure || terminal {
        failed_attempt
    } else {
        0
    }
}

fn is_failed_status(status: &str) -> bool {
    !matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "completed" | "complete" | "success" | "succeeded" | "ok"
    )
}

fn append_history(job: &mut ScheduledJob, entry: ScheduleRunHistoryEntry) {
    job.history.push(entry);
    if job.history.len() > SCHEDULE_HISTORY_LIMIT {
        let excess = job.history.len() - SCHEDULE_HISTORY_LIMIT;
        job.history.drain(0..excess);
    }
}

fn validate_schedule_options(options: &ScheduleJobOptions) -> Result<()> {
    if let Some(interval_seconds) = options.interval_seconds {
        validate_interval_seconds(interval_seconds)?;
    }
    validate_retry_policy(&options.retry)?;
    if let Some(grace_period_seconds) = options.grace_period_seconds {
        validate_interval_seconds(grace_period_seconds).map_err(|_| {
            IkarosError::Message("schedule grace period must be greater than zero".into())
        })?;
    }
    if let Some(timezone) = &options.timezone
        && timezone.trim().is_empty()
    {
        return Err(IkarosError::Message(
            "schedule timezone must not be empty".into(),
        ));
    }
    Ok(())
}

fn validate_retry_policy(retry: &ScheduleRetryPolicy) -> Result<()> {
    if retry.max_attempts == 0 {
        return Err(IkarosError::Message(
            "schedule retry max attempts must be greater than zero".into(),
        ));
    }
    if retry.max_attempts > 1 {
        validate_interval_seconds(retry.backoff_seconds).map_err(|_| {
            IkarosError::Message("schedule retry backoff must be greater than zero".into())
        })?;
    }
    Ok(())
}

fn normalize_timezone(options: &mut ScheduleJobOptions) -> Result<()> {
    if let Some(timezone) = &mut options.timezone {
        *timezone = timezone.trim().to_string();
        if timezone.is_empty() {
            return Err(IkarosError::Message(
                "schedule timezone must not be empty".into(),
            ));
        }
    }
    Ok(())
}

fn validate_interval_seconds(interval_seconds: u64) -> Result<()> {
    if interval_seconds == 0 {
        return Err(IkarosError::Message(
            "schedule interval must be greater than zero".into(),
        ));
    }
    if i64::try_from(interval_seconds).is_err() {
        return Err(IkarosError::Message(format!(
            "schedule interval must be less than or equal to {} seconds",
            i64::MAX
        )));
    }
    Ok(())
}

fn schedule_interval_duration(interval_seconds: u64) -> Result<Duration> {
    validate_interval_seconds(interval_seconds)?;
    let seconds = i64::try_from(interval_seconds).map_err(|_| {
        IkarosError::Message(format!(
            "schedule interval must be less than or equal to {} seconds",
            i64::MAX
        ))
    })?;
    Ok(Duration::seconds(seconds))
}

fn parse_time(value: &str) -> Result<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|source| IkarosError::Message(format!("invalid schedule time: {source}")))
}

fn format_time(value: OffsetDateTime) -> Result<String> {
    value
        .format(&Rfc3339)
        .map_err(|source| IkarosError::Message(format!("failed to format schedule time: {source}")))
}

fn sort_jobs(jobs: &mut [ScheduledJob]) {
    jobs.sort_by(|left, right| {
        left.run_at
            .cmp(&right.run_at)
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.id.cmp(&right.id))
    });
}
