use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::LibraryScope;
use serde_json::{Map, Value, json};

const SCHEMA_VERSION: u64 = 1;
const MAX_STATE_BYTES: u64 = 64 * 1024;
const MAX_VALUE_CHARS: usize = 4096;
const STATE_FILE: &str = "tui-navigation-v1.json";
static NEXT_STAGING_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum NavigationLocation {
    Projects {
        selected_scope: Option<LibraryScope>,
    },
    Categories {
        scope: LibraryScope,
        selected_note_type: Option<String>,
    },
    Notes {
        scope: LibraryScope,
        note_type: String,
        selected_note: Option<String>,
    },
    Note {
        scope: LibraryScope,
        note_type: String,
        id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NavigationState {
    root_key: String,
    pub location: NavigationLocation,
}

impl NavigationState {
    pub fn new(root: &Path, location: NavigationLocation) -> Self {
        Self {
            root_key: path_key(root),
            location,
        }
    }

    pub fn belongs_to(&self, root: &Path) -> bool {
        self.root_key == path_key(root)
    }

    fn to_json(&self) -> Value {
        let (view, scope, note_type, note_id, selected) = match &self.location {
            NavigationLocation::Projects { selected_scope } => (
                "projects",
                selected_scope.as_ref().map(scope_json),
                None,
                None,
                None,
            ),
            NavigationLocation::Categories {
                scope,
                selected_note_type,
            } => (
                "categories",
                Some(scope_json(scope)),
                None,
                None,
                selected_note_type.clone(),
            ),
            NavigationLocation::Notes {
                scope,
                note_type,
                selected_note,
            } => (
                "notes",
                Some(scope_json(scope)),
                Some(note_type.clone()),
                None,
                selected_note.clone(),
            ),
            NavigationLocation::Note {
                scope,
                note_type,
                id,
            } => (
                "note",
                Some(scope_json(scope)),
                Some(note_type.clone()),
                Some(id.clone()),
                None,
            ),
        };
        json!({
            "schema_version": SCHEMA_VERSION,
            "root_key": self.root_key,
            "view": view,
            "scope": scope,
            "note_type": note_type,
            "note_id": note_id,
            "selected": selected,
        })
    }

    fn from_json(value: Value) -> Result<Self, String> {
        let object = exact_object(
            value,
            &[
                "schema_version",
                "root_key",
                "view",
                "scope",
                "note_type",
                "note_id",
                "selected",
            ],
            "navigation state",
        )?;
        if object.get("schema_version").and_then(Value::as_u64) != Some(SCHEMA_VERSION) {
            return Err("unsupported navigation-state schema version".into());
        }
        let root_key = required_string(&object, "root_key")?;
        if root_key.len() % 2 != 0 || !root_key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("navigation state root_key is not hexadecimal".into());
        }
        let view = required_string(&object, "view")?;
        let scope = optional_scope(&object, "scope")?;
        let note_type = optional_string(&object, "note_type")?;
        let note_id = optional_string(&object, "note_id")?;
        let selected = optional_string(&object, "selected")?;
        let location = match view.as_str() {
            "projects" if note_type.is_none() && note_id.is_none() && selected.is_none() => {
                NavigationLocation::Projects {
                    selected_scope: scope,
                }
            }
            "categories" if scope.is_some() && note_type.is_none() && note_id.is_none() => {
                NavigationLocation::Categories {
                    scope: scope.expect("guarded above"),
                    selected_note_type: selected,
                }
            }
            "notes" if scope.is_some() && note_type.is_some() && note_id.is_none() => {
                NavigationLocation::Notes {
                    scope: scope.expect("guarded above"),
                    note_type: note_type.expect("guarded above"),
                    selected_note: selected,
                }
            }
            "note"
                if scope.is_some()
                    && note_type.is_some()
                    && note_id.is_some()
                    && selected.is_none() =>
            {
                NavigationLocation::Note {
                    scope: scope.expect("guarded above"),
                    note_type: note_type.expect("guarded above"),
                    id: note_id.expect("guarded above"),
                }
            }
            "projects" | "categories" | "notes" | "note" => {
                return Err("navigation state fields do not match its view".into());
            }
            _ => return Err("navigation state has an unknown view".into()),
        };
        Ok(Self { root_key, location })
    }
}

pub(super) fn state_path() -> Option<PathBuf> {
    state_path_from(
        std::env::var_os("XDG_STATE_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

fn state_path_from(xdg_state_home: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    xdg_state_home
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            home.filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".local/state"))
        })
        .map(|base| base.join("akasha").join(STATE_FILE))
}

pub(super) fn load(path: &Path) -> Result<Option<NavigationState>, String> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "could not read navigation state {}: {error}",
                path.display()
            ));
        }
    };
    let metadata = file.metadata().map_err(|error| {
        format!(
            "could not inspect navigation state {}: {error}",
            path.display()
        )
    })?;
    if metadata.len() > MAX_STATE_BYTES {
        return Err(format!(
            "navigation state {} exceeds {MAX_STATE_BYTES} bytes",
            path.display()
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes).map_err(|error| {
        format!(
            "could not read navigation state {}: {error}",
            path.display()
        )
    })?;
    let value = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "could not parse navigation state {}: {error}",
            path.display()
        )
    })?;
    NavigationState::from_json(value)
        .map(Some)
        .map_err(|error| format!("invalid navigation state {}: {error}", path.display()))
}

