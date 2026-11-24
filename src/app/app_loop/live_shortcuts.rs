//! `[shortcuts]` and enabled named workspaces applied while Spaceman runs: a changed table or
//! workspace list re-registers the global hotkeys at once, and Settings › Shortcuts holds them off
//! while it records a chord.

use crate::shortcuts::Shortcuts;

use super::*;

/// The longest Settings may hold the hotkeys off for one recording. A recorder left listening
/// (a window that never reported losing focus) must not cost the user every shortcut.
const RECORDING_LIMIT: Duration = Duration::from_secs(60);

/// The inputs that build a [`HotkeyPlan`]: enabled workspaces and `[shortcuts]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HotkeyConfig {
    pub(super) workspaces: Vec<WorkspaceId>,
    pub(super) shortcuts: Shortcuts,
}

impl HotkeyConfig {
    fn differs_from(&self, other: &Self) -> bool {
        self.workspaces != other.workspaces || self.shortcuts.differs_from(&other.shortcuts)
    }
}

/// Whether Settings › Shortcuts is holding the hotkeys off for a recording, and any
/// shortcut or workspace change saved while the hold is active.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum HotkeyHold {
    #[default]
    None,
    /// Hotkeys were unregistered for recording; `MainThreadAgent` keeps the last working
    /// `(hotkey_workspaces, shortcuts)` so a saved replacement that fails to plan or register
    /// rolls back to it.
    Held { pending: Option<HotkeyConfig> },
}

impl HotkeyHold {
    const fn is_held(&self) -> bool {
        matches!(self, Self::Held { .. })
    }
}

/// What a shortcut or workspace change asks of the registered hotkeys.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum ShortcutChange {
    /// The same hotkeys, perhaps spelled differently: nothing to do.
    Same,
    /// Keep the config for the next registration; none is installed or held now.
    Store,
    /// Hotkeys are suspended while Settings records; try this config when recording ends.
    Defer,
    /// Register the new config in place of the installed hotkeys.
    Reinstall,
}

pub(super) fn shortcut_change(
    active: &HotkeyConfig,
    next: &HotkeyConfig,
    installed: bool,
    held: bool,
) -> ShortcutChange {
    if held {
        ShortcutChange::Defer
    } else if !active.differs_from(next) {
        ShortcutChange::Same
    } else if installed {
        ShortcutChange::Reinstall
    } else {
        ShortcutChange::Store
    }
}

/// Computes the sorted enabled workspaces that should hold hotkeys for `scope`:
/// every configured workspace in scope, plus any currently occupied named
/// workspace in scope so removing its rule/entry does not drop its shortcut
/// while it still holds windows.
pub(super) fn rebuild_hotkey_workspaces(
    configured: &[(WorkspaceId, DisplaySelector)],
    occupied_named: &[WorkspaceId],
    scope: ManagementScope,
) -> Vec<WorkspaceId> {
    let mut next: Vec<WorkspaceId> = configured
        .iter()
        .filter(|(_, selector)| scope.manages_selector(selector))
        .map(|(workspace, _)| *workspace)
        .collect();
    for workspace in occupied_named {
        let selector = configured
            .iter()
            .find(|(id, _)| id == workspace)
            .map_or_else(
                || Workspaces::default_assignment(*workspace),
                |(_, selector)| selector.clone(),
            );
        if workspace.letter().is_some()
            && scope.manages_selector(&selector)
            && !next.contains(workspace)
        {
            next.push(*workspace);
        }
    }
    next.sort();
    next.dedup();
    next
}

/// Result of attempting to activate a replacement hotkey config over the active one.
#[derive(Debug, Eq, PartialEq)]
enum InstallOutcome {
    /// The replacement config is now active and installed.
    Applied,
    /// The replacement could not be applied; the previous working config is active and `String`
    /// explains why.
    Reverted(String),
    /// Neither the replacement nor the previous config could be installed.
    Unavailable {
        error: String,
        not_applied: Option<String>,
    },
}

