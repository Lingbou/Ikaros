// SPDX-License-Identifier: GPL-3.0-only

use super::*;

impl FileSystem for LocalExecutionEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>> {
        Box::pin(async move {
            let metadata =
                fs::symlink_metadata(path).map_err(|source| IkarosError::io(path, source))?;
            let file_type = metadata.file_type();
            Ok(FileMetadata {
                is_file: metadata.is_file(),
                is_dir: metadata.is_dir(),
                is_symlink: file_type.is_symlink(),
                modified_at: metadata.modified().ok().and_then(system_time_to_rfc3339),
            })
        })
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(
            async move { fs::read_to_string(path).map_err(|source| IkarosError::io(path, source)) },
        )
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        Box::pin(async move { fs::read(path).map_err(|source| IkarosError::io(path, source)) })
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move { write_file_no_follow(path, content.as_bytes()) })
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move { write_file_no_follow(path, &content) })
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(
            async move { fs::create_dir_all(path).map_err(|source| IkarosError::io(path, source)) },
        )
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        Box::pin(async move {
            let mut entries = Vec::new();
            for entry in fs::read_dir(path).map_err(|source| IkarosError::io(path, source))? {
                let entry = entry.map_err(|source| IkarosError::io(path, source))?;
                entries.push(entry.file_name().to_string_lossy().to_string());
            }
            entries.sort();
            Ok(entries)
        })
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(
            async move { fs::remove_file(path).map_err(|source| IkarosError::io(path, source)) },
        )
    }
}

pub(super) fn write_file_no_follow(path: &Path, content: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
    }
    let mut file = no_follow_write_options()
        .open(path)
        .map_err(|source| IkarosError::io(path, source))?;
    file.write_all(content)
        .map_err(|source| IkarosError::io(path, source))
}

#[cfg(unix)]
pub(super) fn no_follow_write_options() -> fs::OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = fs::OpenOptions::new();
    options
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW);
    options
}

#[cfg(not(unix))]
pub(super) fn no_follow_write_options() -> fs::OpenOptions {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    options
}
pub(super) fn system_time_to_rfc3339(time: std::time::SystemTime) -> Option<String> {
    let duration = time.duration_since(std::time::UNIX_EPOCH).ok()?;
    let datetime = ::time::OffsetDateTime::from_unix_timestamp(duration.as_secs() as i64).ok()?;
    datetime
        .format(&::time::format_description::well_known::Rfc3339)
        .ok()
}
