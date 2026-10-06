//! Exercise real one-target wiring transactions in disposable homes, with returned I/O faults.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::writes::error_tests::{Fault, Guard, Stage};
use crate::*;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
const CLIENTS: [AgentClient; 2] = [AgentClient::Codex, AgentClient::Claude];
const KINDS: [io::ErrorKind; 3] = [
    io::ErrorKind::PermissionDenied,
    io::ErrorKind::StorageFull,
    io::ErrorKind::Other,
];

#[derive(Clone, Copy, Debug)]
enum Surface {
    Instructions,
    Hook,
}

#[derive(Debug)]
struct Failure {
    code: u8,
    message: String,
    kind: Option<io::ErrorKind>,
}

impl Failure {
    fn new(code: u8, error: impl Error + 'static) -> Self {
        let mut source: &dyn Error = &error;
        let kind = loop {
            if let Some(error) = source.downcast_ref::<io::Error>() {
                break Some(error.kind());
            }
            match source.source() {
                Some(next) => source = next,
                None => break None,
            }
        };
        Self {
            code,
            message: error.to_string(),
            kind,
        }
    }

    fn assert_io(&self, kind: io::ErrorKind) {
        assert_eq!(self.code, 6, "{}", self.message);
        assert_eq!(self.kind, Some(kind), "{}", self.message);
        assert!(self.message.contains("synthetic returned I/O failure"));
    }
}

#[derive(Debug)]
struct Completion {
    changed: bool,
    recovery: String,
}

struct Review {
    id: String,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
    action: String,
}

struct Fixture {
    base: PathBuf,
    home: PathBuf,
    request: ResolveRequest,
    client: AgentClient,
    surface: Surface,
    remove: bool,
    target: PathBuf,
    journal: PathBuf,
    lock: PathBuf,
}

impl Fixture {
    fn new(surface: Surface, client: AgentClient) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "akasha-wiring-errors-{}-{unique}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let home = base.join("home");
        let root = base.join("root");
        fs::create_dir_all(root.join("Meta")).unwrap();
        fs::create_dir(&home).unwrap();
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/resolution/valid-root");
        for path in ["akasha.toml", "Meta/AGENTS.md"] {
            fs::copy(source.join(path), root.join(path)).unwrap();
        }
        let (name, suffix) = match (surface, client) {
            (Surface::Instructions, AgentClient::Codex) => ("AGENTS.md", "agent-wiring"),
            (Surface::Instructions, AgentClient::Claude) => ("CLAUDE.md", "agent-wiring"),
            (Surface::Hook, AgentClient::Codex) => ("hooks.json", "session-hook"),
            (Surface::Hook, AgentClient::Claude) => ("settings.json", "session-hook"),
        };
        let target = home.join(name);
        let journal = home.join(format!(".{name}.akasha-{suffix}-journal.json"));
        let lock = home.join(format!(".{name}.akasha-{suffix}.lock"));
        let request = ResolveRequest {
            root_override: Some(root),
            project_override: None,
            cwd: home.clone(),
            environment: ResolutionEnvironment::default(),
        };
        Self {
            base,
            home,
            request,
            client,
            surface,
            remove: false,
            target,
            journal,
            lock,
        }
    }

    fn review(&self) -> Review {
        let plan = match self.surface {
            Surface::Instructions => {
                let plan = if self.remove {
                    prepare_agent_wiring_removal(&self.request, self.client, &self.home)
                } else {
                    prepare_agent_wiring(&self.request, self.client, &self.home)
                }
                .unwrap();
                serde_json::to_value(plan).unwrap()
            }
            Surface::Hook => {
                let plan = if self.remove {
                    prepare_session_hook_removal(&self.request, self.client, &self.home)
                } else {
                    prepare_session_hook_wiring(&self.request, self.client, &self.home)
                }
                .unwrap();
                serde_json::to_value(plan).unwrap()
            }
        };
        let before = optional_bytes(&self.target);
        let after = if plan["result_sha256"].is_null() {
            None
        } else {
            let bytes = before.as_deref().unwrap_or_default();
            let start = plan["patch"]["start"].as_u64().unwrap() as usize;
            let end = plan["patch"]["end"].as_u64().unwrap() as usize;
            Some(
                [
                    &bytes[..start],
                    plan["patch"]["replacement"].as_str().unwrap().as_bytes(),
                    &bytes[end..],
                ]
                .concat(),
            )
        };
        assert_eq!(
            plan["current_sha256"],
            serde_json::to_value(
                before
                    .as_ref()
                    .map(|b| crate::state::content_fingerprint(b))
            )
            .unwrap()
        );
        assert_eq!(
            plan["result_sha256"],
            serde_json::to_value(after.as_ref().map(|b| crate::state::content_fingerprint(b)))
                .unwrap()
        );
        Review {
            id: plan["plan_id"].as_str().unwrap().to_owned(),
            before,
            after,
            action: plan["action"].as_str().unwrap().to_owned(),
        }
    }

    fn commit(&self, id: &str) -> Result<Completion, Failure> {
        match self.surface {
            Surface::Instructions => {
                let result = if self.remove {
                    remove_agent_wiring(&self.request, self.client, &self.home, id)
                } else {
                    apply_agent_wiring(&self.request, self.client, &self.home, id)
                }
                .map_err(|e| Failure::new(e.exit_code(), e))?;
                Ok(Completion {
                    changed: result.changed,
                    recovery: format!("{:?}", result.recovery),
                })
            }
            Surface::Hook => {
                let result = if self.remove {
                    remove_session_hook_wiring(&self.request, self.client, &self.home, id)
                } else {
                    apply_session_hook_wiring(&self.request, self.client, &self.home, id)
                }
                .map_err(|e| Failure::new(e.exit_code(), e))?;
                Ok(Completion {
                    changed: result.changed,
                    recovery: format!("{:?}", result.recovery),
                })
            }
        }
    }

    fn assert_unlocked(&self) {
        if self.lock.exists() {
            let file = File::options()
                .read(true)
                .write(true)
                .open(&self.lock)
                .unwrap();
            file.try_lock()
                .expect("writer scope must release the target lock");
            file.unlock().unwrap();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn optional_bytes(path: &Path) -> Option<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => panic!("read {}: {e}", path.display()),
    }
}

