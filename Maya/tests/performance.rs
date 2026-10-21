use maya_mini::app::App;
use maya_mini::config::RuntimeConfig;
use maya_mini::types::{ActionClass, Actor, ActorKind, Operation};
use std::path::Path;
use std::process::Command;

#[test]
#[ignore = "run on release hardware for recorded resource evidence"]
fn extended_34_external_sampler_records_real_proc_values() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("measurements.csv");
    let status = Command::new("bash")
        .arg("scripts/measure.sh")
        .args(["--pid", &std::process::id().to_string()])
        .args(["--duration-seconds", "1"])
        .args(["--interval-seconds", "0.2"])
        .arg("--output")
        .arg(&output)
        .args(["--fixture", "sampler-test"])
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success());
    let csv = std::fs::read_to_string(output).unwrap();
    let rows: Vec<_> = csv.lines().collect();
    assert!(rows.len() >= 3, "expected at least two real samples");
    assert!(rows.iter().skip(1).all(|row| row.contains("sampler-test")));
    assert!(rows.iter().skip(1).all(|row| row.ends_with(",ok")));
}

#[tokio::test]
#[ignore = "run with release binary and release-hardware stress evidence"]
async fn extended_35_twenty_five_persisted_jobs_remain_within_core_memory_gate() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::build(RuntimeConfig {
        data_dir: dir.path().join("data"),
        cache_dir: dir.path().join("cache"),
        runtime_dir: dir.path().join("runtime"),
        port: 0,
        open_browser: false,
        headless: true,
        print_launch_link: false,
        test_mode: true,
    })
    .await
    .unwrap();
    for _ in 0..25 {
        let receipt = app
            .state
            .workflow
            .create_operation(
                Operation::ClipboardRead {},
                Actor {
                    kind: ActorKind::Owner,
                    id: Some(uuid::Uuid::new_v4()),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            receipt.approval.unwrap().scope.action_classes,
            vec![ActionClass::ClipboardRead]
        );
    }
    let sample = app.state.telemetry.sample(false).await.unwrap();
    let rss = sample.core.rss_bytes.expect("Linux /proc core RSS");
    assert!(rss < 500_000_000, "core RSS {rss} exceeds release gate");
}
