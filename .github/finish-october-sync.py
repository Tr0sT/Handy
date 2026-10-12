"""Preserve upstream short-recording behavior and virtual-to-local switching."""
from pathlib import Path

def replace_once(text, old, new):
    assert text.count(old) == 1, (old, text.count(old))
    return text.replace(old, new, 1)

p = Path('src-tauri/src/actions.rs')
s = p.read_text()
s = replace_once(s,
    '        let rm = Arc::clone(&app.state::<Arc<AudioRecordingManager>>());\n',
    '''        let rm = Arc::clone(&app.state::<Arc<AudioRecordingManager>>());
        // Invalidate synchronously, before scheduling the blocking stop, so a
        // slow microphone cannot play its ready chime after Stop was requested.
        rm.invalidate_recording_readiness();
''')
p.write_text(s)

p = Path('src-tauri/src/managers/transcription.rs')
s = p.read_text()
s = replace_once(s, 'fn panic_payload_message(', '''fn cached_model_matches(is_loaded: bool, current: Option<&str>, selected: &str) -> bool {
    is_loaded && current == Some(selected)
}

fn panic_payload_message(''')
s = replace_once(s,
    '''        if !reload_pending && self.is_model_loaded() {
            return;
        }
''',
    '''        let is_loaded = self.is_model_loaded();
        let current_model = self.current_model_id.lock().unwrap().clone();
        if !reload_pending
            && cached_model_matches(is_loaded, current_model.as_deref(), &settings.selected_model)
        {
            return;
        }
''')
s += '''
#[cfg(test)]
mod virtual_model_switch_tests {
    use super::cached_model_matches;
    use crate::managers::model::CODEX_LOCAL_MODEL_ID;

    #[test]
    fn switching_away_from_virtual_provider_does_not_skip_local_loading() {
        assert!(!cached_model_matches(true, Some(CODEX_LOCAL_MODEL_ID), "whisper"));
        assert!(!cached_model_matches(true, Some("whisper"), CODEX_LOCAL_MODEL_ID));
    }

    #[test]
    fn only_the_requested_loaded_model_can_be_reused() {
        for selected in [CODEX_LOCAL_MODEL_ID, "whisper"] {
            assert!(cached_model_matches(true, Some(selected), selected));
            assert!(!cached_model_matches(false, Some(selected), selected));
            assert!(!cached_model_matches(true, None, selected));
        }
    }
}
'''
p.write_text(s)