fn persistent_recovery_sync(surface: Surface) {
    for client in CLIENTS {
        let fixture = Fixture::new(surface, client);
        let review = fixture.review();
        // First sync publishes the lock; second sync publishes the journal.
        let guard = Guard::arm(vec![
            Fault::new(&fixture.home, Stage::DirectorySync, io::ErrorKind::Other)
                .skip(2)
                .persistent(),
        ]);
        let error = fixture
            .commit(&review.id)
            .expect_err("persistent sync must refuse");
        error.assert_io(io::ErrorKind::Other);
        assert_eq!(optional_bytes(&fixture.target), review.after);
        assert!(
            fixture.journal.is_file(),
            "failed sync must retain recovery authority"
        );
        let journal = fs::read(&fixture.journal).unwrap();
        for _ in 0..2 {
            fixture
                .commit(&review.id)
                .unwrap_err()
                .assert_io(io::ErrorKind::Other);
            assert_eq!(fs::read(&fixture.journal).unwrap(), journal);
            assert_eq!(optional_bytes(&fixture.target), review.after);
            fixture.assert_unlocked();
        }
        assert_eq!(guard.hits(), [4]);
        drop(guard);
        let result = fixture
            .commit(&review.id)
            .expect("finalize after filesystem is corrected");
        assert_eq!(result.recovery, "Finalized");
        assert!(result.changed);
        assert!(!fixture.journal.exists());
        fixture.assert_unlocked();
    }
}

#[test]
fn recovery_retries_instruction_directory_sync_before_journal_removal() {
    persistent_recovery_sync(Surface::Instructions);
}

#[test]
fn recovery_retries_hook_directory_sync_before_journal_removal() {
    persistent_recovery_sync(Surface::Hook);
}

#[derive(Clone, Copy)]
struct Case {
    surface: Surface,
    action: &'static str,
    seed: Option<&'static str>,
    remove: bool,
}

