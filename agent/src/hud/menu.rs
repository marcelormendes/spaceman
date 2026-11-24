//! Interactive HUD actions, per-app Float/Pin context menus, and the macOS secondary-click
//! bridge for non-activating overlay panels.

use rune::{PopupItem, PopupMenu};
use spaceman::WorkspaceId;

use crate::rules::Place;

pub const ID_TOGGLE_FLOAT: &str = "toggle-float";
pub const ID_TOGGLE_PIN: &str = "toggle-pin";
pub const ID_UNPIN_OTHER: &str = "unpin-other";

/// Side effects triggered by an interactive HUD overlay: switching workspaces and reading or
/// editing per-app `config.toml` rules.
pub trait HudSink: Send + Sync + 'static {
    fn switch_workspace(&self, workspace: WorkspaceId);
    fn app_place(&self, bundle_id: &str) -> Option<Place>;
    fn set_app_place(&self, bundle_id: &str, place: Option<Place>) -> Result<(), String>;
}

/// Inert sink used by default in unit and motion tests that do not attach a custom recorder.
#[cfg(test)]
pub struct NoopHudSink;

#[cfg(test)]
impl HudSink for NoopHudSink {
    fn switch_workspace(&self, _workspace: WorkspaceId) {}

    fn app_place(&self, _bundle_id: &str) -> Option<Place> {
        None
    }

    fn set_app_place(&self, _bundle_id: &str, _place: Option<Place>) -> Result<(), String> {
        Ok(())
    }
}

/// Live sink wired into the running `spaceman-agent`: routes workspace switches and Settings
/// opens through `UiBridge` and edits per-app rules through `rules::Doc`.
#[cfg(target_os = "macos")]
pub struct LiveHudSink {
    bridge: spaceman::app::UiBridge,
    window: rune::AppHandle<crate::Ui>,
}

#[cfg(target_os = "macos")]
impl LiveHudSink {
    #[must_use]
    pub fn new(bridge: spaceman::app::UiBridge, window: rune::AppHandle<crate::Ui>) -> Self {
        Self { bridge, window }
    }
}

#[cfg(target_os = "macos")]
impl HudSink for LiveHudSink {
    fn switch_workspace(&self, workspace: WorkspaceId) {
        self.bridge
            .send(spaceman::app::UiAction::SwitchWorkspace(workspace));
    }

    fn app_place(&self, bundle_id: &str) -> Option<Place> {
        crate::rules::Doc::load().app_place(bundle_id)
    }

    fn set_app_place(&self, bundle_id: &str, place: Option<Place>) -> Result<(), String> {
        let mut doc = crate::rules::Doc::load();
        doc.set_app_place(bundle_id, place);
        doc.save()?;
        self.bridge.send(spaceman::app::UiAction::Retile);
        self.window.update_if(|ui, _| {
            if ui.drag.is_some() {
                return false;
            }
            ui.doc = crate::rules::Doc::load();
            ui.look_up_apps();
            true
        });
        Ok(())
    }
}

/// Builds the compact native context menu for right-clicking an app icon inside a HUD slot.
#[must_use]
pub fn build_app_menu(app_name: &str, workspace: WorkspaceId, current: Option<Place>) -> PopupMenu {
    let float_item = if current == Some(Place::Float) {
        PopupItem::new(ID_TOGGLE_FLOAT, format!("Tile {app_name}")).symbol("rectangle.split.2x1")
    } else {
        PopupItem::new(ID_TOGGLE_FLOAT, format!("Float {app_name}"))
            .symbol("rectangle.on.rectangle")
    };
    let mut menu = PopupMenu::new().item(float_item);
    match current {
        Some(Place::Workspace(pinned)) if pinned == workspace => {
            menu = menu.item(
                PopupItem::new(
                    ID_TOGGLE_PIN,
                    format!("Unpin {app_name} from Workspace {workspace}"),
                )
                .symbol("pin.slash"),
            );
        }
        Some(Place::Workspace(other)) => {
            menu = menu
                .item(
                    PopupItem::new(
                        ID_TOGGLE_PIN,
                        format!("Pin {app_name} to Workspace {workspace}"),
                    )
                    .symbol("pin"),
                )
                .item(
                    PopupItem::new(
                        ID_UNPIN_OTHER,
                        format!("Unpin {app_name} from Workspace {other}"),
                    )
                    .symbol("pin.slash"),
                );
        }
        _ => {
            menu = menu.item(
                PopupItem::new(
                    ID_TOGGLE_PIN,
                    format!("Pin {app_name} to Workspace {workspace}"),
                )
                .symbol("pin"),
            );
        }
    }
    menu
}

