// SPDX-License-Identifier: GPL-3.0-only

use crate::{SkillEnvironment, VoiceAsrSkill, VoiceTtsSkill};
use ikaros_toolkit::{SkillRegistry, Toolset};

pub(super) fn register(registry: &mut SkillRegistry, env: &SkillEnvironment) {
    registry.register_with_toolset(
        VoiceTtsSkill::new(env.voice_tts.clone(), env.voice_tts_provider.clone()),
        Toolset::Voice,
    );
    registry.register_with_toolset(
        VoiceAsrSkill::new(env.voice_asr.clone(), env.voice_asr_provider.clone()),
        Toolset::Voice,
    );
}