const HUMAN_INSTRUCTIONS: &str = "# Human guidance\r\n\r\nPreserve café / 雪 exactly.\r\n";
const HUMAN_SETTINGS: &str = "{\r\n  \"theme\": \"café 雪\"\r\n}\r\n";
const OTHER_HOOK: &str = "{\"theme\":\"human\",\"hooks\":{\"Other\":[]}}\n";
const HUMAN_SESSION_START: &str = "{\"theme\":\"human\",\"hooks\":{\"SessionStart\":[{\"matcher\":\"startup\",\"hooks\":[{\"type\":\"command\",\"command\":\"echo human\"}]}]}}\n";

const CASES: [Case; 13] = [
    Case {
        surface: Surface::Instructions,
        action: "create",
        seed: None,
        remove: false,
    },
    Case {
        surface: Surface::Instructions,
        action: "append",
        seed: Some(HUMAN_INSTRUCTIONS),
        remove: false,
    },
    Case {
        surface: Surface::Instructions,
        action: "refresh-managed-section",
        seed: Some(HUMAN_INSTRUCTIONS),
        remove: false,
    },
    Case {
        surface: Surface::Instructions,
        action: "remove-managed-section",
        seed: Some(HUMAN_INSTRUCTIONS),
        remove: true,
    },
    Case {
        surface: Surface::Instructions,
        action: "remove-created-file",
        seed: None,
        remove: true,
    },
    Case {
        surface: Surface::Hook,
        action: "create",
        seed: None,
        remove: false,
    },
    Case {
        surface: Surface::Hook,
        action: "add-hooks",
        seed: Some(HUMAN_SETTINGS),
        remove: false,
    },
    Case {
        surface: Surface::Hook,
        action: "add-session-start",
        seed: Some(OTHER_HOOK),
        remove: false,
    },
    Case {
        surface: Surface::Hook,
        action: "append-session-start",
        seed: Some(HUMAN_SESSION_START),
        remove: false,
    },
    Case {
        surface: Surface::Hook,
        action: "remove-session-start-entry",
        seed: Some(HUMAN_SESSION_START),
        remove: true,
    },
    Case {
        surface: Surface::Hook,
        action: "remove-session-start-key",
        seed: Some(OTHER_HOOK),
        remove: true,
    },
    Case {
        surface: Surface::Hook,
        action: "remove-hooks-key",
        seed: Some(HUMAN_SETTINGS),
        remove: true,
    },
    Case {
        surface: Surface::Hook,
        action: "remove-managed-file",
        seed: None,
        remove: true,
    },
];

fn cases() -> impl Iterator<Item = (Case, AgentClient)> {
    CASES.into_iter().flat_map(|case| {
        CLIENTS.into_iter().filter_map(move |client| {
            // Claude's absolute import stays live; changed source bytes need no target refresh.
            (!(case.action == "refresh-managed-section" && client == AgentClient::Claude))
                .then_some((case, client))
        })
    })
}

fn fixture_for(case: Case, client: AgentClient) -> (Fixture, Review) {
    let mut fixture = Fixture::new(case.surface, client);
    if let Some(seed) = case.seed {
        fs::write(&fixture.target, seed).unwrap();
        set_mode(&fixture.target, 0o640);
    }
    if case.remove || case.action == "refresh-managed-section" {
        let initial = fixture.review();
        fixture.commit(&initial.id).unwrap();
    }
    if case.action == "refresh-managed-section" {
        fs::write(
            fixture
                .request
                .root_override
                .as_ref()
                .unwrap()
                .join("Meta/AGENTS.md"),
            "# Changed canonical guidance\nRead the current project context.\n",
        )
        .unwrap();
    }
    fixture.remove = case.remove;
    fs::write(
        fixture.home.join("human-private.txt"),
        "private café / 雪\r\n",
    )
    .unwrap();
    set_mode(&fixture.home.join("human-private.txt"), 0o640);
    // Precreate only the empty persistent lock, so sync occurrence selection is consistent.
    if !fixture.lock.exists() {
        crate::create_file_atomically(&fixture.lock, b"").unwrap();
        crate::writes::sync_directory(&fixture.home).unwrap();
    }
    let review = fixture.review();
    assert_eq!(review.action, case.action);
    (fixture, review)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn mode(_path: &Path) -> u32 {
    0
}

#[derive(Debug, PartialEq, Eq)]
enum Image {
    File(Vec<u8>, u32),
    Link(PathBuf),
    Directory(u32),
}

fn snapshot(directory: &Path) -> BTreeMap<PathBuf, Image> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            let image = if metadata.file_type().is_symlink() {
                Image::Link(fs::read_link(&path).unwrap())
            } else if metadata.is_dir() {
                Image::Directory(mode(&path))
            } else {
                Image::File(fs::read(&path).unwrap(), mode(&path))
            };
            (path, image)
        })
        .collect()
}

