"""Reproduce the reviewed Handy rebase without changing origin/main."""
from pathlib import Path
import hashlib
import os
import re
import subprocess

OLD = "ec96125738d24b0eb3057bbc5797549999016792"
BASE = "37a26fd6ab905259d66affea57fff448288ca1aa"
UPSTREAM = "bc7facea3a777869182203cfcf5c90f7a98efd99"
os.environ["GIT_EDITOR"] = "true"

def git(*args, check=True):
    return subprocess.run(["git", *args], check=check, text=True)

def conflicts():
    return set(subprocess.check_output(["git", "diff", "--name-only", "--diff-filter=U"], text=True).splitlines())

git("checkout", "--detach", OLD)
result = git("rebase", "--onto", UPSTREAM, BASE, check=False)
assert result.returncode != 0
paths = {"src-tauri/src/actions.rs", "src-tauri/src/clipboard.rs", "src/bindings.ts", "src/lib/constants/languages.ts"}
assert conflicts() == paths, conflicts()
pattern = re.compile(r"^<<<<<<< HEAD\n(.*?)^=======\n(.*?)^>>>>>>> [^\n]*\n", re.M | re.S)
for name in sorted(paths):
    path = Path(name)
    def resolve(match):
        upstream, fork = match.groups()
        if name.endswith("actions.rs"):
            return upstream.replace("_shortcut_str", "shortcut_str") + "\n" + fork.split("\n", 1)[1]
        if name.endswith("clipboard.rs"):
            return upstream + "    }\n\n" + fork
        if name.endswith("bindings.ts"):
            return fork.replace("id: number; name: string; total_vram_mb", "id: string; name: string; total_vram_mb")
        return upstream + fork
    content, count = pattern.subn(resolve, path.read_text())
    assert count == 1, (name, count)
    path.write_text(content)
git("add", *sorted(paths))
result = git("rebase", "--continue", check=False)
assert result.returncode != 0
assert conflicts() == {".nix/bun-lock-hash"}, conflicts()
Path(".nix/bun-lock-hash").write_text(hashlib.sha256(Path("bun.lock").read_bytes()).hexdigest() + "\n")
git("add", ".nix/bun-lock-hash")
git("rebase", "--continue")

# Use the upstream microphone-readiness path for the virtual Codex model.
path = Path("src-tauri/src/actions.rs")
content = path.read_text()
old = '''        let settings = get_settings(app);
        if selected_model_engine(app, &settings) == Some(EngineType::CloudCodex) {
            let action = CloudCodexTranscribeAction {
                post_process: self.post_process,
            };
            action.start(app, binding_id, shortcut_str);
            return;
        }

'''
assert content.count(old) == 1
content = content.replace(old, "", 1)
start = content.index("    fn start(", content.index("impl ShortcutAction for CloudCodexTranscribeAction"))
end = content.index("    fn stop(", start)
content = content[:start] + '''    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str) {
        // Share the upstream readiness, VAD, feedback and error-handling path.
        // TranscribeAction::start also supports the virtual CloudCodex model;
        // only its stop path dispatches to the cloud-specific HTTP pipeline.
        TranscribeAction {
            post_process: self.post_process,
        }
        .start(app, binding_id, shortcut_str);
    }

''' + content[end:]
marker = "impl ShortcutAction for TranscribeAction {\n    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str)"
assert marker in content
content = content.replace(marker, marker.replace("shortcut_str:", "_shortcut_str:"), 1)
content = content.replace("change_tray_icon(", "set_tray_state(")
path.write_text(content)
git("diff", "--check")