/// Applies `next` over `active`, rolling back to `active` if planning or registering `next`
/// fails.
///
/// `hotkeys` is `Some` during a live reload, or `None` when resuming after a recording hold (in
/// which case a planning failure must also re-register `active`).
fn apply_with_rollback<H, P>(
    active: &mut HotkeyConfig,
    hotkeys: &mut Option<H>,
    next: HotkeyConfig,
    mut plan: impl FnMut(&HotkeyConfig) -> Result<P, String>,
    mut install: impl FnMut(P) -> Result<H, String>,
    mut release_before_install: impl FnMut(&mut Option<H>),
) -> InstallOutcome {
    let planned = match plan(&next) {
        Ok(planned) => planned,
        Err(error) => {
            if hotkeys.is_some() {
                return InstallOutcome::Reverted(error);
            }
            return restore_active(active, hotkeys, error, plan, install);
        }
    };
    // Carbon cannot register the new chords until the old ones are released.
    release_before_install(hotkeys);
    match install(planned) {
        Ok(installed) => {
            *active = next;
            *hotkeys = Some(installed);
            InstallOutcome::Applied
        }
        Err(error) => restore_active(active, hotkeys, error, plan, install),
    }
}

fn restore_active<H, P>(
    active: &HotkeyConfig,
    hotkeys: &mut Option<H>,
    apply_error: String,
    mut plan: impl FnMut(&HotkeyConfig) -> Result<P, String>,
    mut install: impl FnMut(P) -> Result<H, String>,
) -> InstallOutcome {
    match plan(active).and_then(&mut install) {
        Ok(installed) => {
            *hotkeys = Some(installed);
            InstallOutcome::Reverted(apply_error)
        }
        Err(previous_error) => InstallOutcome::Unavailable {
            error: previous_error,
            not_applied: Some(apply_error),
        },
    }
}

impl MainThreadAgent {
    /// Applies a `[shortcuts]` table read from `config.toml`, on a Settings save or a hand edit.
    /// A table that can't be registered leaves the previous hotkeys working and says why.
    pub fn apply_shortcuts(&mut self, shortcuts: Shortcuts) {
        let workspaces = match &self.hotkey_hold {
            HotkeyHold::Held {
                pending: Some(pending),
            } => pending.workspaces.clone(),
            _ => self.hotkey_workspaces.clone(),
        };
        self.apply_hotkey_config(HotkeyConfig {
            workspaces,
            shortcuts,
        });
    }

    /// Applies `[shortcuts]` and enabled workspace changes from the background config probe in one
    /// hotkey reload.
    pub(super) fn apply_probed_hotkeys(
        &mut self,
        shortcuts: Option<Shortcuts>,
        workspaces: Option<&[(WorkspaceId, DisplaySelector)]>,
    ) {
        let (base_workspaces, base_shortcuts) = match &self.hotkey_hold {
            HotkeyHold::Held {
                pending: Some(pending),
            } => (pending.workspaces.clone(), pending.shortcuts.clone()),
            _ => (self.hotkey_workspaces.clone(), self.shortcuts.clone()),
        };
        let next_workspaces = workspaces.map_or(base_workspaces, |configured| {
            rebuild_hotkey_workspaces(configured, &self.occupied_named_workspaces, self.scope)
        });
        let next_shortcuts = shortcuts.unwrap_or(base_shortcuts);
        self.apply_hotkey_config(HotkeyConfig {
            workspaces: next_workspaces,
            shortcuts: next_shortcuts,
        });
    }