fn assert_exact_target(fixture: &Fixture, expected: &Option<Vec<u8>>, expected_mode: u32) {
    assert_eq!(optional_bytes(&fixture.target), *expected);
    if expected.is_some() {
        assert!(fs::symlink_metadata(&fixture.target).unwrap().is_file());
        #[cfg(unix)]
        assert_eq!(mode(&fixture.target), expected_mode);
        #[cfg(not(unix))]
        let _ = expected_mode;
    }
    assert_eq!(
        fs::read(fixture.home.join("human-private.txt")).unwrap(),
        "private café / 雪\r\n".as_bytes()
    );
    #[cfg(unix)]
    assert_eq!(mode(&fixture.home.join("human-private.txt")), 0o640);
    fixture.assert_unlocked();
}

fn finish(fixture: &Fixture, review: &Review, expected_mode: u32) {
    // Preview remains read-only, even with an outstanding matching journal.
    let before_review = snapshot(&fixture.home);
    let fresh = fixture.review();
    assert_eq!(snapshot(&fixture.home), before_review);
    let id = if fixture.journal.exists() {
        &review.id
    } else {
        &fresh.id
    };
    fixture.commit(id).unwrap();
    assert_exact_target(fixture, &review.after, expected_mode);
    assert!(!fixture.journal.exists());
    let completed = snapshot(&fixture.home);
    let current = fixture.review();
    assert_eq!(current.action, "no-change");
    assert!(!fixture.commit(&current.id).unwrap().changed);
    assert_eq!(snapshot(&fixture.home), completed);
}

#[test]
fn wiring_publication_errors_cover_all_actions_clients_and_original_error_kinds() {
    let mut count = 0;
    for (case, client) in cases() {
        for kind in KINDS {
            for location in ["journal", "target"] {
                for stage in [
                    Stage::Create,
                    Stage::PartialWrite,
                    Stage::Permissions,
                    Stage::FileSync,
                    Stage::Publish,
                    Stage::Remove,
                ] {
                    let (fixture, review) = fixture_for(case, client);
                    let path = if location == "journal" {
                        &fixture.journal
                    } else {
                        &fixture.target
                    };
                    let deletion = location == "target" && review.after.is_none();
                    if (deletion && stage != Stage::Remove)
                        || (!deletion && stage == Stage::Remove)
                        || (stage == Stage::Permissions
                            && (location == "journal" || review.before.is_none()))
                    {
                        continue;
                    }
                    let expected_mode = review
                        .before
                        .as_ref()
                        .map_or(0o600, |_| mode(&fixture.target));
                    let guard = Guard::arm(vec![Fault::new(path, stage, kind)]);
                    fixture.commit(&review.id).unwrap_err().assert_io(kind);
                    assert_eq!(
                        guard.hits(),
                        [1],
                        "{} {client:?} {location} {stage:?}",
                        case.action
                    );
                    assert_exact_target(&fixture, &review.before, expected_mode);
                    assert!(!fixture.journal.exists());
                    assert!(
                        !snapshot(&fixture.home)
                            .keys()
                            .any(|p| p.extension().is_some_and(|s| s == "tmp"))
                    );
                    drop(guard);
                    finish(&fixture, &review, expected_mode);
                    count += 1;
                }
            }
        }
    }
    assert_eq!(count, 615);
    eprintln!("wiring publication returned-I/O scenarios: {count}");
}

