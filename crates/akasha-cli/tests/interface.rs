use std::path::PathBuf;
use std::process::Command;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/resolution")
}

fn assert_uncolored(bytes: &[u8]) {
    assert!(
        !bytes.windows(2).any(|window| window == b"\x1b["),
        "output contains an ANSI control sequence"
    );
}

#[test]
fn help_and_invalid_usage_follow_the_stream_and_exit_contract() {
    let binary = env!("CARGO_BIN_EXE_akasha");
    let help = Command::new(binary)
        .arg("--help")
        .env("NO_COLOR", "1")
        .output()
        .expect("run akasha --help");

    assert!(help.status.success());
    assert!(help.stderr.is_empty());
    assert_uncolored(&help.stdout);
    let stdout = String::from_utf8(help.stdout).expect("help stdout is UTF-8");
    assert!(stdout.contains("Usage: akasha [OPTIONS] <COMMAND>"));
    assert!(stdout.contains("context"));
    assert!(stdout.contains("--json"));
    assert!(stdout.contains("--no-color"));

    let usage = Command::new(binary)
        .args(["resolve", "unexpected-argument"])
        .env("NO_COLOR", "1")
        .output()
        .expect("run invalid akasha usage");

    assert_eq!(usage.status.code(), Some(2));
    assert!(usage.stdout.is_empty());
    assert_uncolored(&usage.stderr);
    let stderr = String::from_utf8(usage.stderr).expect("usage stderr is UTF-8");
    assert!(stderr.contains("Usage: akasha resolve"));
}

#[test]
fn piped_plain_output_is_stable_under_no_color_controls() {
    let fixture = fixtures();
    let root = fixture.join("valid-root");
    let binary = env!("CARGO_BIN_EXE_akasha");

    let run = |no_color_flag: bool, no_color_environment: bool| {
        let mut command = Command::new(binary);
        command.args([
            "--root",
            root.to_str().expect("fixture path is UTF-8"),
            "--project",
            "example",
        ]);
        if no_color_flag {
            command.arg("--no-color");
        }
        command.arg("resolve").env_remove("AKASHA_ROOT");
        if no_color_environment {
            command.env("NO_COLOR", "1");
        } else {
            command.env_remove("NO_COLOR");
        }
        command.output().expect("run piped akasha resolve")
    };

    let baseline = run(false, false);
    let flag = run(true, false);
    let environment = run(false, true);

    for output in [&baseline, &flag, &environment] {
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_uncolored(&output.stdout);
    }
    assert_eq!(flag.stdout, baseline.stdout);
    assert_eq!(environment.stdout, baseline.stdout);
}
