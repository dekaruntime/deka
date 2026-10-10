#!/usr/bin/env python3
"""Headless component behaviour proofs; restore every mutation before returning."""
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / '.tmp/components-revert'
COMPONENTS = ROOT / 'crates/deka_ui/src/components.rs'
TREE = ROOT / 'crates/deka_native_ir/src/tree.rs'
VIEW = ROOT / 'crates/deka_ui/src/view.rs'
WEB = ROOT / 'crates/deka_ui/src/web.rs'
NATIVE = ROOT / 'crates/deka_native_ui/src/lib.rs'
originals = {path: path.read_text() for path in [COMPONENTS, TREE, VIEW, WEB, NATIVE]}
CASES = [
    ('button-callback', COMPONENTS, '                on_press();', '                // Reverted action delivery.',
     'button_pointer_keyboard_focus_signal_and_disabled_ingress'),
    ('input-two-way-binding', COMPONENTS, '.value(value)', '.value(String::new())',
     'input_pointer_two_way_signal_programmatic_updates_and_focus_order'),
    ('keyed-author-order', TREE, '        let kept: HashSet<_> = roots.iter()',
     '        roots.sort_by_key(|node| node.slot());\n        let kept: HashSet<_> = roots.iter()',
     'list_reorder_keeps_identity_focus_handlers_and_updates_labels'),
    ('tabs-keyboard-selection', COMPONENTS, '                selected.set(key_items[next].key.clone());',
     '                // Reverted keyboard selection.',
     'tabs_pointer_roving_keyboard_wrap_disabled_skipping_panels_and_signals'),
    ('tabs-focus-request', VIEW, '    fn take_focus_request(&self) -> Option<String> {',
     '    fn take_focus_request(&self) -> Option<String> { return None;\n',
     'tabs_pointer_roving_keyboard_wrap_disabled_skipping_panels_and_signals'),
    ('dark-theme-tokens', COMPONENTS, '                background: "#1e1610",', '                background: "#f3efe3",',
     'live_theme_signal_changes_all_components_without_replacing_controls'),
    ('press-motion-preset', COMPONENTS,
     'Self::Press => "frames-scale-[0:1,40:0.96,100:1] repeat-1 duration-120"',
     'Self::Press => "frames-scale-[0:1,40:1,100:1] repeat-1 duration-120"',
     'enter_exit_and_repeat_press_present_motion_and_reduced_motion_snaps'),
    ('browser-focus-delivery', WEB,
     '    pub fn take_requested_focus(&mut self) -> Option<String> {',
     '    pub fn take_requested_focus(&mut self) -> Option<String> { return None;\n',
     'input_pointer_two_way_signal_programmatic_updates_and_focus_order'),
    ('badge-live-label', COMPONENTS,
     '.child(View::live_text(move || label.get().unwrap_or_default())),',
     '.child(View::text("Ready")),',
     'badge_live_status_click_keyboard_signal_and_focus_skipping'),
    ('toast-dismissal', COMPONENTS,
     'on_press={Rc::new(move || open.set(false))}/>}), id.clone())',
     'on_press={Rc::new(move || open.set(true))}/>}), id.clone())',
     'toast_dismissal_pointer_keyboard_signal_focus_and_remount'),
    ('dialog-escape', COMPONENTS,
     'if matches!(event, Event::KeyDown(key) if key == "Escape") {\n                    open.set(false);',
     'if matches!(event, Event::KeyDown(key) if key == "Escape") {\n                    open.set(true);',
     'dialog_modal_entry_tab_wrap_inert_background_escape_restore_and_signals'),
    ('modal-tab-trap', NATIVE,
     'wrap.then(||', 'false.then(||',
     'dialog_modal_entry_tab_wrap_inert_background_escape_restore_and_signals'),
    ('modal-restore', NATIVE,
     'focused.map(str::to_owned)', 'None',
     'dialog_modal_entry_tab_wrap_inert_background_escape_restore_and_signals'),
    ('modal-background-inert', VIEW,
     'node.hidden |= !inside.contains(&node.id);', 'node.hidden |= false;',
     'dialog_modal_entry_tab_wrap_inert_background_escape_restore_and_signals'),
    ('menu-selection', COMPONENTS,
     '                                        key_action();', '                                        // Reverted menu action.',
     'menu_pointer_keyboard_roving_wrap_disabled_escape_restore_and_signal'),
    ('menu-roving-focus', COMPONENTS,
     '                                focus(next);', '                                let _ = next;',
     'menu_pointer_keyboard_roving_wrap_disabled_escape_restore_and_signal'),
]
# The theme test observes controls, so revert their surface/accent too.
def mutation(name, text):
    if name == 'dark-theme-tokens':
        for old, new in [("#2a2018","#fbf8f0"),("#f5efe2","#1a1611"),("#3ccb77","#0c8b43")]:
            text = text.replace(old, new)
        # Both themes must now use the light accent foreground.
        text = text.replace('accent_foreground: "#1a1611"', 'accent_foreground: "#ffffff"')
    return text

def main():
    OUT.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, CARGO_TARGET_DIR=str(ROOT/'.target'), TMPDIR=str(ROOT/'.tmp'))
    for name, path, old, new, test in CASES:
        try:
            text = originals[path]
            if text.count(old) != 1:
                raise RuntimeError(f'{name}: mutation anchor is not unique ({text.count(old)})')
            path.write_text(mutation(name, text.replace(old, new)))
            command = ['cargo','test','--locked','--release','-p','deka-ui','--no-default-features','--features','tour,web','--test','components',test,'--','--exact']
            with (OUT/f'{name}.log').open('w') as log:
                result = subprocess.run(command,cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=600)
            log = (OUT/f'{name}.log').read_text()
            if result.returncode == 0 or f'test {test} ... FAILED' not in log:
                raise RuntimeError(f'{name}: expected a behavioural failure, not success/build failure; see {OUT/name}.log')
            print(f'PASS revert proof: {name} fails {test}',flush=True)
        finally:
            path.write_text(originals[path])
    with (OUT/'restored.log').open('w') as log:
        subprocess.run(['cargo','test','--locked','--release','-p','deka-ui','--no-default-features','--features','tour,web','--test','components'],cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT,check=True,timeout=600)
    print('PASS restored: all component interactions',flush=True)

if __name__ == '__main__':
    main()
