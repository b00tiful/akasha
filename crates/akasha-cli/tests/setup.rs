use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "akasha-cli-setup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_akasha"))
            .args(args)
            .current_dir(&self.0)
            .env_remove("AKASHA_ROOT")
            .env("XDG_CONFIG_HOME", self.0.join("unconfigured"))
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn setup_plain_json_confirmation_and_offline_onboarding_walkthrough() {
    let f = Fixture::new();
    let preview = f.run(&["setup-root", "memory 世界"]);
    assert!(preview.status.success());
    assert!(preview.stderr.is_empty());
    assert!(!preview.stdout.contains(&0x1b));
    let text = String::from_utf8(preview.stdout).unwrap();
    assert!(text.contains("ROOT SETUP REVIEW"));
    assert!(text.contains("akasha.toml"));
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 0);
    let preview = f.run(&["setup-root", "memory 世界", "--json"]);
    assert!(preview.status.success());
    let json: serde_json::Value = serde_json::from_slice(&preview.stdout).unwrap();
    let id = json["plan_id"].as_str().unwrap();
    assert!(text.contains(id));
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 0);
    let wrong = f.run(&["setup-root", "memory 世界", "--plan-id", "wrong"]);
    assert_eq!(wrong.status.code(), Some(5));
    assert!(wrong.stdout.is_empty());
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 0);
    let result = f.run(&["setup-root", "memory 世界", "--plan-id", id, "--json"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(json["files"], 10);
    let again = f.run(&["setup-root", "memory 世界"]);
    assert_eq!(again.status.code(), Some(5));
    assert!(again.stdout.is_empty());
    let init = f.run(&["--root", "memory 世界", "init", "sample"]);
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    let validate = f.run(&["--root", "memory 世界", "validate"]);
    assert!(validate.status.success());
    let handoff = f.run(&["--root", "memory 世界", "onboard", "--json"]);
    assert!(
        handoff.status.success(),
        "{}",
        String::from_utf8_lossy(&handoff.stderr)
    );
    assert!(handoff.stderr.is_empty());
    let json: serde_json::Value = serde_json::from_slice(&handoff.stdout).unwrap();
    assert_eq!(json["project"], "sample");
    assert_eq!(json["command"], "akasha-onboarding-mcp");
    assert_eq!(json["args"][1], f.0.join("memory 世界").to_str().unwrap());
    let handoff = f.run(&["--root", "memory 世界", "onboard"]);
    assert!(!handoff.stdout.contains(&0x1b));
    let text = String::from_utf8(handoff.stdout).unwrap();
    assert!(text.contains("No agent connection"));
    assert!(text.contains("human approval"));
    assert!(text.contains("akasha_onboarding_preview"));
    assert!(!f.0.join("unconfigured").exists());
}
#[test]
fn setup_rejects_ambiguous_flags_and_onboarding_without_an_initialized_project() {
    let f = Fixture::new();
    for args in [
        vec!["--root", "elsewhere", "setup-root", "memory"],
        vec!["--project", "sample", "setup-root", "memory"],
    ] {
        let result = f.run(&args);
        assert_eq!(result.status.code(), Some(3));
        assert!(result.stdout.is_empty());
    }
    let result = f.run(&["onboard"]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(!result.stderr.is_empty());
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 0);
}
