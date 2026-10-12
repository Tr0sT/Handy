"""Rebase the preserved fork without editing origin/main."""
from pathlib import Path
import json
import re
import subprocess

OLD = '9d4253da60b479c8bdc01b43d255b415dbd0bd09'
BASE = '330b2ddbed3daab8f1fb09385cc1aaf73708e9d7'
UPSTREAM = '7b2a28a30aa24174292ddd7f4e92e12e6e3bd501'

def git(*args, check=True):
    return subprocess.run(['git', *args], check=check, text=True, capture_output=True)

def original(name, rev=UPSTREAM):
    return git('show', f'{rev}:{name}').stdout

def replace_once(text, old, new):
    assert text.count(old) == 1, (old[:120], text.count(old))
    return text.replace(old, new, 1)

git('checkout', '--detach', OLD)
result = git('rebase', '--onto', UPSTREAM, BASE, check=False)
assert result.returncode != 0, result.stdout
conflicts = set(git('diff', '--name-only', '--diff-filter=U').stdout.splitlines())
assert conflicts == {
    'src-tauri/Cargo.lock', 'src-tauri/Cargo.toml',
    'src-tauri/src/actions.rs', 'src-tauri/src/audio_toolkit/audio/recorder.rs',
    'src-tauri/src/managers/transcription.rs', 'src/bindings.ts',
}, conflicts
pattern = re.compile(r'^<<<<<<< .*?\n(.*?)^=======\n(.*?)^>>>>>>> .*?\n', re.M | re.S)

def resolve(name, handler, count):
    p = Path(name)
    text, actual = pattern.subn(lambda m: handler(m[1], m[2]), p.read_text())
    assert actual == count, (name, actual)
    p.write_text(text)

# Keep current recorder and playback-only rodio, plus fork HTTP dependencies.
resolve('src-tauri/Cargo.toml', lambda up, fork: (
    up[:up.index('reqwest =')] + fork[fork.index('reqwest ='):]
    if 'reqwest =' in up else up + fork
), 2)
resolve('src-tauri/Cargo.lock', lambda up, fork: up + fork.replace(' "vad-rs",\n', ''), 1)
git('rm', 'src-tauri/src/audio_toolkit/audio/recorder.rs')

# Combine immediate unloading with cancellation/paste completion lifetimes.
number = [0]
def actions_conflict(up, fork):
    i = number[0]
    number[0] += 1
    if i == 0:
        return up + '''        shortcut::unregister_cancel_shortcut(&self.0);
        utils::hide_recording_overlay(&self.0);
        set_tray_state(&self.0, TrayIconState::Idle);
'''
    return fork.replace('FinishGuard(ah.clone())', 'FinishGuard(ah.clone(), Arc::clone(&tm))')
resolve('src-tauri/src/actions.rs', actions_conflict, 3)
p = Path('src-tauri/src/actions.rs')
s = p.read_text().replace('use crate::managers::model::ModelManager;', 'use crate::managers::model::{EngineType, ModelManager};')
s = replace_once(s, '''        if !tm.is_model_loaded() {
            let selected_model = get_settings(app).selected_model;
''', '''        let selected_model = get_settings(app).selected_model;
        let selected_engine = app
            .state::<Arc<ModelManager>>()
            .get_model_info(&selected_model)
            .map(|model| model.engine_type);
        if needs_local_recording_model(tm.is_model_loaded(), selected_engine.as_ref()) {
''')
s = replace_once(s, 'async fn post_process_transcription(', '''/// Virtual providers do not have a downloadable file, even while their
/// background selection is completing. Local models still use the upstream
/// missing-file guard before the microphone is opened.
fn needs_local_recording_model(is_loaded: bool, engine: Option<&EngineType>) -> bool {
    !is_loaded && !matches!(engine, Some(EngineType::CloudCodex))
}

async fn post_process_transcription(''')
s = replace_once(s, '    fn blank_transcription_is_detected() {', '''    fn virtual_provider_never_requires_a_local_recording_model() {
        for loaded in [false, true] {
            assert!(!super::needs_local_recording_model(
                loaded, Some(&super::EngineType::CloudCodex)
            ));
        }
    }

    #[test]
    fn unloaded_local_or_unknown_model_requires_the_missing_file_guard() {
        for engine in [None, Some(&super::EngineType::TranscribeCpp), Some(&super::EngineType::Parakeet)] {
            assert!(super::needs_local_recording_model(false, engine));
            assert!(!super::needs_local_recording_model(true, engine));
        }
    }

    #[test]
    fn blank_transcription_is_detected() {''')
