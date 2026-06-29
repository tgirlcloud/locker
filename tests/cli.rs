use std::process::Command;

fn locker(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_locker"))
        .args(args)
        .output()
        .expect("failed to run locker binary")
}

#[test]
fn duplicates_exit_code_and_stderr() {
    let out = locker(&["test/flake-lock.json"]);

    assert_eq!(out.status.code(), Some(1));

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("test/flake-lock.json: duplicate inputs found:"),
        "stderr missing header; got: {stderr}"
    );
    assert!(
        stderr.contains("github:nixos/nixpkgs"),
        "stderr missing expected uri; got: {stderr}"
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("No duplicate inputs found."),
        "stdout should not contain success message on failure; got: {stdout}"
    );
}

#[test]
fn sorted_uri_output() {
    let out = locker(&["test/flake-lock.json"]);
    let stderr = String::from_utf8_lossy(&out.stderr);

    let pos_git_hooks = stderr
        .find("github:cachix/git-hooks.nix")
        .expect("stderr should contain github:cachix/git-hooks.nix");
    let pos_nixpkgs = stderr
        .find("github:nixos/nixpkgs")
        .expect("stderr should contain github:nixos/nixpkgs");

    assert!(
        pos_git_hooks < pos_nixpkgs,
        "github:cachix/git-hooks.nix should appear before github:nixos/nixpkgs in sorted output"
    );
}

#[test]
fn clean_file_success() {
    let out = locker(&["test/no-duplicates.lock"]);

    assert_eq!(out.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("No duplicate inputs found."),
        "stdout missing success message; got: {stdout}"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.is_empty(),
        "stderr should be empty for clean file; got: {stderr}"
    );
}

#[test]
fn missing_file_exit_code_and_stderr() {
    let out = locker(&["does-not-exist.lock"]);

    assert_eq!(out.status.code(), Some(1));

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("does-not-exist.lock"),
        "stderr should mention the missing file; got: {stderr}"
    );
}

#[test]
fn batch_does_not_abort_on_first_failure() {
    let out = locker(&["does-not-exist.lock", "test/flake-lock.json"]);

    assert_eq!(out.status.code(), Some(1));

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("does-not-exist.lock"),
        "stderr should mention the missing file; got: {stderr}"
    );
    assert!(
        stderr.contains("duplicate inputs found:"),
        "stderr should contain duplicate report from second file; got: {stderr}"
    );
}
