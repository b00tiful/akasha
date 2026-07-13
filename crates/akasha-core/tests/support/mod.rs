use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

pub fn write_project_state(
    project_dir: &Path,
    events: &[&str],
    index_sources: &[&str],
    roadmap_sources: &[&str],
) {
    let event_hashes = events
        .iter()
        .map(|path| {
            (
                *path,
                content_fingerprint(&fs::read(project_dir.join(path)).expect("read event")),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let index_sources = projection_sources(project_dir, index_sources);
    let roadmap_sources = projection_sources(project_dir, roadmap_sources);
    let index_output = content_fingerprint(
        &fs::read(project_dir.join("index.md")).expect("read index projection"),
    );
    let roadmap_output = content_fingerprint(
        &fs::read(project_dir.join("roadmap.md")).expect("read roadmap projection"),
    );

    let mut source = String::from("schema_version = 1\n\n[events]\n");
    for (path, fingerprint) in event_hashes {
        source.push_str(&format!(
            "{} = {fingerprint:?}\n",
            serde_json::to_string(path).expect("render state path")
        ));
    }
    source.push_str(&format!(
        "\n[projections.index]\nsources = {index_sources:?}\noutput = {index_output:?}\n\n\
         [projections.roadmap]\nsources = {roadmap_sources:?}\noutput = {roadmap_output:?}\n"
    ));
    fs::write(project_dir.join(".akasha-state.toml"), source).expect("write project state");
}

fn projection_sources(project_dir: &Path, paths: &[&str]) -> String {
    let sources = paths
        .iter()
        .map(|path| {
            (
                *path,
                content_fingerprint(
                    &fs::read(project_dir.join(path)).expect("read projection source"),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    content_fingerprint(&serde_json::to_vec(&sources).expect("render projection sources"))
}

fn content_fingerprint(source: &[u8]) -> String {
    let digest = Sha256::digest(source);
    let mut fingerprint = String::with_capacity(71);
    fingerprint.push_str("sha256:");
    for byte in digest {
        write!(fingerprint, "{byte:02x}").expect("writing to a string cannot fail");
    }
    fingerprint
}