#[test]
fn wiring_lock_creation_errors_release_authority_and_preserve_existing_home_bytes() {
    let mut count = 0;
    for surface in [Surface::Instructions, Surface::Hook] {
        for client in CLIENTS {
            for kind in KINDS {
                for stage in [
                    Stage::Create,
                    Stage::FileSync,
                    Stage::Publish,
                    Stage::DirectorySync,
                ] {
                    let fixture = Fixture::new(surface, client);
                    let review = fixture.review();
                    let path = if stage == Stage::DirectorySync {
                        &fixture.home
                    } else {
                        &fixture.lock
                    };
                    let guard = Guard::arm(vec![Fault::new(path, stage, kind)]);
                    fixture.commit(&review.id).unwrap_err().assert_io(kind);
                    assert_eq!(guard.hits(), [1]);
                    assert!(!fixture.target.exists());
                    assert!(!fixture.journal.exists());
                    fixture.assert_unlocked();
                    if fixture.lock.exists() {
                        assert!(fs::read(&fixture.lock).unwrap().is_empty());
                        #[cfg(unix)]
                        assert_eq!(mode(&fixture.lock), 0o600);
                    }
                    drop(guard);
                    fixture.commit(&review.id).unwrap();
                    assert_eq!(optional_bytes(&fixture.target), review.after);
                    count += 1;
                }
            }
        }
    }
    assert_eq!(count, 48);
    eprintln!("wiring lock returned-I/O scenarios: {count}");
}

#[test]
fn wiring_transient_sync_and_journal_cleanup_failures_finish_or_report_exact_state() {
    let mut count = 0;
    for (case, client) in cases() {
        for kind in KINDS {
            for fault_at in 0..4 {
                let (fixture, review) = fixture_for(case, client);
                let expected_mode = review
                    .before
                    .as_ref()
                    .map_or(0o600, |_| mode(&fixture.target));
                let fault = if fault_at == 3 {
                    Fault::new(&fixture.journal, Stage::Remove, kind)
                } else {
                    Fault::new(&fixture.home, Stage::DirectorySync, kind).skip(fault_at)
                };
                let guard = Guard::arm(vec![fault]);
                let result = fixture.commit(&review.id);
                assert_eq!(guard.hits(), [1]);
                match fault_at {
                    0 => {
                        result.unwrap_err().assert_io(kind);
                        assert_exact_target(&fixture, &review.before, expected_mode);
                        assert!(fixture.journal.is_file());
                    }
                    1 | 3 => {
                        let result = result.unwrap();
                        assert_eq!(result.recovery, "Finalized");
                        assert!(result.changed);
                        assert_exact_target(&fixture, &review.after, expected_mode);
                        assert!(!fixture.journal.exists());
                    }
                    2 => {
                        result.unwrap_err().assert_io(kind);
                        assert_exact_target(&fixture, &review.after, expected_mode);
                        assert!(!fixture.journal.exists());
                    }
                    _ => unreachable!(),
                }
                drop(guard);
                finish(&fixture, &review, expected_mode);
                count += 1;
            }
        }
    }
    assert_eq!(count, 300);
    eprintln!("wiring transient sync/cleanup scenarios: {count}");
}

#[test]
fn wiring_persistent_sync_and_cleanup_failures_preserve_recovery_authority() {
    let mut count = 0;
    for (case, client) in cases() {
        for kind in KINDS {
            for fault_at in 0..4 {
                let (fixture, review) = fixture_for(case, client);
                let expected_mode = review
                    .before
                    .as_ref()
                    .map_or(0o600, |_| mode(&fixture.target));
                let fault = if fault_at == 3 {
                    Fault::new(&fixture.journal, Stage::Remove, kind).persistent()
                } else {
                    Fault::new(&fixture.home, Stage::DirectorySync, kind)
                        .skip(fault_at)
                        .persistent()
                };
                let guard = Guard::arm(vec![fault]);
                fixture.commit(&review.id).unwrap_err().assert_io(kind);
                let expected = if fault_at == 0 {
                    &review.before
                } else {
                    &review.after
                };
                assert_exact_target(&fixture, expected, expected_mode);
                if fault_at == 2 {
                    // Unlink already completed: an error is visible but no journal remains.
                    assert!(!fixture.journal.exists());
                } else {
                    assert!(fixture.journal.is_file());
                    let pending = snapshot(&fixture.home);
                    for attempt in 0..2 {
                        let current = fixture.review();
                        let id = if attempt == 0 {
                            &review.id
                        } else {
                            &current.id
                        };
                        fixture.commit(id).unwrap_err().assert_io(kind);
                        assert_eq!(snapshot(&fixture.home), pending);
                        fixture.assert_unlocked();
                    }
                    assert_eq!(guard.hits(), [if fault_at == 0 { 3 } else { 4 }]);
                    #[cfg(unix)]
                    assert_eq!(mode(&fixture.journal), 0o600);
                    let journal = String::from_utf8(fs::read(&fixture.journal).unwrap()).unwrap();
                    assert!(!journal.contains("café"));
                    assert!(!journal.contains("Akasha project memory"));
                    assert!(!journal.contains("echo human"));
                }
                drop(guard);
                finish(&fixture, &review, expected_mode);
                count += 1;
            }
        }
    }
    assert_eq!(count, 300);
    eprintln!("wiring persistent sync/cleanup scenarios: {count}");
}

