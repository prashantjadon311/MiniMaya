use std::path::Path;
use std::process::Command;

#[test]
fn extended_40_package_assets_and_real_startup_smoke() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let syntax = Command::new("bash")
        .args([
            "-n",
            "scripts/smoke.sh",
            "scripts/measure.sh",
            "scripts/package.sh",
        ])
        .current_dir(root)
        .status()
        .expect("bash is required on the supported Ubuntu target");
    assert!(syntax.success());

    let desktop = std::fs::read_to_string(root.join("packaging/maya-mini.desktop")).unwrap();
    assert!(desktop.contains("Type=Application"));
    assert!(desktop.contains("Exec=maya-mini --open"));
    assert!(!desktop.contains("Terminal=true"));
    let service = std::fs::read_to_string(root.join("packaging/maya-mini.service")).unwrap();
    assert!(service.contains("--headless --port 0"));
    assert!(service.contains("NoNewPrivileges=true"));
    assert!(!service.contains("0.0.0.0"));

    let smoke = Command::new("bash")
        .arg("scripts/smoke.sh")
        .arg(env!("CARGO_BIN_EXE_maya-mini"))
        .current_dir(root)
        .output()
        .expect("execute smoke harness");
    assert!(
        smoke.status.success(),
        "smoke failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&smoke.stdout),
        String::from_utf8_lossy(&smoke.stderr)
    );
    assert!(String::from_utf8_lossy(&smoke.stdout).contains("smoke: PASS"));
}

#[test]
fn package_gate_fails_closed_without_release_evidence() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    if root.join("release/validation.json").exists()
        && root.join("target/release/maya-mini").exists()
    {
        return;
    }
    let output = Command::new("bash")
        .arg("scripts/package.sh")
        .current_dir(root)
        .output()
        .expect("execute package gate");
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("missing") || error.contains("not ready"));
}
