// SPDX-License-Identifier: GPL-3.0-only
//! Persona, emotion, and relationship primitives for Ikaros.

mod loader;
mod model;

pub use loader::{PersonaLoader, load_or_default};
pub use model::{
    BehaviorRule, EmotionState, Identity, PersonaProfile, PersonalityTrait, RelationshipMemory,
    RelationshipModel, RuntimeSignal, ToneConfig,
};
