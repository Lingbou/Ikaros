// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Result, anyhow};

pub(in crate::chat::interactive) fn workbench_code_alias_command(
    command: &str,
    input: &str,
) -> Result<String> {
    let subcommand = command
        .strip_prefix('/')
        .ok_or_else(|| anyhow!("coding alias must start with '/'"))?;
    let rest = input
        .strip_prefix(command)
        .map(str::trim)
        .unwrap_or_default();
    if rest.is_empty() {
        Ok(subcommand.to_owned())
    } else {
        Ok(format!("{subcommand} {rest}"))
    }
}

#[cfg(test)]
mod tests {
    use super::workbench_code_alias_command;

    #[test]
    fn workbench_code_aliases_delegate_to_code_subcommands() {
        assert_eq!(
            workbench_code_alias_command("/review", "/review --diff \"diff text\"")
                .expect("review alias"),
            "review --diff \"diff text\""
        );
        assert_eq!(
            workbench_code_alias_command(
                "/rollback",
                "/rollback coding-session --turn-id coding-turn"
            )
            .expect("rollback alias"),
            "rollback coding-session --turn-id coding-turn"
        );
    }
}