    fn apply_hotkey_config(&mut self, next: HotkeyConfig) {
        let active = HotkeyConfig {
            workspaces: self.hotkey_workspaces.clone(),
            shortcuts: self.shortcuts.clone(),
        };
        let installed = self.hotkeys.is_some();
        let held = self.hotkey_hold.is_held();
        match shortcut_change(&active, &next, installed, held) {
            ShortcutChange::Same => {
                if installed && self.is_shortcuts_not_applied_error() {
                    self.last_error = None;
                    self.ui_dirty = true;
                }
            }
            ShortcutChange::Store => {
                log_event!("hotkeys: shortcuts changed; they register when tiling starts");
                self.hotkey_workspaces = next.workspaces;
                self.shortcuts = next.shortcuts;
            }
            ShortcutChange::Defer => {
                let differs = active.differs_from(&next);
                if let HotkeyHold::Held { pending } = &mut self.hotkey_hold {
                    *pending = differs.then_some(next);
                }
                if !differs && self.is_shortcuts_not_applied_error() {
                    self.last_error = None;
                    self.ui_dirty = true;
                }
            }
            ShortcutChange::Reinstall => {
                self.reinstall_changed_hotkeys(next);
            }
        }
    }

    pub(super) fn is_shortcuts_not_applied_error(&self) -> bool {
        self.last_error
            .as_deref()
            .is_some_and(|error| error.starts_with(NOT_APPLIED))
    }

    /// Drops any active recording hold when automatic tiling stops, storing a pending config so
    /// the next tiling start uses the latest saved shortcuts and workspaces.
    pub(super) fn release_hotkey_hold_on_stop(&mut self) {
        if let HotkeyHold::Held {
            pending: Some(next),
        } = std::mem::take(&mut self.hotkey_hold)
        {
            self.hotkey_workspaces = next.workspaces;
            self.shortcuts = next.shortcuts;
        }
    }

    fn reinstall_changed_hotkeys(&mut self, next: HotkeyConfig) {
        let mut active = HotkeyConfig {
            workspaces: self.hotkey_workspaces.clone(),
            shortcuts: self.shortcuts.clone(),
        };
        let sender = &self.actions_sender;
        let delivery = &mut self.hotkey_delivery;
        let outcome = apply_with_rollback(
            &mut active,
            &mut self.hotkeys,
            next,
            |config| HotkeyPlan::current(&config.workspaces, &config.shortcuts),
            |plan| {
                let hotkeys = Hotkeys::install(sender.clone(), plan)?;
                log_event!("hotkeys: reloaded {} shortcuts", hotkeys.len());
                for skipped in hotkeys.skipped() {
                    log_event!("hotkeys: {skipped}");
                }
                Ok(hotkeys)
            },
            |hotkeys| {
                *hotkeys = None;
                delivery.clear();
            },
        );
        self.hotkey_workspaces = active.workspaces;
        self.shortcuts = active.shortcuts;
        match outcome {
            InstallOutcome::Applied => {
                if self.is_shortcuts_not_applied_error() {
                    self.last_error = None;
                }
                self.ui_dirty = true;
            }
            InstallOutcome::Reverted(error) => {
                self.shortcuts_not_applied(&error);
            }
            InstallOutcome::Unavailable { error, not_applied } => {
                log_event!("hotkeys: could not restore the previous shortcuts: {error}");
                let message = not_applied.as_deref().unwrap_or(&error);
                self.shortcuts_not_applied(message);
                self.stop_automatic_and_restore();
                status_item::set_status(&format!("Hotkeys unavailable: {error}"));
            }
        }
    }

    fn shortcuts_not_applied(&mut self, error: &str) {
        let message = format!("{NOT_APPLIED}: {error}");
        log_event!("hotkeys: {message}");
        self.last_error = Some(message);
        self.ui_dirty = true;
    }

