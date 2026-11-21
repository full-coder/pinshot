//! Integration tests that drive the real binary: argument parsing, the fake
//! backend, PNG output and the history database, all without a screen.

use std::path::Path;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pinshot")
}

fn run(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("run pinshot");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pinshot-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn snap_fake_writes_a_png_of_the_requested_size() {
    let dir = temp_dir("snap");
    let out = dir.join("shot.png");
    let (ok, stdout, stderr) = run(&[
        "--json",
        "--data-dir",
        dir.to_str().unwrap(),
        "snap",
        "--fake",
        "--area",
        "100",
        "100",
        "200",
        "150",
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.contains("\"width\":400"), "{stdout}");
    assert!(stdout.contains("\"height\":300"), "{stdout}");

    let bytes = std::fs::read(&out).expect("png written");
    let image = image::load_from_memory(&bytes).expect("decodable png");
    assert_eq!((image.width(), image.height()), (400, 300));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_capture_lands_in_the_history() {
    let dir = temp_dir("history");
    let (ok, stdout, stderr) = run(&[
        "--json",
        "--data-dir",
        dir.to_str().unwrap(),
        "snap",
        "--fake",
        "--area",
        "0",
        "0",
        "40",
        "30",
        "--no-save",
    ]);
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.contains("historyId"), "{stdout}");

    let (ok, stdout, stderr) = run(&[
        "--json",
        "--data-dir",
        dir.to_str().unwrap(),
        "history",
        "--limit",
        "5",
    ]);
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.contains("\"kind\":\"snip\""), "{stdout}");
    assert!(stdout.contains("\"width\":80"), "{stdout}");

    // The blob is on disk, under blobs/<xx>/<hash>.png.
    let blobs = dir.join("history").join("blobs");
    let count = walk(&blobs);
    assert_eq!(count, 1, "expected exactly one blob in {}", blobs.display());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bad_arguments_fail_loudly() {
    let dir = temp_dir("bad");
    let (ok, _, stderr) = run(&[
        "--data-dir",
        dir.to_str().unwrap(),
        "snap",
        "--fake",
        "--area",
        "0",
        "0",
        "0",
        "0",
    ]);
    assert!(!ok);
    assert!(stderr.contains("--area"), "{stderr}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn doctor_reports_the_platform() {
    let (ok, stdout, stderr) = run(&["--json", "doctor"]);
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.contains("\"os\""), "{stdout}");
    assert!(stdout.contains("\"permission\""), "{stdout}");
}

fn walk(directory: &Path) -> usize {
    let mut total = 0;
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            total += walk(&path);
        } else {
            total += 1;
        }
    }
    total
}