p.write_text(s)

# Preserve the complete supervisor-aware loader and all terminal error events.
p = Path('src-tauri/src/managers/transcription.rs')
s = p.read_text()
start = s.index('    pub fn load_model_with_device(')
end = s.index('    /// Kicks off the model loading', start)
u = original(str(p))
loader = u[u.index('    pub fn load_model_with_device('):u.index('    /// Kicks off the model loading')]
virtual = '''        // A virtual provider has neither an ONNX engine nor a native worker.
        // Queue the unload before publishing its id so it cannot unload a
        // later selection when the previous worker eventually exits.
        if model_info.engine_type == EngineType::CloudCodex {
            self.cancel_stream();
            self.request_unload();
            *self.current_model_id.lock().unwrap() = Some(model_id.to_string());
            self.touch_activity();
            let _ = self.app_handle.emit(
                "model-state-changed",
                ModelStateEvent {
                    event_type: "loading_completed".to_string(),
                    model_id: Some(model_id.to_string()),
                    model_name: Some(model_info.name.clone()),
                    error: None,
                },
            );
            return Ok(());
        }

'''
loader = replace_once(loader, '        if !model_info.is_downloaded {', virtual + '        if !model_info.is_downloaded {')
loader = replace_once(loader, '        let loaded_onnx = match model_info.engine_type {', '''        let loaded_onnx = match model_info.engine_type {
            EngineType::CloudCodex => unreachable!("virtual provider handled before local loading"),''')
s = s[:start] + loader + s[end:]
# The stream text emitter is now an upstream free function.
s, n = pattern.subn(lambda m: m[2][m[2].index('    /// Provider-aware entry point'):], s)
assert n == 1, n
# Use the same asynchronous loading slot for virtual selection.
start = s.index('        if self\n', s.index('    pub fn initiate_model_load('))
end = s.index('        let mut is_loading =', start)
s = s[:start] + s[end:]
s = replace_once(s, '''        self.engine.loaded().is_some() || self.lock_onnx().is_some()
''', '''        self.engine.loaded().is_some()
            || self.lock_onnx().is_some()
            || self.current_model_id.lock().unwrap().as_deref().is_some_and(|id| {
                self.model_manager.get_model_info(id)
                    .is_some_and(|model| model.engine_type == EngineType::CloudCodex)
            })
''')
assert 'lock_engine' not in s
p.write_text(s)

# Live streaming feeds StreamRouter through CaptureSink. Remove the unused
# adapter for the deleted recorder instead of resurrecting that implementation.
p = Path('src-tauri/src/managers/audio.rs')
s = p.read_text()
start = s.index('    /// Set a data callback on the recorder')
end = s.index('    /// Cancel any ongoing recording', start)
p.write_text(s[:start] + s[end:])
resolve('src/bindings.ts', lambda up, fork: up + fork, 1)

# Upstream added two locales; keep the fork's translated control there too.
for locale, label in [('ca', 'Alternativa del porta-retalls per a Unicode'), ('id', 'Metode papan klip cadangan untuk Unicode')]:
    p = Path(f'src/i18n/locales/{locale}/translation.json')
    data = json.loads(p.read_text())
    assert 'unicodeFallback' not in data['settings']['advanced']['pasteMethod']
    data['settings']['advanced']['pasteMethod']['unicodeFallback'] = label
    p.write_text(json.dumps(data, indent=2, ensure_ascii=False) + '\n')

# Install exactly the workflow tree authorized by the connector on the seed
# branch. The Actions token does not author workflow changes.
seed = 'd12cb71a3395d50166f74870f4f202a30f00dabe'
p = Path('.github/workflows/fork-regressions.yml')
p.write_text(original(str(p), seed))
assert Path('.github/workflows/nix-check.yml').read_text() == original('.github/workflows/nix-check.yml', seed)
git('add', '-u')
assert not git('diff', '--name-only', '--diff-filter=U').stdout
print('Resolved six conflicts and adapted virtual providers to the new recorder/supervisor.')
