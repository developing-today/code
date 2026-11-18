use std::fs;
use std::process::Command;

fn run_case(name: &str, args: &[&str], config: Option<&str>) -> (i32, serde_json::Value) {
    let dir =
        std::env::temp_dir().join(format!("fleet-entry-ledger-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let ledger = dir.join("invocations.jsonl");
    let mut command = Command::new(env!("CARGO_BIN_EXE_fleet"));
    command.args(args).env("FLEET_INVOCATION_LEDGER", &ledger);
    if let Some(config) = config {
        let config_path = dir.join("config.toml");
        fs::write(&config_path, config).unwrap();
        command.env("FLEET_CONFIG", config_path);
    }
    let output = command.output().unwrap();
    let text = fs::read_to_string(&ledger).unwrap();
    let rows: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let _ = fs::remove_dir_all(dir);
    assert_eq!(rows.len(), 1, "one record for {name}");
    (
        output.status.code().unwrap_or(1),
        rows.into_iter().next().unwrap(),
    )
}

#[test]
fn entry_path_records_success_usage_error_and_config_failure_once() {
    let (code, row) = run_case("success", &["version"], None);
    assert_eq!(code, 0);
    assert_eq!(row["exit_code"], 0);

    let (code, row) = run_case("usage", &["--not-a-fleet-command"], None);
    assert_eq!(code, 2);
    assert_eq!(row["exit_code"], 2);

    let (code, row) = run_case("config-error", &["version"], Some("not = [valid toml"));
    assert_eq!(code, 2);
    assert_eq!(row["exit_code"], 2);
}
