//! Launch the actual stdio server using the descriptor returned after an empty start.
use akasha_core::{
    InitRequest, ResolutionEnvironment, ResolveRequest, apply_root_setup, initialize_project,
    prepare_onboarding_handoff, prepare_root_setup,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Client {
    child: Child,
    input: ChildStdin,
    responses: Receiver<Value>,
}
impl Client {
    fn launch(args: &[String]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_akasha-onboarding-mcp"))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (sender, responses) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                let Ok(value) = serde_json::from_str(&line) else {
                    break;
                };
                if sender.send(value).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            responses,
        }
    }
    fn send(&mut self, request: Value) {
        serde_json::to_writer(&mut self.input, &request).unwrap();
        self.input.write_all(b"\n").unwrap();
        self.input.flush().unwrap();
    }
    fn call(&mut self, id: u32, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let response = self
                .responses
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .expect("bounded stdio response");
            if response.get("id") == Some(&json!(id)) {
                assert!(response.get("error").is_none(), "{response}");
                return response["result"].clone();
            }
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

#[test]
fn fresh_setup_handoff_launches_project_scoped_read_only_stdio_tools() {
    let f =
        Fixture(std::env::temp_dir().join(format!("akasha-bootstrap-wire-{}", std::process::id())));
    fs::create_dir(&f.0).unwrap();
    let root = f.0.join("memory 世界");
    let repo = f.0.join("repository with spaces");
    fs::create_dir(&repo).unwrap();
    apply_root_setup(&prepare_root_setup(&root).unwrap()).unwrap();
    initialize_project(&InitRequest {
        root_override: Some(root.clone()),
        project: "first".into(),
        cwd: repo.clone(),
        environment: ResolutionEnvironment::default(),
    })
    .unwrap();
    let request = ResolveRequest {
        root_override: Some(root),
        project_override: None,
        cwd: repo,
        environment: ResolutionEnvironment::default(),
    };
    let handoff = prepare_onboarding_handoff(&request).unwrap();
    let before = snapshot(&f.0);
    assert_eq!(handoff.command, "akasha-onboarding-mcp");
    let mut client = Client::launch(&handoff.args);
    let init = client.call(1, "initialize", json!({"protocolVersion":"2025-03-26", "capabilities":{}, "clientInfo":{"name":"akasha-empty-start-test","version":"1"}}));
    assert!(init["capabilities"]["tools"].is_object());
    client.send(json!({"jsonrpc":"2.0", "method":"notifications/initialized"}));
    let tools = client.call(2, "tools/list", json!({}));
    let tools = tools["tools"].as_array().unwrap();
    let names: Vec<_> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 4);
    for name in [
        "akasha_onboarding_prepare",
        "akasha_onboarding_validate",
        "akasha_onboarding_preview",
        "akasha_onboarding_apply",
    ] {
        assert!(names.contains(&name));
    }
    let apply = tools
        .iter()
        .find(|tool| tool["name"] == "akasha_onboarding_apply")
        .unwrap();
    assert_eq!(apply["annotations"]["destructiveHint"], true);
    let prepared = client.call(
        3,
        "tools/call",
        json!({"name":"akasha_onboarding_prepare", "arguments":{}}),
    );
    assert_ne!(prepared["isError"], true, "{prepared}");
    assert_eq!(
        prepared["structuredContent"]["preparation"]["project"],
        "first"
    );
    assert_eq!(
        prepared["structuredContent"]["preparation"]["templates"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
    drop(client);
    assert_eq!(snapshot(&f.0), before);
}
