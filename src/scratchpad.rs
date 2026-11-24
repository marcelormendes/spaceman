//! The Scratchpad Lab (`[labs] scratchpad`): an i3/sway-style stash of windows kept off-screen
//! until summoned. This module is the pure, platform-neutral model: which windows are in the
//! scratchpad, which one is shown where, the order a toggle cycles through them, and where a
//! summoned window lands. `app::scratchpad` performs the native writes.
//!
//! A hidden scratchpad window is parked exactly like a window on an inactive workspace (its
//! recovery frame in `parked-windows.json`), so pause, quit, restart and crashes bring it back on
//! screen. A shown one floats on the workspace it was summoned to. None of this is persisted:
//! after a restart a recovered scratchpad window is an ordinary window again.

use std::collections::VecDeque;

use crate::{PhysicalWindowId, Rect, WorkspaceId};

/// What a Scratchpad shortcut or `spaceman ctl scratchpad …` asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScratchpadCommand {
    /// Summon the next scratchpad window onto the focused display, or hide the one shown there.
    Toggle,
    /// Send the focused window to the scratchpad, or take a shown scratchpad window out of it.
    Mark,
}

impl ScratchpadCommand {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Toggle => "toggle",
            Self::Mark => "mark",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "toggle" => Some(Self::Toggle),
            "mark" => Some(Self::Mark),
            _ => None,
        }
    }
}

/// A summoned window covers this fraction of the display's width and height until the user
/// gives it a size of its own.
const DEFAULT_SHARE: (i32, i32) = (3, 4);
/// The smallest summoned frame, so a window remembered at a sliver stays usable.
const MIN_SIZE: (i32, i32) = (320, 240);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScratchpadEntry {
    pub window: PhysicalWindowId,
    /// The size it last floated at, kept for the next summon. `None` until it has floated.
    pub size: Option<(i32, i32)>,
    /// The workspace it floats on while shown; `None` while hidden (parked off-screen).
    pub shown_on: Option<WorkspaceId>,
    /// Centered frame a summon requested that the window server has not confirmed yet.
    pub pending_summon: Option<Rect>,
}

/// Scratchpad windows in toggle order: a toggle summons the first one not already shown on the
/// focused workspace, and hiding a window sends it to the back, so repeated toggles cycle.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScratchpadState {
    entries: VecDeque<ScratchpadEntry>,
}

impl ScratchpadState {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn contains(&self, window: &PhysicalWindowId) -> bool {
        self.entry(window).is_some()
    }

    #[must_use]
    pub fn entry(&self, window: &PhysicalWindowId) -> Option<&ScratchpadEntry> {
        self.entries.iter().find(|entry| &entry.window == window)
    }

    pub fn entries(&self) -> impl Iterator<Item = &ScratchpadEntry> {
        self.entries.iter()
    }

    /// Adds a hidden window at the back of the order. Returns false if it was already there.
    pub fn insert(&mut self, window: PhysicalWindowId, size: Option<(i32, i32)>) -> bool {
        if self.contains(&window) {
            return false;
        }
        self.entries.push_back(ScratchpadEntry {
            window,
            size,
            shown_on: None,
            pending_summon: None,
        });
        true
    }

    pub fn remove(&mut self, window: &PhysicalWindowId) -> Option<ScratchpadEntry> {
        let index = self
            .entries
            .iter()
            .position(|entry| &entry.window == window)?;
        self.entries.remove(index)
    }

    /// Empties the scratchpad, returning what it held.
    pub fn clear(&mut self) -> Vec<ScratchpadEntry> {
        self.entries.drain(..).collect()
    }

    /// The windows shown on `workspace`, in order.
    #[must_use]
    pub fn shown_on(&self, workspace: WorkspaceId) -> Vec<PhysicalWindowId> {
        self.entries
            .iter()
            .filter(|entry| entry.shown_on == Some(workspace))
            .map(|entry| entry.window.clone())
            .collect()
    }

    /// The hidden windows, in order.
    #[must_use]
    pub fn hidden(&self) -> Vec<PhysicalWindowId> {
        self.entries
            .iter()
            .filter(|entry| entry.shown_on.is_none())
            .map(|entry| entry.window.clone())
            .collect()
    }

    /// The window a toggle on `workspace` summons: the first one not already shown there.
    #[must_use]
    pub fn next_for(&self, workspace: WorkspaceId) -> Option<PhysicalWindowId> {
        self.entries
            .iter()
            .find(|entry| entry.shown_on != Some(workspace))
            .map(|entry| entry.window.clone())
    }

