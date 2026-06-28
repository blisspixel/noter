//! CLI smoke tests for the current planning skeleton.

use std::process::Command;

#[test]
fn binary_prints_planning_skeleton_status() {
    let output = Command::new(env!("CARGO_BIN_EXE_noter"))
        .output()
        .expect("noter binary should run");

    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "stderr should be empty, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");

    assert_eq!(
        stdout,
        format!(
            "Noter (planning skeleton)\nVersion: {}\nSee README.md for build instructions and current status.\nAll planning documents live in the repo root and are part of the product.\n",
            env!("CARGO_PKG_VERSION")
        )
    );
}
