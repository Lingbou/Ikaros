// SPDX-License-Identifier: GPL-3.0-only

pub(super) fn is_destructive_command(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    [
        "rm -rf",
        "rm -fr",
        "mkfs",
        "dd if=",
        ":(){",
        "chmod -r 777 /",
        "chown -r",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub(super) fn is_forbidden_publication_or_git_action(action: &str) -> bool {
    let lower = action.to_ascii_lowercase();
    [
        "git commit",
        "git push",
        "git tag",
        "gh release",
        "cargo publish",
        "npm publish",
        "docker push",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}