#[test]
fn wiring_compound_publication_and_recovery_errors_keep_both_failures_and_exact_preimages() {
    let mut count = 0;
    for (case, client) in cases() {
        for kind in KINDS {
            let (fixture, review) = fixture_for(case, client);
            let expected_mode = review
                .before
                .as_ref()
                .map_or(0o600, |_| mode(&fixture.target));
            let stage = if review.after.is_none() {
                Stage::Remove
            } else {
                Stage::PartialWrite
            };
            let guard = Guard::arm(vec![
                Fault::new(&fixture.target, stage, kind),
                Fault::new(&fixture.journal, Stage::Remove, kind).persistent(),
            ]);
            let error = fixture.commit(&review.id).unwrap_err();
            error.assert_io(kind);
            assert!(error.message.contains("automatic recovery also failed"));
            assert_eq!(
                error
                    .message
                    .matches("synthetic returned I/O failure")
                    .count(),
                2
            );
            assert_exact_target(&fixture, &review.before, expected_mode);
            let pending = snapshot(&fixture.home);
            fixture.commit(&review.id).unwrap_err().assert_io(kind);
            assert_eq!(snapshot(&fixture.home), pending);
            assert_eq!(guard.hits(), [1, 2]);
            fixture.assert_unlocked();
            drop(guard);
            finish(&fixture, &review, expected_mode);
            count += 1;
        }
    }
    assert_eq!(count, 75);
    eprintln!("wiring compound publication/recovery scenarios: {count}");
}

#[test]
fn wiring_failed_stage_cleanup_preserves_private_residue_without_adopting_it() {
    let mut count = 0;
    for (case, client) in cases() {
        for kind in KINDS {
            for stage in [Stage::PartialWrite, Stage::Publish] {
                let (fixture, review) = fixture_for(case, client);
                let Some(after) = &review.after else { continue };
                let expected_mode = review
                    .before
                    .as_ref()
                    .map_or(0o600, |_| mode(&fixture.target));
                let prefix = fixture.home.join(format!(
                    ".{}.akasha-{}-",
                    fixture.target.file_name().unwrap().to_str().unwrap(),
                    std::process::id()
                ));
                let guard = Guard::arm(vec![
                    Fault::new(&fixture.target, stage, kind),
                    Fault::new(prefix, Stage::Remove, kind)
                        .file_name_prefix()
                        .persistent(),
                ]);
                fixture.commit(&review.id).unwrap_err().assert_io(kind);
                assert_eq!(guard.hits(), [1, 1]);
                assert_exact_target(&fixture, &review.before, expected_mode);
                assert!(!fixture.journal.exists());
                let residue = snapshot(&fixture.home)
                    .into_iter()
                    .filter(|(path, _)| {
                        path.extension().is_some_and(|extension| extension == "tmp")
                    })
                    .collect::<BTreeMap<_, _>>();
                assert_eq!(residue.len(), 1);
                let (path, image) = residue.first_key_value().unwrap();
                let Image::File(bytes, permissions) = image else {
                    panic!("stage must be regular")
                };
                let expected = if stage == Stage::PartialWrite {
                    &after[..after.len() / 2]
                } else {
                    after
                };
                assert_eq!(bytes, expected);
                #[cfg(unix)]
                assert_eq!(
                    *permissions,
                    if stage == Stage::PartialWrite {
                        0o600
                    } else {
                        expected_mode
                    }
                );
                #[cfg(not(unix))]
                let _ = permissions;
                drop(guard);
                finish(&fixture, &review, expected_mode);
                assert_eq!(snapshot(&fixture.home).get(path), Some(image));
                count += 1;
            }
        }
    }
    assert_eq!(count, 126);
    eprintln!("wiring retained private staging scenarios: {count}");
}

