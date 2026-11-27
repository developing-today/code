use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_repo-queue"))
        .args(args)
        .output()
        .expect("run repo-queue link command")
}

#[test]
fn link_group_and_nested_help_are_safe_without_arguments() {
    let bare = run(&["link"]);
    assert_eq!(bare.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bare.stdout).contains("link <add|remove|list>"));

    let group_help = run(&["link", "--help"]);
    assert_eq!(group_help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&group_help.stdout).contains("link <add|remove|list>"));

    let missing_task = run(&["link", "add"]);
    assert_eq!(missing_task.status.code(), Some(2));

    let add_help = run(&["link", "add", "--help"]);
    assert_eq!(add_help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&add_help.stdout).contains("link add <task-id>"));
}
