// SPDX-License-Identifier: GPL-3.0-only

use super::CodeCommand;
use anyhow::Result;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "code")]
struct InteractiveCodeCli {
    #[command(subcommand)]
    command: CodeCommand,
}

pub(crate) fn parse_interactive_code_command(input: &str) -> Result<CodeCommand> {
    let args = split_interactive_code_line(input)?;
    let cli = InteractiveCodeCli::try_parse_from(
        std::iter::once("code").chain(args.iter().map(String::as_str)),
    )?;
    Ok(cli.command)
}

fn split_interactive_code_line(input: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for ch in input.chars() {
        if escaped {
            current.push(decode_interactive_code_escape(ch));
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            ch if ch.is_whitespace() => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if escaped {
        anyhow::bail!("unterminated escape in /code command");
    }
    if quote.is_some() {
        anyhow::bail!("unterminated quote in /code command");
    }
    if !current.is_empty() {
        args.push(current);
    }
    Ok(args)
}

fn decode_interactive_code_escape(ch: char) -> char {
    match ch {
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        other => other,
    }
}
