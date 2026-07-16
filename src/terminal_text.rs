pub(crate) fn sanitize_terminal(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            *character == '\n'
                || *character == '\t'
                || (!character.is_control() && *character != '\u{1b}')
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::sanitize_terminal;

    #[test]
    fn removes_terminal_control_sequences() {
        assert_eq!(sanitize_terminal("safe\u{1b}[31m red"), "safe[31m red");
        assert_eq!(
            sanitize_terminal("\u{1b}]8;;https://evil.test\u{7}click\u{1b}]8;;\u{7}"),
            "]8;;https://evil.testclick]8;;"
        );
    }
}