    /// Settings › Shortcuts is recording a chord (`true`) or finished (`false`). While it records,
    /// the hotkeys are unregistered so the chord reaches the recorder instead of firing; they
    /// register again, with any config saved meanwhile, the moment it stops.
    pub fn set_shortcut_recording(&mut self, recording: bool) {
        let now = Instant::now();
        let hold = recording && self.recording_since.get_or_insert(now).elapsed() < RECORDING_LIMIT;
        if !recording {
            self.recording_since = None;
        }
        if hold {
            if self.hotkeys.take().is_some() {
                log_event!("hotkeys: paused while Settings records a shortcut");
                self.hotkey_delivery.clear();
                self.hotkey_hold = HotkeyHold::Held { pending: None };
            }
            return;
        }
        let HotkeyHold::Held { pending } = std::mem::take(&mut self.hotkey_hold) else {
            return;
        };
        if self.hotkeys.is_some() {
            return;
        }
        log_event!("hotkeys: resumed after recording");
        if let Some(next) = pending {
            self.reinstall_changed_hotkeys(next);
            return;
        }
        match HotkeyPlan::current(&self.hotkey_workspaces, &self.shortcuts) {
            Ok(plan) => {
                let preserved = self
                    .is_shortcuts_not_applied_error()
                    .then(|| self.last_error.clone())
                    .flatten();
                self.reinstall_hotkeys(plan);
                if self.hotkeys.is_some()
                    && let Some(message) = preserved
                {
                    self.last_error = Some(message);
                }
            }
            Err(error) => {
                let error = format!("Hotkeys unavailable: {error}");
                log_event!("{error}");
                self.last_error = Some(error.clone());
                self.stop_automatic_and_restore();
                status_item::set_status(&error);
            }
        }
    }
}

