// SPDX-License-Identifier: GPL-3.0-only

use crate::{IkarosError, Result};
use serde::{Deserialize, Serialize};
use std::{env, fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IkarosPaths {
    pub home: PathBuf,
    pub config: PathBuf,
    pub persona_dir: PathBuf,
    pub persona_profile: PathBuf,
    pub memory_dir: PathBuf,
    pub rag_dir: PathBuf,
    pub audit_dir: PathBuf,
    pub automation_dir: PathBuf,
    pub gateway_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub skills_dir: PathBuf,
}

impl IkarosPaths {
    pub fn from_env() -> Result<Self> {
        let home = match env::var_os("IKAROS_HOME") {
            Some(value) => PathBuf::from(value),
            None => default_home()?,
        };
        Ok(Self::from_home(home))
    }

    pub fn from_home(home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        let persona_dir = home.join("persona");
        let persona_profile = persona_dir.join("profile.md");
        Self {
            config: home.join("config.yaml"),
            persona_dir,
            persona_profile,
            memory_dir: home.join("memory"),
            rag_dir: home.join("rag"),
            audit_dir: home.join("audit"),
            automation_dir: home.join("automation"),
            gateway_dir: home.join("gateway"),
            cache_dir: home.join("cache"),
            logs_dir: home.join("logs"),
            skills_dir: home.join("skills"),
            home,
        }
    }

    pub fn ensure(&self) -> Result<()> {
        for path in [
            &self.home,
            &self.persona_dir,
            &self.memory_dir,
            &self.rag_dir,
            &self.audit_dir,
            &self.automation_dir,
            &self.gateway_dir,
            &self.cache_dir,
            &self.logs_dir,
            &self.skills_dir,
        ] {
            fs::create_dir_all(path).map_err(|source| IkarosError::io(path, source))?;
        }
        Ok(())
    }
}

fn default_home() -> Result<PathBuf> {
    if cfg!(windows) {
        env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .map(|path| path.join(".ikaros"))
            .ok_or_else(|| IkarosError::MissingHome("USERPROFILE".into()))
    } else {
        env::var_os("HOME")
            .map(PathBuf::from)
            .map(|path| path.join(".ikaros"))
            .ok_or_else(|| IkarosError::MissingHome("HOME".into()))
    }
}
