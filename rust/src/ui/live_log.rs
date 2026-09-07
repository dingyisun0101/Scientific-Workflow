//! Synchronous real-time persistence of UI messages in an execution scope.

use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use super::state::Message;

#[derive(Default)]
pub(super) struct LiveLog {
    path: Option<PathBuf>,
    file: Option<File>,
    pending: Vec<String>,
}

impl LiveLog {
    pub(super) fn start(&mut self, execution_directory: &Path) -> io::Result<()> {
        if self.file.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "the Workflow live log was already started",
            ));
        }
        let path = execution_directory.join("log.txt");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|source| {
                io::Error::new(
                    source.kind(),
                    format!("create live log `{}`: {source}", path.display()),
                )
            })?;
        for message in self.pending.drain(..) {
            writeln!(file, "{message}")?;
        }
        file.flush()?;
        self.path = Some(path);
        self.file = Some(file);
        Ok(())
    }

    pub(super) fn append(&mut self, message: &Message) -> io::Result<()> {
        if let Some(file) = &mut self.file {
            writeln!(file, "{}", message.text)?;
            file.flush()
        } else {
            self.pending.push(message.text.clone());
            Ok(())
        }
    }

    pub(super) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::LiveLog;
    use crate::ui::state::Message;

    #[test]
    fn pending_and_live_messages_are_immediately_visible_in_the_execution_log() {
        let root = std::env::temp_dir().join(format!(
            "scientific-workflow-live-log-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let mut log = LiveLog::default();
        log.append(&Message {
            level: "info".into(),
            text: "before start".into(),
        })
        .unwrap();
        log.start(&root).unwrap();
        log.append(&Message {
            level: "success".into(),
            text: "after start".into(),
        })
        .unwrap();

        assert_eq!(
            fs::read_to_string(log.path().unwrap()).unwrap(),
            "before start\nafter start\n"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
