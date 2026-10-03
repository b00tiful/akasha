//! Returned-I/O-error injection, scoped to one test thread and removed on guard drop.
//! No environment switch, alternate filesystem, or product feature is introduced.

use std::cell::RefCell;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    Create,
    PartialWrite,
    Permissions,
    FileSync,
    Publish,
    DirectorySync,
    Remove,
}

pub(crate) struct Fault {
    path: PathBuf,
    stage: Stage,
    skip: usize,
    remaining: usize,
    kind: io::ErrorKind,
    hits: usize,
}

impl Fault {
    pub(crate) fn new(path: impl Into<PathBuf>, stage: Stage, kind: io::ErrorKind) -> Self {
        Self {
            path: path.into(),
            stage,
            skip: 0,
            remaining: 1,
            kind,
            hits: 0,
        }
    }

    pub(crate) fn skip(mut self, count: usize) -> Self {
        self.skip = count;
        self
    }

    pub(crate) fn persistent(mut self) -> Self {
        self.remaining = usize::MAX;
        self
    }
}

thread_local! {
    static FAULTS: RefCell<Vec<Fault>> = const { RefCell::new(Vec::new()) };
}

pub(crate) struct Guard;

impl Guard {
    pub(crate) fn arm(faults: Vec<Fault>) -> Self {
        FAULTS.with_borrow_mut(|current| {
            assert!(current.is_empty(), "nested I/O fault scope");
            *current = faults;
        });
        Self
    }

    pub(crate) fn hits(&self) -> Vec<usize> {
        FAULTS.with_borrow(|faults| faults.iter().map(|fault| fault.hits).collect())
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        FAULTS.with_borrow_mut(Vec::clear);
    }
}

pub(crate) fn check(path: &Path, stage: Stage) -> io::Result<()> {
    FAULTS.with_borrow_mut(|faults| {
        for fault in faults {
            if fault.path != path || fault.stage != stage || fault.remaining == 0 {
                continue;
            }
            if fault.skip > 0 {
                fault.skip -= 1;
                continue;
            }
            fault.remaining -= 1;
            fault.hits += 1;
            return Err(io::Error::new(fault.kind, "synthetic returned I/O failure"));
        }
        Ok(())
    })
}

pub(crate) fn partial_write(file: &mut File, path: &Path, contents: &[u8]) -> io::Result<()> {
    let selected = FAULTS.with_borrow(|faults| {
        faults.iter().any(|fault| {
            fault.path == path
                && fault.stage == Stage::PartialWrite
                && fault.remaining > 0
                && fault.skip == 0
        })
    });
    if selected {
        assert!(
            contents.len() > 1,
            "fixture must exercise a genuine short write"
        );
        file.write_all(&contents[..contents.len() / 2])?;
    }
    check(path, Stage::PartialWrite)
}
