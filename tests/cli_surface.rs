use std::process::Command;

#[test]
fn cli_exposes_only_global_options() {
    let binary = env!("CARGO_BIN_EXE_ikaros");
    let help = Command::new(binary)
        .arg("--help")
        .output()
        .expect("help process");

    assert!(help.status.success());
    let stdout = String::from_utf8_lossy(&help.stdout);
    assert!(stdout.contains("Usage: ikaros [OPTIONS]"));
    assert!(stdout.contains("--home"));
    assert!(stdout.contains("--workspace"));
    for removed in ["init", "chat", "sessions", "doctor"] {
        assert!(
            !stdout.contains(removed),
            "removed subcommand still appears in help: {removed}"
        );
    }

    let removed_subcommand = Command::new(binary)
        .arg("init")
        .output()
        .expect("removed subcommand process");
    assert!(!removed_subcommand.status.success());
}