/// Resolves a picked menu item ID into the new `Place` to write with `Doc::set_app_place`.
/// Returns `None` if the pick ID is unrecognized.
#[must_use]
pub fn resolve_app_menu_pick(
    picked: &str,
    workspace: WorkspaceId,
    current: Option<Place>,
) -> Option<Option<Place>> {
    match picked {
        ID_TOGGLE_FLOAT => {
            if current == Some(Place::Float) {
                Some(None)
            } else {
                Some(Some(Place::Float))
            }
        }
        ID_TOGGLE_PIN => {
            if current == Some(Place::Workspace(workspace)) {
                Some(None)
            } else {
                Some(Some(Place::Workspace(workspace)))
            }
        }
        ID_UNPIN_OTHER => Some(None),
        _ => None,
    }
}

/// Ensures right-clicks (and Control+clicks) on any HUD overlay panel reach `HudOverlay`.
///
/// Rune's macOS overlay host forwards left clicks through winit but drops `MouseButton::Right`.
/// Installing a process-local AppKit monitor scoped strictly to our HUD panels' window numbers
/// keeps normal windows (like Settings) untouched while opening the native `NSMenu` right at the
/// pointer without activating Spaceman.
#[cfg(target_os = "macos")]
pub(super) fn install_secondary_click_monitor(
    handles: Vec<rune::OverlayHandle<super::HudOverlay>>,
) {
    macos::install(handles);
}

#[cfg(not(target_os = "macos"))]
pub(super) fn install_secondary_click_monitor(
    _handles: Vec<rune::OverlayHandle<super::HudOverlay>>,
) {
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos {
    use std::{cell::RefCell, ptr::NonNull};

    use block2::RcBlock;
    use objc2::{MainThreadMarker, rc::Retained, runtime::AnyObject};
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags, NSEventType};
    use rune::OverlayHandle;

    use super::super::{HudOverlay, OVERLAY_SIZE};

    struct Monitor {
        token: Retained<AnyObject>,
    }

    impl Drop for Monitor {
        fn drop(&mut self) {
            // SAFETY: `token` was returned by `addLocalMonitorForEventsMatchingMask:handler:`
            // on the main thread and is removed once on drop.
            unsafe {
                NSEvent::removeMonitor(&self.token);
            }
        }
    }

    thread_local! {
        static MONITOR: RefCell<Option<Monitor>> = const { RefCell::new(None) };
    }

    pub(super) fn install(handles: Vec<OverlayHandle<HudOverlay>>) {
        if MainThreadMarker::new().is_none() {
            return;
        }
        MONITOR.with(|slot| {
            let mut slot = slot.borrow_mut();
            let block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
                // SAFETY: AppKit invokes the local monitor block on the main thread with a valid
                // non-null `NSEvent` reference for the duration of the callback.
                let ev = unsafe { event.as_ref() };
                let kind = ev.r#type();
                let is_secondary = kind == NSEventType::RightMouseDown
                    || (kind == NSEventType::LeftMouseDown
                        && ev.modifierFlags().contains(NSEventModifierFlags::Control));
                if !is_secondary {
                    return event.as_ptr();
                }
                let Ok(window_number) = u32::try_from(ev.windowNumber()) else {
                    return event.as_ptr();
                };
                let Some(handle) = handles
                    .iter()
                    .find(|handle| handle.window_number() == Some(window_number))
                else {
                    return event.as_ptr();
                };
                let loc = ev.locationInWindow();
                let height = MainThreadMarker::new()
                    .and_then(|mtm| ev.window(mtm))
                    .map_or(f64::from(OVERLAY_SIZE.1), |window| {
                        window.frame().size.height
                    });
                let x = loc.x as f32;
                let y = (height - loc.y) as f32;
                handle.update(move |overlay, tx| {
                    overlay.secondary_click_at(x, y, tx);
                });
                std::ptr::null_mut()
            });
            let mask = NSEventMask::RightMouseDown | NSEventMask::LeftMouseDown;
            let token = {
                // SAFETY: Main thread verified above; `block` is retained by AppKit until removed
                // in `Monitor::drop`.
                unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) }
            };
            if let Some(token) = token {
                *slot = Some(Monitor { token });
            }
        });
    }
}