const NOT_APPLIED: &str = "Shortcuts not applied";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hotkeys::{CONTROL_KEY, OPTION_KEY},
        keyboard_layout::LayoutSnapshot,
    };

    fn parse(bindings: &str) -> Shortcuts {
        crate::config::Config::parse(&format!("version = 1\n[shortcuts.bindings]\n{bindings}"))
            .unwrap()
            .shortcuts()
            .clone()
    }

    fn parse_custom(defaults: bool, bindings: &str) -> Shortcuts {
        crate::config::Config::parse(&format!(
            "version = 1\n[shortcuts]\ndefaults = {defaults}\n[shortcuts.bindings]\n{bindings}"
        ))
        .unwrap()
        .shortcuts()
        .clone()
    }

    fn config(shortcuts: Shortcuts) -> HotkeyConfig {
        HotkeyConfig {
            workspaces: Vec::new(),
            shortcuts,
        }
    }

    #[test]
    fn a_changed_table_reinstalls_only_registered_hotkeys() {
        let active = config(parse(r#""alt+h" = "none""#));
        let rebound = config(parse(r#""alt+h" = "focus right""#));
        assert_eq!(
            shortcut_change(&active, &rebound, true, false),
            ShortcutChange::Reinstall
        );
        assert_eq!(
            shortcut_change(&active, &rebound, false, false),
            ShortcutChange::Store
        );
        assert_eq!(
            shortcut_change(&active, &rebound, false, true),
            ShortcutChange::Defer
        );
    }

    #[test]
    fn a_respelled_or_untouched_table_is_no_change() {
        let active = config(parse(r#""alt+h" = "none""#));
        let respelled = config(parse(r#""Option + H" = "none""#));
        assert_eq!(
            shortcut_change(&active, &respelled, true, false),
            ShortcutChange::Same
        );
        assert_eq!(
            shortcut_change(
                &config(Shortcuts::default()),
                &config(Shortcuts::default()),
                true,
                false
            ),
            ShortcutChange::Same
        );
    }

    #[test]
    fn recording_an_unplannable_chord_restores_the_previous_keymap_instead_of_discarding_it() {
        let layout = LayoutSnapshot::for_test(
            vec![
                (0x18, '=', false, false),
                (0x18, '+', true, false),
                (0x45, '+', false, false),
                (0x0b, 'b', false, false),
                (0x09, 'v', false, false),
            ],
            vec![],
        );
        let original = config(parse_custom(
            false,
            "'alt++' = 'none'\n'ctrl+alt+b' = 'balance'\n",
        ));
        let unusable = config(parse_custom(
            false,
            "'alt++' = 'none'\n'alt+shift+=' = 'balance'\n",
        ));
        let replacement = config(parse_custom(
            false,
            "'alt++' = 'none'\n'ctrl+alt+v' = 'balance'\n",
        ));

        // Start with the original keymap installed, then suspend hotkeys for recording.
        let mut active = original.clone();
        let initial_plan =
            HotkeyPlan::from_layout(&active.workspaces, &active.shortcuts, &layout).unwrap();
        assert!(initial_plan.has_registration(
            0x0b,
            CONTROL_KEY | OPTION_KEY,
            &WorkerAction::Balance
        ));
        let mut hotkeys: Option<HotkeyPlan> = None; // held while Settings records
        assert_eq!(
            shortcut_change(&active, &unusable, hotkeys.is_some(), true),
            ShortcutChange::Defer
        );

        // Recording ends and tries the unusable table: planning fails, so the previous
        // working keymap (Control+Option+B) is restored and active stays intact.
        let outcome = apply_with_rollback(
            &mut active,
            &mut hotkeys,
            unusable,
            |config| HotkeyPlan::from_layout(&config.workspaces, &config.shortcuts, &layout),
            Ok,
            |slot| *slot = None,
        );
        match outcome {
            InstallOutcome::Reverted(error) => {
                assert!(
                    error.contains("another shortcut already uses that key"),
                    "{error}"
                );
            }
            other => panic!("expected Reverted, got {other:?}"),
        }
        assert_eq!(active, original);
        let restored = hotkeys
            .as_ref()
            .expect("previous working hotkeys must be restored");
        assert!(restored.has_registration(0x0b, CONTROL_KEY | OPTION_KEY, &WorkerAction::Balance));

        // Control: recording the unused Control+Option+V chord while held replaces Control+Option+B.
        hotkeys = None; // held again for the next recording
        let control = apply_with_rollback(
            &mut active,
            &mut hotkeys,
            replacement.clone(),
            |config| HotkeyPlan::from_layout(&config.workspaces, &config.shortcuts, &layout),
            Ok,
            |slot| *slot = None,
        );
        assert_eq!(control, InstallOutcome::Applied);
        assert_eq!(active, replacement);
        let applied = hotkeys.as_ref().expect("replacement hotkeys installed");
        assert!(applied.has_registration(0x09, CONTROL_KEY | OPTION_KEY, &WorkerAction::Balance));
        assert!(!applied.has_registration(0x0b, CONTROL_KEY | OPTION_KEY, &WorkerAction::Balance));
    }

    #[test]
    fn failed_registration_after_recording_hold_also_restores_previous_keymap() {
        let layout = LayoutSnapshot::for_test(
            vec![(0x0b, 'b', false, false), (0x09, 'v', false, false)],
            vec![],
        );
        let original = config(parse_custom(false, "'ctrl+alt+b' = 'balance'\n"));
        let blocked = config(parse_custom(false, "'ctrl+alt+v' = 'balance'\n"));

        let mut active = original.clone();
        let mut hotkeys: Option<HotkeyPlan> = None; // held during recording
        let mut calls = 0usize;
        let outcome = apply_with_rollback(
            &mut active,
            &mut hotkeys,
            blocked,
            |config| HotkeyPlan::from_layout(&config.workspaces, &config.shortcuts, &layout),
            |plan| {
                calls += 1;
                if calls == 1 {
                    Err("skipped balance layout: hotkey is unavailable (-9878)".into())
                } else {
                    Ok(plan)
                }
            },
            |slot| *slot = None,
        );
        assert_eq!(
            outcome,
            InstallOutcome::Reverted(
                "skipped balance layout: hotkey is unavailable (-9878)".into()
            )
        );
        assert_eq!(active, original);
        assert!(hotkeys.as_ref().is_some_and(|plan| plan.has_registration(
            0x0b,
            CONTROL_KEY | OPTION_KEY,
            &WorkerAction::Balance
        )));
    }

    #[test]
    fn named_workspace_shortcuts_register_unregister_defer_and_roll_back() {
        let named_m = WorkspaceId::named('M').unwrap();
        let named_s = WorkspaceId::named('S').unwrap();
        let mut configured: Vec<_> = WorkspaceId::numbered()
            .map(|workspace| (workspace, Workspaces::default_assignment(workspace)))
            .collect();
        configured.push((named_m, DisplaySelector::Main));
        configured.push((named_s, DisplaySelector::Secondary));

        let with_m = rebuild_hotkey_workspaces(&configured, &[], ManagementScope::Main);
        assert!(with_m.contains(&named_m));
        assert!(!with_m.contains(&named_s));

        // Removing M from config keeps its shortcut while M is still occupied, then drops it once empty.
        let without_named: Vec<_> = WorkspaceId::numbered()
            .map(|workspace| (workspace, Workspaces::default_assignment(workspace)))
            .collect();
        assert_eq!(
            rebuild_hotkey_workspaces(&without_named, &[named_m], ManagementScope::Main),
            with_m
        );
        let after_empty = rebuild_hotkey_workspaces(&without_named, &[], ManagementScope::Main);
        assert!(!after_empty.contains(&named_m));

        // Adding a named workspace triggers live reinstall (or defers while recording) and rolls
        // back to the previous workspace list if installation fails.
        let layout = LayoutSnapshot::for_test(
            vec![
                (0x12, '1', false, false),
                (0x2e, 'm', false, false),
                (0x0b, 'b', false, false),
            ],
            vec![],
        );
        let original = HotkeyConfig {
            workspaces: vec![WorkspaceId::ONE],
            shortcuts: parse_custom(true, ""),
        };
        let added = HotkeyConfig {
            workspaces: vec![WorkspaceId::ONE, named_m],
            shortcuts: parse_custom(true, ""),
        };
        assert_eq!(
            shortcut_change(&original, &added, true, false),
            ShortcutChange::Reinstall
        );
        assert_eq!(
            shortcut_change(&original, &added, false, true),
            ShortcutChange::Defer
        );

        let mut active = original.clone();
        let mut hotkeys =
            Some(HotkeyPlan::from_layout(&active.workspaces, &active.shortcuts, &layout).unwrap());
        let mut calls = 0usize;
        let rolled_back = apply_with_rollback(
            &mut active,
            &mut hotkeys,
            added.clone(),
            |config| HotkeyPlan::from_layout(&config.workspaces, &config.shortcuts, &layout),
            |plan| {
                calls += 1;
                if calls == 1 {
                    Err("hotkey unavailable".into())
                } else {
                    Ok(plan)
                }
            },
            |slot| *slot = None,
        );
        assert_eq!(
            rolled_back,
            InstallOutcome::Reverted("hotkey unavailable".into())
        );
        assert_eq!(active, original);
        assert!(hotkeys.as_ref().is_some_and(|plan| !plan.has_registration(
            0x2e,
            OPTION_KEY,
            &WorkerAction::SwitchWorkspace(named_m)
        )));

        let applied = apply_with_rollback(
            &mut active,
            &mut hotkeys,
            added.clone(),
            |config| HotkeyPlan::from_layout(&config.workspaces, &config.shortcuts, &layout),
            Ok,
            |slot| *slot = None,
        );
        assert_eq!(applied, InstallOutcome::Applied);
        assert_eq!(active, added);
        assert!(hotkeys.as_ref().is_some_and(|plan| plan.has_registration(
            0x2e,
            OPTION_KEY,
            &WorkerAction::SwitchWorkspace(named_m)
        )));
    }
}
