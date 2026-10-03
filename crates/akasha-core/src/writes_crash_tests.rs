//! Private, thread-local fault selection. Never compiled into product builds.

use std::cell::RefCell;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    Created,
    PartialWrite,
    BeforeFileSync,
    AfterFileSync,
    Published,
    BeforeDirectorySync,
    AfterDirectorySync,
}

impl Stage {
    pub(crate) const ALL: &[Self] = &[
        Self::Created,
        Self::PartialWrite,
        Self::BeforeFileSync,
        Self::AfterFileSync,
        Self::Published,
        Self::BeforeDirectorySync,
        Self::AfterDirectorySync,
    ];

    pub(crate) fn published(self) -> bool {
        matches!(
            self,
            Self::Published | Self::BeforeDirectorySync | Self::AfterDirectorySync
        )
    }
}

struct Selection {
    target: PathBuf,
    stage: Stage,
    published: bool,
}

thread_local! {
    static SELECTED: RefCell<Option<Selection>> = const { RefCell::new(None) };
}

pub(crate) fn arm(target: PathBuf, stage: Stage) {
    SELECTED.set(Some(Selection {
        target,
        stage,
        published: false,
    }));
}

pub(crate) fn interrupt_at(path: &Path, stage: Stage) {
    SELECTED.with_borrow_mut(|selected| {
        if let Some(selected) = selected
            && selected.target == path
        {
            if stage == Stage::Published {
                selected.published = true;
            }
            if selected.stage == stage {
                // No unwinding: open files, staging guards and locks are not dropped.
                std::process::exit(74);
            }
        }
    });
}

pub(crate) fn partial_write(file: &mut File, path: &Path, contents: &[u8]) -> io::Result<()> {
    let selected = SELECTED.with_borrow(|selected| {
        selected.as_ref().is_some_and(|selected| {
            selected.target == path && selected.stage == Stage::PartialWrite
        })
    });
    if selected {
        assert!(
            contents.len() > 1,
            "partial-write fixture needs nonempty halves"
        );
        file.write_all(&contents[..contents.len() / 2])?;
        interrupt_at(path, Stage::PartialWrite);
    }
    Ok(())
}

pub(crate) fn directory_sync(directory: &Path, stage: Stage) {
    SELECTED.with_borrow(|selected| {
        if let Some(selected) = selected
            && selected.published
            && selected.target.parent() == Some(directory)
            && selected.stage == stage
        {
            std::process::exit(74);
        }
    });
}
