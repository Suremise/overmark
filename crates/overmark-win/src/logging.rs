//! Logging with `tracing` to a small rolling file beside the settings: `logs\
//! overmark.log`, rolled to `overmark.1.log` and `overmark.2.log` when it reaches 1 MB, so at
//! most three files of 1 MB each. Only the resident app writes it (one instance per session);
//! the grabber helper doesn't log to file.
//!
//! `log_panics` and `panic_message` are Replica's (`replica-win/src/logging.rs`); the rolling
//! writer is Overmark's, since Replica keeps one file per process instead.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// The log's base name: `overmark.log`, then `overmark.1.log` and so on.
pub const LOG_NAME: &str = "overmark";
/// When the current file rolls.
pub const MAX_BYTES: u64 = 1024 * 1024;
/// How many files are kept, the current one included.
pub const KEEP: usize = 3;

/// A log file that rolls by size: `<dir>\<name>.log`, then `<name>.1.log` … `<name>.<keep-1>.log`.
pub struct RollingFile {
    dir: PathBuf,
    name: String,
    max_bytes: u64,
    keep: usize,
    file: Option<File>,
    size: u64,
}

impl RollingFile {
    /// Opens (or creates) `<dir>\<name>.log` to append to, creating `dir`.
    pub fn open(dir: &Path, name: &str, max_bytes: u64, keep: usize) -> io::Result<RollingFile> {
        fs::create_dir_all(dir)?;
        let mut r = RollingFile {
            dir: dir.to_path_buf(),
            name: name.to_owned(),
            max_bytes: max_bytes.max(1),
            keep: keep.max(1),
            file: None,
            size: 0,
        };
        r.reopen()?;
        Ok(r)
    }

    /// The `n`th file: 0 is the current one.
    pub fn path(&self, n: usize) -> PathBuf {
        match n {
            0 => self.dir.join(format!("{}.log", self.name)),
            n => self.dir.join(format!("{}.{n}.log", self.name)),
        }
    }

    fn reopen(&mut self) -> io::Result<()> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path(0))?;
        self.size = file.metadata()?.len();
        self.file = Some(file);
        Ok(())
    }

    /// Moves `.log` to `.1.log`, `.1` to `.2` and so on, dropping the oldest, and starts afresh.
    fn roll(&mut self) -> io::Result<()> {
        self.file = None;
        let _ = fs::remove_file(self.path(self.keep - 1));
        for n in (0..self.keep - 1).rev() {
            let from = self.path(n);
            if from.exists() {
                fs::rename(&from, self.path(n + 1))?;
            }
        }
        self.reopen()
    }
}

impl Write for RollingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // A record that would cross the limit starts the next file, so no record is split.
        if self.size > 0 && self.size + buf.len() as u64 > self.max_bytes {
            self.roll()?;
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("no log file"))?;
        let n = file.write(buf)?;
        self.size += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.as_mut().map_or(Ok(()), File::flush)
    }
}

/// Shares one [`RollingFile`] among `tracing`'s writers.
#[derive(Clone)]
struct Shared(Arc<Mutex<RollingFile>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).flush()
    }
}

impl<'a> MakeWriter<'a> for Shared {
    type Writer = Shared;
    fn make_writer(&'a self) -> Shared {
        self.clone()
    }
}

/// Sets up the global `tracing` subscriber: to the rolling file in `dir` (none if `None`, a
/// folder that can't be written to) and to stderr. Returns the log file's path, if any. A file
/// that can't be opened is reported, and the logs go to stderr only.
pub fn init(dir: Option<&Path>) -> Option<PathBuf> {
    let file = dir.and_then(|d| match RollingFile::open(d, LOG_NAME, MAX_BYTES, KEEP) {
        Ok(f) => Some(f),
        Err(error) => {
            eprintln!(
                "{}",
                crate::core::tr!(
                    "cli-no-log-file",
                    folder = d.display().to_string(),
                    reason = error.to_string()
                )
            );
            None
        }
    });
    let path = file.as_ref().map(|f| f.path(0));
    let file_layer = file.map(|f| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_target(false)
            .with_writer(Shared(Arc::new(Mutex::new(f))))
    });
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(io::stderr)
        .with_target(false);
    let _ = tracing_subscriber::registry()
        .with(LevelFilter::INFO)
        .with(file_layer)
        .with(stderr_layer)
        .try_init();
    path
}

/// Logs every panic, on any thread, as an error with its thread and place, then does what
/// Rust would have (which writes to stderr: nowhere, in a GUI exe).
pub fn log_panics() {
    let earlier = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let place = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        tracing::error!(
            thread = thread.name().unwrap_or("unnamed"),
            place,
            message = panic_message(info.payload()),
            "panicked"
        );
        earlier(info);
    }));
}

/// A panic's message, from its payload (a `&str` or a `String`, as `panic!` makes them).
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic without a message".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolls_by_size_and_keeps_three_files() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("logs");
        let mut log = RollingFile::open(&dir, "overmark", 100, 3).unwrap();
        // Ten records of 40 bytes: two to a file, and only the newest three files stay.
        for i in 0..10u8 {
            log.write_all(&[b'a' + i; 40]).unwrap();
        }
        log.flush().unwrap();
        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["overmark.1.log", "overmark.2.log", "overmark.log"]);
        let read = |n: usize| fs::read(log.path(n)).unwrap();
        // The current file has the last two records; .1 the two before; .2 the two before that.
        assert_eq!(read(0), [vec![b'i'; 40], vec![b'j'; 40]].concat());
        assert_eq!(read(1), [vec![b'g'; 40], vec![b'h'; 40]].concat());
        assert_eq!(read(2), [vec![b'e'; 40], vec![b'f'; 40]].concat());
        for n in 0..3 {
            assert!(read(n).len() as u64 <= 100);
        }
    }

    #[test]
    fn appends_to_the_current_file_across_runs() {
        let temp = tempfile::tempdir().unwrap();
        {
            let mut log = RollingFile::open(temp.path(), "overmark", 1000, 3).unwrap();
            log.write_all(b"first run\n").unwrap();
        }
        let mut log = RollingFile::open(temp.path(), "overmark", 1000, 3).unwrap();
        log.write_all(b"second run\n").unwrap();
        assert_eq!(
            fs::read_to_string(log.path(0)).unwrap(),
            "first run\nsecond run\n"
        );
    }

    #[test]
    fn a_record_bigger_than_the_limit_still_gets_written() {
        let temp = tempfile::tempdir().unwrap();
        let mut log = RollingFile::open(temp.path(), "overmark", 10, 3).unwrap();
        log.write_all(&[b'x'; 25]).unwrap();
        log.write_all(&[b'y'; 25]).unwrap();
        assert_eq!(fs::read(log.path(0)).unwrap(), vec![b'y'; 25]);
        assert_eq!(fs::read(log.path(1)).unwrap(), vec![b'x'; 25]);
    }

    #[test]
    fn a_panic_s_message_from_either_payload() {
        let message = |f: fn()| panic_message(std::panic::catch_unwind(f).unwrap_err().as_ref());
        assert_eq!(message(|| panic!("plain")), "plain");
        assert_eq!(message(|| panic!("formatted {}", 1)), "formatted 1");
        assert_eq!(
            message(|| std::panic::panic_any(7)),
            "a panic without a message"
        );
    }
}
