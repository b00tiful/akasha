// Synthetic complete scaffold plus exact version-1 journal; never stages live data.
use akasha_core::{InitPlan, InitRequest, ResolutionEnvironment, prepare_project_init};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

pub fn stage(root: &Path, repository: &Path, project: &str, committed: bool) -> InitPlan {
    fs::create_dir_all(repository).unwrap();
    let plan = prepare_project_init(&InitRequest {
        root_override: Some(root.into()),
        project: project.into(),
        cwd: repository.into(),
        environment: ResolutionEnvironment::default(),
    })
    .unwrap();
    let destination = &plan.destination;
    fs::create_dir(&destination.project_dir).unwrap();
    for directory in &plan.directories {
        fs::create_dir(destination.project_dir.join(directory)).unwrap();
    }
    for (path, bytes) in &plan.files {
        fs::write(destination.project_dir.join(path), bytes).unwrap();
    }
    fs::write(&destination.pointer, &plan.pointer_source).unwrap();
    let fingerprint = |bytes: &[u8]| {
        format!(
            "sha256:{}",
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        )
    };
    let value = serde_json::json!({
        "schema_version": 1, "project": project,
        "project_dir": destination.project_dir, "repository_dir": destination.repository_dir,
        "directories": plan.directories,
        "files": plan.files.iter().map(|(path, bytes)| serde_json::json!({"path": path, "after": fingerprint(bytes)})).collect::<Vec<_>>(),
        "pointer_after": fingerprint(&plan.pointer_source),
        "registry_before": fingerprint(&plan.registry_before), "registry_after": fingerprint(&plan.registry_after),
        "template_files": destination.template_files,
    });
    let name = destination.registry.file_name().unwrap().to_str().unwrap();
    let parent = destination.registry.parent().unwrap();
    fs::write(
        parent.join(format!(".{name}.akasha-init-journal.json")),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    fs::write(parent.join(format!(".{name}.akasha-init.lock")), []).unwrap();
    if committed {
        fs::write(&destination.registry, &plan.registry_after).unwrap();
    }
    plan
}
