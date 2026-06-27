// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use clap::{Args, Subcommand};
use ikaros_core::IkarosPaths;
use ikaros_runtime::{PersonaPatch, reset_persona, update_persona};
use ikaros_soul::{EmotionState, load_or_default};

#[derive(Debug, Subcommand)]
pub(crate) enum PersonaCommand {
    Show,
    Path,
    Set(Box<PersonaSet>),
    Reset,
}

#[derive(Debug, Args)]
pub(crate) struct PersonaSet {
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    role: Option<String>,
    #[arg(long)]
    description: Option<String>,
    #[arg(long = "tone")]
    tone_style: Option<String>,
    #[arg(long = "language")]
    tone_language: Option<String>,
    #[arg(long = "relationship-stance")]
    relationship_stance: Option<String>,
    #[arg(long = "trait")]
    traits: Vec<String>,
    #[arg(long = "boundary")]
    boundaries: Vec<String>,
    #[arg(long = "rule")]
    behavior_rules: Vec<String>,
}

pub(crate) fn persona_command(command: PersonaCommand, paths: &IkarosPaths) -> Result<()> {
    match command {
        PersonaCommand::Show => {
            let persona = load_or_default(&paths.persona_dir)?;
            println!("name: {}", persona.identity.name);
            println!("role: {}", persona.identity.role);
            println!("tone: {}", persona.tone.style);
            println!("emotion: {:?}", EmotionState::Neutral);
            println!();
            println!("{}", persona.context_summary());
        }
        PersonaCommand::Path => {
            println!("{}", paths.persona_dir.display());
        }
        PersonaCommand::Set(args) => {
            let args = *args;
            let report = update_persona(
                paths,
                PersonaPatch {
                    name: args.name,
                    role: args.role,
                    description: args.description,
                    tone_style: args.tone_style,
                    tone_language: args.tone_language,
                    relationship_stance: args.relationship_stance,
                    traits: args.traits,
                    boundaries: args.boundaries,
                    behavior_rules: args.behavior_rules,
                },
            )?;
            print_persona_write_report(&report);
        }
        PersonaCommand::Reset => {
            let report = reset_persona(paths)?;
            print_persona_write_report(&report);
        }
    }
    Ok(())
}

fn print_persona_write_report(report: &ikaros_runtime::PersonaWriteReport) {
    println!("ok: true");
    println!("name: {}", report.name);
    println!("role: {}", report.role);
    println!("changed_fields: {}", report.changed_fields.join(","));
    println!("path: {}", report.path.display());
    println!("audit: {}", report.audit_path.display());
}