#[cfg(unix)]
#[test]
fn wiring_recovery_refuses_foreign_bytes_symlinks_and_types_before_any_sync_or_unlink() {
    use std::os::unix::fs::symlink;
    let mut count = 0;
    for (case, client) in cases() {
        for change in 0..6 {
            let (fixture, review) = fixture_for(case, client);
            let guard = Guard::arm(vec![
                Fault::new(&fixture.journal, Stage::Remove, io::ErrorKind::Other).persistent(),
            ]);
            fixture
                .commit(&review.id)
                .unwrap_err()
                .assert_io(io::ErrorKind::Other);
            drop(guard);
            let journal = fs::read(&fixture.journal).unwrap();
            let selected = if change < 3 {
                &fixture.target
            } else {
                &fixture.journal
            };
            if selected.exists() {
                fs::remove_file(selected).unwrap();
            }
            let outside = fixture.base.join("foreign-private.txt");
            fs::write(&outside, "external café / 雪\r\n").unwrap();
            set_mode(&outside, 0o640);
            match change % 3 {
                0 => fs::write(selected, b"unexpected bytes\n").unwrap(),
                1 => symlink(&outside, selected).unwrap(),
                2 => fs::create_dir(selected).unwrap(),
                _ => unreachable!(),
            }
            let conflict = snapshot(&fixture.home);
            let guard = Guard::arm(vec![
                Fault::new(&fixture.home, Stage::DirectorySync, io::ErrorKind::Other).persistent(),
                Fault::new(&fixture.journal, Stage::Remove, io::ErrorKind::Other).persistent(),
            ]);
            for _ in 0..2 {
                let error = fixture.commit(&review.id).unwrap_err();
                assert_eq!(error.code, 5, "{}", error.message);
                assert_eq!(snapshot(&fixture.home), conflict);
                fixture.assert_unlocked();
            }
            assert_eq!(guard.hits(), [0, 0]);
            assert_eq!(
                fs::read(&outside).unwrap(),
                "external café / 雪\r\n".as_bytes()
            );
            assert_eq!(mode(&outside), 0o640);
            drop(guard);
            if fs::symlink_metadata(selected).unwrap().is_dir() {
                fs::remove_dir(selected).unwrap();
            } else {
                fs::remove_file(selected).unwrap();
            }
            if change < 3 {
                if let Some(after) = &review.after {
                    fs::write(&fixture.target, after).unwrap();
                    set_mode(
                        &fixture.target,
                        if case.seed.is_some() { 0o640 } else { 0o600 },
                    );
                }
            } else {
                fs::write(&fixture.journal, &journal).unwrap();
                set_mode(&fixture.journal, 0o600);
            }
            let result = fixture.commit(&review.id).unwrap();
            assert_eq!(result.recovery, "Finalized");
            assert_eq!(optional_bytes(&fixture.target), review.after);
            assert!(!fixture.journal.exists());
            fixture.assert_unlocked();
            count += 1;
        }
    }
    assert_eq!(count, 150);
    eprintln!("wiring foreign-state refusal/reconciliation scenarios: {count}");
}

#[test]
fn wiring_noop_reviews_and_replay_do_not_hide_pending_transactions_or_rewrite_bytes() {
    let mut count = 0;
    for (case, client) in cases() {
        let (fixture, review) = fixture_for(case, client);
        fixture.commit(&review.id).unwrap();
        let complete = snapshot(&fixture.home);
        let current = fixture.review();
        assert_eq!(current.action, "no-change");
        let guard = Guard::arm(vec![
            Fault::new(&fixture.home, Stage::DirectorySync, io::ErrorKind::Other).persistent(),
        ]);
        assert!(!fixture.commit(&current.id).unwrap().changed);
        assert_eq!(guard.hits(), [0]);
        assert_eq!(snapshot(&fixture.home), complete);
        let error = fixture.commit(&review.id).unwrap_err();
        assert_eq!(error.code, 5);
        assert_eq!(snapshot(&fixture.home), complete);
        fixture.assert_unlocked();
        count += 1;
    }
    assert_eq!(count, 25);
    eprintln!("wiring exact no-op/stale-replay scenarios: {count}");
}
