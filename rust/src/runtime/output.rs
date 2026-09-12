//! Private inferred Runtime output directories.

use fs2::FileExt;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use super::error::RuntimeError;

/// Shared project lease for ordinary runs, exclusive for a cleaning run.
pub(crate) struct OutputLease {
    _project: File,
}

impl OutputLease {
    pub(crate) fn acquire(project: &Path, clean: bool) -> std::io::Result<Self> {
        let file = File::open(project)?;
        if clean {
            FileExt::try_lock_exclusive(&file)?;
        } else {
            FileExt::try_lock_shared(&file)?;
        }
        Ok(Self { _project: file })
    }
}

pub(crate) fn clean_output(root: &Path, protected: &[&Path]) -> std::io::Result<()> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(std::io::Error::other(
            "--clean requires an ordinary output directory, not a symlink or file",
        ));
    }
    let canonical = fs::canonicalize(root)?;
    for path in protected {
        let path = fs::canonicalize(path)?;
        if path.starts_with(&canonical) || canonical.starts_with(&path) {
            return Err(std::io::Error::other(
                "--clean would delete protected project inputs or reused output",
            ));
        }
    }
    // Preserve the validated root itself. Child symlinks are removed, never followed.
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

pub(crate) fn create_execution(root: &Path) -> Result<PathBuf, RuntimeError> {
    let now = time::OffsetDateTime::now_utc();
    let stamp = format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}.{:09}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
        now.nanosecond()
    );
    create_execution_at(root, &stamp)
}

fn create_execution_at(root: &Path, stamp: &str) -> Result<PathBuf, RuntimeError> {
    fs::create_dir_all(root).map_err(|source| RuntimeError::OutputScope {
        path: root.to_path_buf(),
        source,
    })?;
    for sequence in 0..1024 {
        let suffix = if sequence == 0 {
            String::new()
        } else {
            format!("-{sequence:04}")
        };
        let directory = root.join(format!("execution-{stamp}{suffix}"));
        match fs::create_dir(&directory) {
            Ok(()) => return Ok(directory),
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(RuntimeError::OutputScope {
                    path: directory,
                    source,
                });
            }
        }
    }
    Err(RuntimeError::OutputScope {
        path: root.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate a unique execution directory",
        ),
    })
}

pub(crate) fn create_replicate(root: &Path, index: u64) -> Result<PathBuf, RuntimeError> {
    let directory = root.join(format!("replicate-{index:06}"));
    fs::create_dir(&directory).map_err(|source| RuntimeError::OutputScope {
        path: directory.clone(),
        source,
    })?;
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_collisions_cleanup_scope_and_project_leases() {
        let project =
            std::env::temp_dir().join(format!("workflow-output-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&project);
        let root = project.join("output");
        fs::create_dir_all(project.join("wf_configs")).unwrap();
        let first = create_execution_at(&root, "20260912T143052.123456789Z").unwrap();
        let second = create_execution_at(&root, "20260912T143052.123456789Z").unwrap();
        assert_eq!(
            first.file_name().unwrap(),
            "execution-20260912T143052.123456789Z"
        );
        assert_eq!(
            second.file_name().unwrap(),
            "execution-20260912T143052.123456789Z-0001"
        );
        assert!(clean_output(&root, &[&first]).is_err());
        assert!(first.is_dir());
        let running = OutputLease::acquire(&project, false).unwrap();
        assert!(OutputLease::acquire(&project, true).is_err());
        drop(running);
        let cleaning = OutputLease::acquire(&project, true).unwrap();
        assert!(OutputLease::acquire(&project, false).is_err());
        #[cfg(unix)]
        std::os::unix::fs::symlink(project.join("wf_configs"), root.join("outside-link")).unwrap();
        clean_output(&root, &[&project.join("wf_configs")]).unwrap();
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        assert!(project.join("wf_configs").is_dir());
        drop(cleaning);
        #[cfg(unix)]
        {
            fs::remove_dir(&root).unwrap();
            std::os::unix::fs::symlink(project.join("wf_configs"), &root).unwrap();
            assert!(clean_output(&root, &[]).is_err());
        }
        fs::remove_dir_all(project).unwrap();
    }
}