pub(super) fn save(path: &Path, state: &NavigationState) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "navigation-state path has no parent directory",
        )
    })?;
    create_private_directory(parent)?;
    let mut bytes = serde_json::to_vec_pretty(&state.to_json()).map_err(io::Error::other)?;
    bytes.push(b'\n');

    let (staging_path, mut staging) = create_staging_file(parent)?;
    let result = (|| {
        staging.write_all(&bytes)?;
        staging.sync_all()?;
        fs::rename(&staging_path, path)?;
        File::open(parent)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staging_path);
    }
    result
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)
    }
}

fn create_staging_file(parent: &Path) -> io::Result<(PathBuf, File)> {
    for _ in 0..16 {
        let id = NEXT_STAGING_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".{STATE_FILE}.{}.{}.tmp", std::process::id(), id));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a navigation-state staging file",
    ))
}

fn scope_json(scope: &LibraryScope) -> Value {
    match scope {
        LibraryScope::Global => json!({ "kind": "global" }),
        LibraryScope::Project { project } => {
            json!({ "kind": "project", "project": project })
        }
    }
}

fn optional_scope(object: &Map<String, Value>, name: &str) -> Result<Option<LibraryScope>, String> {
    let Some(value) = object.get(name) else {
        return Err(format!("navigation state is missing {name}"));
    };
    if value.is_null() {
        return Ok(None);
    }
    let scope = exact_object(value.clone(), &["kind", "project"], "navigation scope")
        .or_else(|_| exact_object(value.clone(), &["kind"], "navigation scope"))?;
    let kind = required_string(&scope, "kind")?;
    match kind.as_str() {
        "global" if scope.len() == 1 => Ok(Some(LibraryScope::Global)),
        "project" if scope.len() == 2 => Ok(Some(LibraryScope::Project {
            project: required_string(&scope, "project")?,
        })),
        "global" | "project" => Err("navigation scope fields do not match its kind".into()),
        _ => Err("navigation state has an unknown scope kind".into()),
    }
}

fn exact_object(value: Value, fields: &[&str], label: &str) -> Result<Map<String, Value>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))?;
    if object.len() != fields.len() || fields.iter().any(|field| !object.contains_key(*field)) {
        return Err(format!("{label} has missing or unknown fields"));
    }
    Ok(object.clone())
}

fn required_string(object: &Map<String, Value>, name: &str) -> Result<String, String> {
    let value = object
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("navigation state {name} must be a string"))?;
    validate_string(value, name).map(str::to_owned)
}

fn optional_string(object: &Map<String, Value>, name: &str) -> Result<Option<String>, String> {
    match object.get(name) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => validate_string(value, name).map(|value| Some(value.into())),
        Some(_) => Err(format!("navigation state {name} must be a string or null")),
        None => Err(format!("navigation state is missing {name}")),
    }
}

fn validate_string<'a>(value: &'a str, name: &str) -> Result<&'a str, String> {
    if value.is_empty() || value.chars().count() > MAX_VALUE_CHARS {
        return Err(format!(
            "navigation state {name} must contain 1..={MAX_VALUE_CHARS} characters"
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(format!(
            "navigation state {name} contains control characters"
        ));
    }
    Ok(value)
}

fn path_key(path: &Path) -> String {
    path.as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(String::new(), |mut encoded, byte| {
            use std::fmt::Write;
            write!(encoded, "{byte:02x}").expect("writing to a string cannot fail");
            encoded
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn state_path_uses_absolute_xdg_then_home_fallback() {
        assert_eq!(
            state_path_from(Some(OsStr::new("/tmp/state")), Some(OsStr::new("/home/me"))),
            Some(PathBuf::from("/tmp/state/akasha/tui-navigation-v1.json"))
        );
        assert_eq!(
            state_path_from(Some(OsStr::new("relative")), Some(OsStr::new("/home/me"))),
            Some(PathBuf::from(
                "/home/me/.local/state/akasha/tui-navigation-v1.json"
            ))
        );
        assert_eq!(state_path_from(None, None), None);
    }

    #[test]
    fn exact_state_round_trips_through_private_atomic_file() {
        let temp = test_dir();
        let path = temp.join("state/akasha").join(STATE_FILE);
        let state = NavigationState::new(
            Path::new("/vault"),
            NavigationLocation::Note {
                scope: LibraryScope::Project {
                    project: "example".into(),
                },
                note_type: "entity".into(),
                id: "Projects/example/entities/core.md".into(),
            },
        );

        save(&path, &state).unwrap();

        assert_eq!(load(&path).unwrap(), Some(state));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn malformed_or_oversized_state_is_rejected() {
        let temp = test_dir();
        let path = temp.join(STATE_FILE);
        fs::write(&path, br#"{"schema_version":1,"extra":true}"#).unwrap();
        assert!(
            load(&path)
                .unwrap_err()
                .contains("missing or unknown fields")
        );
        fs::write(&path, vec![b'x'; MAX_STATE_BYTES as usize + 1]).unwrap();
        assert!(load(&path).unwrap_err().contains("exceeds"));
        fs::remove_dir_all(temp).unwrap();
    }

    fn test_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "akasha-tui-state-{}-{}",
            std::process::id(),
            NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }
}
