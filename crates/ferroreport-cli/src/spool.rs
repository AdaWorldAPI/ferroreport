//! Where the server keeps bytes it cannot send yet (a document that must arrive whole, the
//! rows of a request that will be answered as it renders): in memory up to a size, then in a
//! temporary file, with one budget of disk for all requests together.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// The limits every spool of a server shares.
pub struct SpoolLimits {
    /// Bytes one spool keeps in memory before it moves to a file.
    pub memory: usize,
    /// Directory for the files; `None` when there is nowhere to write.
    pub dir: Option<PathBuf>,
    /// Bytes all spools together may have on disk.
    pub disk: u64,
    used: AtomicU64,
    files: AtomicU64,
}

impl SpoolLimits {
    pub fn new(memory: usize, dir: Option<PathBuf>, disk: u64) -> SpoolLimits {
        SpoolLimits { memory, dir, disk, used: AtomicU64::new(0), files: AtomicU64::new(0) }
    }

    /// Bytes on disk right now.
    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }
}

/// Whether files can be created in `dir`.
pub fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!("ferroreport-{}.probe", std::process::id()));
    let ok = File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Said when a document outgrows memory and there is no directory for it.
pub const NO_DIR: &str = "is larger than what one report may keep in memory, and there is no directory to keep it in: \
                          start the server with --spool-dir (or FERROREPORT_SPOOL_DIR), or raise --spool-memory";
/// Said when the disk budget is spent.
pub const DISK_FULL: &str =
    "does not fit in the disk space reports may use together (--spool-disk): try again when others finish";

pub struct Spool {
    limits: Arc<SpoolLimits>,
    what: &'static str,
    memory: Vec<u8>,
    file: Option<File>,
    /// The file's path, where it could not be unlinked while open (it is removed on drop).
    path: Option<PathBuf>,
    on_disk: u64,
    len: u64,
    read_at: usize,
}

impl Spool {
    /// A spool for `what` ("the document", "the request"), for error messages.
    pub fn new(limits: Arc<SpoolLimits>, what: &'static str) -> Spool {
        Spool { limits, what, memory: Vec::new(), file: None, path: None, on_disk: 0, len: 0, read_at: 0 }
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    /// An error of the disk itself, said so that it reads as the server's problem.
    fn failed(&self, e: io::Error) -> io::Error {
        io::Error::other(format!("{} could not be kept on disk (--spool-dir): {e}", self.what))
    }

    fn reserve(&mut self, bytes: usize) -> io::Result<()> {
        let used = self.limits.used.fetch_add(bytes as u64, Ordering::Relaxed) + bytes as u64;
        self.on_disk += bytes as u64;
        if used > self.limits.disk {
            return Err(io::Error::other(format!("{} {DISK_FULL}", self.what)));
        }
        Ok(())
    }

    fn move_to_disk(&mut self) -> io::Result<()> {
        let dir = self.limits.dir.as_ref().ok_or_else(|| io::Error::other(format!("{} {NO_DIR}", self.what)))?;
        let n = self.limits.files.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("ferroreport-{}-{n}.spool", std::process::id()));
        let mut file =
            std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path).map_err(|e| self.failed(e))?;
        // Unlinked while open where the system allows it: the space comes back when the
        // file is closed, even if the server is killed.
        if cfg!(unix) && std::fs::remove_file(&path).is_ok() {
            self.path = None;
        } else {
            self.path = Some(path);
        }
        self.reserve(self.memory.len())?;
        file.write_all(&self.memory).map_err(|e| self.failed(e))?;
        self.memory = Vec::new();
        self.file = Some(file);
        Ok(())
    }

    /// Goes back to the start, to read what was written.
    pub fn rewind(&mut self) -> io::Result<()> {
        self.read_at = 0;
        match &mut self.file {
            Some(f) => {
                f.flush()?;
                f.seek(SeekFrom::Start(0)).map(|_| ())
            }
            None => Ok(()),
        }
    }
}

impl Write for Spool {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.file.is_none() {
            if self.memory.len() + buf.len() <= self.limits.memory {
                self.memory.extend_from_slice(buf);
                self.len += buf.len() as u64;
                return Ok(buf.len());
            }
            self.move_to_disk()?;
        }
        self.reserve(buf.len())?;
        if let Some(f) = &mut self.file {
            if let Err(e) = f.write_all(buf) {
                return Err(self.failed(e));
            }
        }
        self.len += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for Spool {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        match &mut self.file {
            Some(f) => f.read(out),
            None => {
                let n = out.len().min(self.memory.len() - self.read_at);
                out[..n].copy_from_slice(&self.memory[self.read_at..self.read_at + n]);
                self.read_at += n;
                Ok(n)
            }
        }
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        self.limits.used.fetch_sub(self.on_disk, Ordering::Relaxed);
        self.file = None;
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("ferroreport-spool-test-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn read_all(mut s: Spool) -> Vec<u8> {
        s.rewind().unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn small_stays_in_memory_and_large_goes_to_disk_and_back() {
        let d = dir();
        let limits = Arc::new(SpoolLimits::new(10, Some(d.clone()), 1000));
        let mut small = Spool::new(limits.clone(), "the document");
        small.write_all(b"12345").unwrap();
        assert_eq!((small.len(), limits.used()), (5, 0));
        assert_eq!(read_all(small), b"12345");

        let mut large = Spool::new(limits.clone(), "the document");
        large.write_all(b"12345678").unwrap();
        large.write_all(b"9abcdef").unwrap();
        assert_eq!((large.len(), limits.used()), (15, 15));
        assert_eq!(read_all(large), b"123456789abcdef");
        // Dropping gives the space back and leaves no file.
        assert_eq!(limits.used(), 0);
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn refuses_what_does_not_fit() {
        let none = Arc::new(SpoolLimits::new(4, None, 1000));
        let mut s = Spool::new(none, "the document");
        let e = s.write_all(b"12345").unwrap_err().to_string();
        assert!(e.contains("--spool-dir"), "{e}");

        let d = dir();
        let tight = Arc::new(SpoolLimits::new(4, Some(d.clone()), 8));
        let mut a = Spool::new(tight.clone(), "the document");
        a.write_all(b"123456").unwrap();
        let mut b = Spool::new(tight.clone(), "the document");
        let e = b.write_all(b"123456").unwrap_err().to_string();
        assert!(e.contains("--spool-disk"), "{e}");
        drop(b);
        drop(a);
        assert_eq!(tight.used(), 0);
        let _ = std::fs::remove_dir_all(d);
    }
}
