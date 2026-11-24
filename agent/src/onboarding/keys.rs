//! Hears the practice shortcuts while the Done step shows.
//!
//! Carbon hotkeys take Option shortcuts before any window sees them once Spaceman has
//! registered them, so the window's own key events can't drive the practice desktop. A
//! listen-only key tap hears them either way, and only while Spaceman is the active app and
//! the Done step is up. It never stops a press: when tiling is already on, the real shortcut
//! still runs. Spaceman never manages its own windows, so the setup window is never parked, and
//! it comes back to the front if that shortcut brought another app forward.

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use spaceman::macos::KeyMonitor;

use super::sandbox::Key;
use crate::Ui;

/// Carbon's `optionKey` modifier bit, as the key tap reports modifiers.
const OPTION: u32 = 1 << 11;

pub struct Listener {
    _monitor: KeyMonitor,
}

/// Starts hearing Option shortcuts for `window`, or `None` without Accessibility.
pub fn listen(window: rune::AppHandle<Ui>) -> Option<Listener> {
    KeyMonitor::install(move |key_code, modifiers| {
        if modifiers != OPTION || !active() {
            return;
        }
        let Some(key) = Key::from_key_code(key_code) else {
            return;
        };
        window.update_if(move |ui, tx| {
            // A press from the tap isn't input to this window, so ask for its motion.
            tx.animated();
            super::heard(ui, key, tx)
        });
    })
    .ok()
    .map(|monitor| Listener { _monitor: monitor })
}

fn active() -> bool {
    MainThreadMarker::new().is_some_and(|mtm| NSApplication::sharedApplication(mtm).isActive())
}