    pub fn mark_shown(&mut self, window: &PhysicalWindowId, workspace: WorkspaceId) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| &entry.window == window)
        {
            entry.shown_on = Some(workspace);
        }
    }

    #[must_use]
    pub fn pending_summon(&self, window: &PhysicalWindowId) -> Option<Rect> {
        self.entry(window).and_then(|entry| entry.pending_summon)
    }

    pub fn set_pending_summon(&mut self, window: &PhysicalWindowId, frame: Option<Rect>) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| &entry.window == window)
        {
            entry.pending_summon = frame;
        }
    }

    /// Hides `window` and sends it to the back of the order, so the next toggle summons the
    /// window after it. `size` replaces the remembered size when known.
    pub fn mark_hidden(&mut self, window: &PhysicalWindowId, size: Option<(i32, i32)>) {
        let Some(mut entry) = self.remove(window) else {
            return;
        };
        entry.shown_on = None;
        entry.pending_summon = None;
        entry.size = size.or(entry.size);
        self.entries.push_back(entry);
    }
}

/// Where a summoned window lands on `area` (a display's frame): centred, at its remembered
/// size or three quarters of the display, and never larger than the display.
#[must_use]
pub fn summon_frame(area: Rect, size: Option<(i32, i32)>) -> Rect {
    let (width, height) = size.unwrap_or((
        area.width * DEFAULT_SHARE.0 / DEFAULT_SHARE.1,
        area.height * DEFAULT_SHARE.0 / DEFAULT_SHARE.1,
    ));
    let width = width.max(MIN_SIZE.0).min(area.width);
    let height = height.max(MIN_SIZE.1).min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppInstanceId;

    fn window(id: u32) -> PhysicalWindowId {
        PhysicalWindowId {
            app: AppInstanceId {
                pid: 1,
                launched_at: 1,
            },
            window_id: id,
        }
    }

    fn area() -> Rect {
        Rect {
            x: 1000,
            y: 25,
            width: 1600,
            height: 1000,
        }
    }

    #[test]
    fn summoned_windows_centre_at_three_quarters_by_default() {
        assert_eq!(
            summon_frame(area(), None),
            Rect {
                x: 1200,
                y: 150,
                width: 1200,
                height: 750,
            }
        );
    }

    #[test]
    fn summoned_windows_keep_their_size_clamped_to_the_display() {
        assert_eq!(
            summon_frame(area(), Some((800, 600))),
            Rect {
                x: 1400,
                y: 225,
                width: 800,
                height: 600,
            }
        );
        assert_eq!(summon_frame(area(), Some((5000, 5000))), area());
        let tiny = summon_frame(area(), Some((10, 10)));
        assert_eq!((tiny.width, tiny.height), MIN_SIZE);
        let small_display = Rect {
            x: 0,
            y: 0,
            width: 200,
            height: 100,
        };
        assert_eq!(summon_frame(small_display, Some((10, 10))), small_display);
    }

    #[test]
    fn toggles_cycle_through_windows_in_order() {
        let mut scratchpad = ScratchpadState::default();
        let one = WorkspaceId::ONE;
        assert_eq!(scratchpad.next_for(one), None);
        assert!(scratchpad.insert(window(1), None));
        assert!(scratchpad.insert(window(2), Some((500, 400))));
        assert!(!scratchpad.insert(window(1), None));
        assert_eq!(scratchpad.len(), 2);

        assert_eq!(scratchpad.next_for(one), Some(window(1)));
        scratchpad.mark_shown(&window(1), one);
        assert_eq!(scratchpad.shown_on(one), vec![window(1)]);
        assert_eq!(scratchpad.hidden(), vec![window(2)]);
        // A window shown on another workspace can be summoned here.
        assert_eq!(scratchpad.next_for(WorkspaceId::TWO), Some(window(1)));

        scratchpad.mark_hidden(&window(1), Some((640, 480)));
        assert_eq!(scratchpad.next_for(one), Some(window(2)));
        assert_eq!(scratchpad.entry(&window(1)).unwrap().size, Some((640, 480)));
        scratchpad.mark_shown(&window(2), one);
        scratchpad.mark_hidden(&window(2), None);
        assert_eq!(scratchpad.entry(&window(2)).unwrap().size, Some((500, 400)));
        assert_eq!(scratchpad.next_for(one), Some(window(1)));

        assert_eq!(scratchpad.remove(&window(1)).unwrap().window, window(1));
        assert!(!scratchpad.contains(&window(1)));
        assert_eq!(scratchpad.clear().len(), 1);
        assert!(scratchpad.is_empty());
    }

    #[test]
    fn commands_round_trip_their_ctl_names() {
        for command in [ScratchpadCommand::Toggle, ScratchpadCommand::Mark] {
            assert_eq!(ScratchpadCommand::parse(command.as_str()), Some(command));
        }
        assert_eq!(ScratchpadCommand::parse("show"), None);
    }
}
