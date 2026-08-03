use anyhow::{Context, Result, bail};
use fs4::TryLockError;
use serde::Deserialize;
use std::{
    env,
    fs::{self, File, OpenOptions},
    path::PathBuf,
};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub home: PathBuf,
    pub config: PathBuf,
    pub database: PathBuf,
    pub lock: PathBuf,
}

#[derive(Debug)]
pub struct AppLock {
    _file: File,
}

impl AppPaths {
    pub fn discover(home_override: Option<PathBuf>) -> Result<Self> {
        let home = match home_override {
            Some(path) => path,
            None => match env::var_os("IKAROS_HOME") {
                Some(path) => PathBuf::from(path),
                None => user_home()?.join(".ikaros"),
            },
        };
        Ok(Self {
            config: home.join("config.yaml"),
            database: home.join("sessions.sqlite3"),
            lock: home.join("runtime.lock"),
            home,
        })
    }

    pub fn ensure_home(&self) -> Result<()> {
        fs::create_dir_all(&self.home)
            .with_context(|| format!("failed to create {}", self.home.display()))
    }

    pub fn acquire_lock(&self) -> Result<AppLock> {
        self.ensure_home()?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.lock)
            .with_context(|| format!("failed to open {}", self.lock.display()))?;
        match fs4::FileExt::try_lock(&file) {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                bail!(
                    "another Ikaros process is already using {}; close it before starting a second instance",
                    self.home.display()
                )
            }
            Err(TryLockError::Error(error)) => {
                return Err(error)
                    .with_context(|| format!("failed to lock {}", self.lock.display()));
            }
        }
        Ok(AppLock { _file: file })
    }
}

fn user_home() -> Result<PathBuf> {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("could not determine the user home directory"))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub provider: ProviderConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub base_url: String,
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct ResolvedProviderConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
}

impl AppConfig {
    pub fn load(paths: &AppPaths) -> Result<Self> {
        let text = fs::read_to_string(&paths.config).with_context(|| {
            format!(
                "missing or unreadable config {}; create it with provider.base_url and provider.model",
                paths.config.display()
            )
        })?;
        serde_yaml_ng::from_str(&text)
            .with_context(|| format!("invalid config {}", paths.config.display()))
    }

    pub fn resolve_provider(&self) -> Result<ResolvedProviderConfig> {
        let base_url = env_value("IKAROS_BASE_URL")
            .unwrap_or_else(|| self.provider.base_url.trim().to_owned());
        let model =
            env_value("IKAROS_MODEL").unwrap_or_else(|| self.provider.model.trim().to_owned());
        let api_key = env_value("IKAROS_API_KEY").or_else(|| env_value("OPENAI_API_KEY"));
        if base_url.is_empty() {
            bail!("provider.base_url is empty");
        }
        if model.is_empty() {
            bail!("provider.model is empty");
        }
        Ok(ResolvedProviderConfig {
            base_url,
            model,
            api_key,
        })
    }
}

fn env_value(name: &str) -> Option<String> {
    env::var(name).ok().and_then(|value| non_empty(Some(value)))
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, AppPaths};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn loads_yaml_config() {
        let temp = tempdir().expect("tempdir");
        let paths = AppPaths::discover(Some(temp.path().join("home"))).expect("paths");
        paths.ensure_home().expect("home");
        fs::write(
            &paths.config,
            "provider:\n  base_url: https://example.test/v1\n  model: test-model\n",
        )
        .expect("config fixture");
        let loaded = AppConfig::load(&paths).expect("load");
        assert_eq!(loaded.provider.model, "test-model");
        assert_eq!(loaded.provider.base_url, "https://example.test/v1");
    }

    #[test]
    fn rejects_toml_config() {
        let temp = tempdir().expect("tempdir");
        let paths = AppPaths::discover(Some(temp.path().join("home"))).expect("paths");
        paths.ensure_home().expect("home");
        fs::write(
            &paths.config,
            "[provider]\nbase_url = 'https://example.test/v1'\nmodel = 'test-model'\n",
        )
        .expect("config fixture");

        assert!(AppConfig::load(&paths).is_err());
    }

    #[test]
    fn app_home_allows_only_one_process_lock() {
        let temp = tempdir().expect("tempdir");
        let paths = AppPaths::discover(Some(temp.path().join("home"))).expect("paths");
        let first = paths.acquire_lock().expect("first lock");
        let error = paths.acquire_lock().expect_err("second lock must fail");
        assert!(error.to_string().contains("another Ikaros process"));
        drop(first);
        paths.acquire_lock().expect("lock after release");
    }
}
