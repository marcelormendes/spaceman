//! `[shortcuts]` and named-workspace shortcuts applied while Spaceman runs: a changed table or
//! workspace set re-registers the global hotkeys at once, and Settings › Shortcuts holds them off
//! while it records a chord.

use crate::shortcuts::Shortcuts;

use super::*;

/// The longest Settings may hold the hotkeys off for one recording. A recorder left listening
/// (a window that never reported losing focus) must not cost the user every shortcut.
const RECORDING_LIMIT: Duration = Duration::from_secs(60);

/// Whether Settings › Shortcuts is holding the hotkeys off for a recording, and any
/// `[shortcuts]` table or managed workspace set saved while the hold is active.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum HotkeyHold {
    #[default]
    None,
    /// Hotkeys were unregistered for recording; `MainThreadAgent::shortcuts` and
    /// `MainThreadAgent::hotkey_workspaces` keep the last working pair so a saved replacement
    /// that fails to plan or register rolls back to them.
    Held {
        pending_shortcuts: Option<Shortcuts>,
        pending_workspaces: Option<Vec<WorkspaceId>>,
    },
}

impl HotkeyHold {
    const fn is_held(&self) -> bool {
        matches!(self, Self::Held { .. })
    }
}

/// What a `[shortcuts]` or workspace-set change asks of the registered hotkeys.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum ShortcutChange {
    /// The same hotkeys, perhaps spelled differently: nothing to do.
    Same,
    /// Keep the table for the next registration; none is installed or held now.
    Store,
    /// Hotkeys are suspended while Settings records; try this table when recording ends.
    Defer,
    /// Register the new table in place of the installed hotkeys.
    Reinstall,
}

pub(super) fn hotkey_change(changed: bool, installed: bool, held: bool) -> ShortcutChange {
    if held {
        ShortcutChange::Defer
    } else if !changed {
        ShortcutChange::Same
    } else if installed {
        ShortcutChange::Reinstall
    } else {
        ShortcutChange::Store
    }
}

#[cfg(test)]
pub(super) fn shortcut_change(
    active: &Shortcuts,
    next: &Shortcuts,
    installed: bool,
    held: bool,
) -> ShortcutChange {
    hotkey_change(active.differs_from(next), installed, held)
}

/// Result of attempting to activate a replacement shortcut/workspace configuration over the
/// active one.
#[derive(Debug, Eq, PartialEq)]
enum InstallOutcome {
    /// The replacement configuration is now active and installed.
    Applied,
    /// The replacement could not be applied; the previous working configuration is active and
    /// `String` explains why.
    Reverted(String),
    /// Neither the replacement nor the previous configuration could be installed.
    Unavailable {
        error: String,
        not_applied: Option<String>,
    },
}

/// Applies `(next_workspaces, next_shortcuts)` over `(active_workspaces, active_shortcuts)`,
/// rolling both back if planning or registering the replacement fails.
///
/// `hotkeys` is `Some` during a live reload, or `None` when resuming after a recording hold (in
/// which case a planning failure must also re-register the active pair).
#[allow(clippy::too_many_arguments)]
fn apply_with_rollback<H, P>(
    active_workspaces: &mut Vec<WorkspaceId>,
    active_shortcuts: &mut Shortcuts,
    hotkeys: &mut Option<H>,
    next_workspaces: Vec<WorkspaceId>,
    next_shortcuts: Shortcuts,
    mut plan: impl FnMut(&[WorkspaceId], &Shortcuts) -> Result<P, String>,
    mut install: impl FnMut(P) -> Result<H, String>,
    mut release_before_install: impl FnMut(&mut Option<H>),
) -> InstallOutcome {
    let planned = match plan(&next_workspaces, &next_shortcuts) {
        Ok(planned) => planned,
        Err(error) => {
            if hotkeys.is_some() {
                return InstallOutcome::Reverted(error);
            }
            return restore_active(
                active_workspaces,
                active_shortcuts,
                hotkeys,
                error,
                plan,
                install,
            );
        }
    };
    // Carbon cannot register the new chords until the old ones are released.
    release_before_install(hotkeys);
    match install(planned) {
        Ok(installed) => {
            *active_workspaces = next_workspaces;
            *active_shortcuts = next_shortcuts;
            *hotkeys = Some(installed);
            InstallOutcome::Applied
        }
        Err(error) => restore_active(
            active_workspaces,
            active_shortcuts,
            hotkeys,
            error,
            plan,
            install,
        ),
    }
}

fn restore_active<H, P>(
    active_workspaces: &[WorkspaceId],
    active_shortcuts: &Shortcuts,
    hotkeys: &mut Option<H>,
    apply_error: String,
    mut plan: impl FnMut(&[WorkspaceId], &Shortcuts) -> Result<P, String>,
    mut install: impl FnMut(P) -> Result<H, String>,
) -> InstallOutcome {
    match plan(active_workspaces, active_shortcuts).and_then(&mut install) {
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

fn rebuild_hotkey_workspaces(
    current: &[WorkspaceId],
    configured: &[(WorkspaceId, DisplaySelector)],
    occupied_named: &[WorkspaceId],
    scope: ManagementScope,
) -> Option<Vec<WorkspaceId>> {
    let mut next: Vec<_> = configured
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
    let mut existing = current.to_vec();
    existing.sort();
    existing.dedup();
    (next != existing).then_some(next)
}

impl MainThreadAgent {
    /// Applies a `[shortcuts]` table read from `config.toml` or saved by Settings › Shortcuts.
    /// A table that can't be registered leaves the previous hotkeys working and says why.
    pub fn apply_shortcuts(&mut self, shortcuts: Shortcuts) {
        self.apply_hotkey_config(None, Some(shortcuts));
    }

    /// Applies both workspace-list changes (adding/removing named workspaces or changing display
    /// assignments) and `[shortcuts]` from a reloaded `config.toml` in a single registration pass.
    pub(super) fn apply_config_hotkeys(
        &mut self,
        configured: &[(WorkspaceId, DisplaySelector)],
        shortcuts: Shortcuts,
    ) {
        let workspaces = rebuild_hotkey_workspaces(
            &self.hotkey_workspaces,
            configured,
            &self.occupied_named_workspaces,
            self.scope,
        )
        .unwrap_or_else(|| self.hotkey_workspaces.clone());
        self.apply_hotkey_config(Some(workspaces), Some(shortcuts));
    }

    fn apply_hotkey_config(
        &mut self,
        next_workspaces: Option<Vec<WorkspaceId>>,
        next_shortcuts: Option<Shortcuts>,
    ) {
        let installed = self.hotkeys.is_some();
        let held = self.hotkey_hold.is_held();
        let workspaces_changed = next_workspaces
            .as_ref()
            .is_some_and(|workspaces| workspaces != &self.hotkey_workspaces);
        let shortcuts_changed = next_shortcuts
            .as_ref()
            .is_some_and(|shortcuts| self.shortcuts.differs_from(shortcuts));
        match hotkey_change(workspaces_changed || shortcuts_changed, installed, held) {
            ShortcutChange::Same => {
                if installed && self.is_shortcuts_not_applied_error() {
                    self.last_error = None;
                    self.ui_dirty = true;
                }
            }
            ShortcutChange::Store => {
                log_event!("hotkeys: shortcuts changed; they register when tiling starts");
                if let Some(workspaces) = next_workspaces {
                    self.hotkey_workspaces = workspaces;
                }
                if let Some(shortcuts) = next_shortcuts {
                    self.shortcuts = shortcuts;
                }
            }
            ShortcutChange::Defer => {
                if let HotkeyHold::Held {
                    pending_shortcuts,
                    pending_workspaces,
                } = &mut self.hotkey_hold
                {
                    if let Some(workspaces) = next_workspaces {
                        *pending_workspaces =
                            (workspaces != self.hotkey_workspaces).then_some(workspaces);
                    }
                    if let Some(shortcuts) = next_shortcuts {
                        *pending_shortcuts =
                            self.shortcuts.differs_from(&shortcuts).then_some(shortcuts);
                    }
                    if pending_shortcuts.is_none()
                        && pending_workspaces.is_none()
                        && self.is_shortcuts_not_applied_error()
                    {
                        self.last_error = None;
                        self.ui_dirty = true;
                    }
                }
            }
            ShortcutChange::Reinstall => {
                let target_workspaces =
                    next_workspaces.unwrap_or_else(|| self.hotkey_workspaces.clone());
                let target_shortcuts = next_shortcuts.unwrap_or_else(|| self.shortcuts.clone());
                self.reinstall_changed_hotkeys(target_workspaces, target_shortcuts);
            }
        }
    }

    pub(super) fn is_shortcuts_not_applied_error(&self) -> bool {
        self.last_error
            .as_deref()
            .is_some_and(|error| error.starts_with(NOT_APPLIED))
    }

    /// Drops any active recording hold when automatic tiling stops, storing any pending table
    /// and workspace set so the next tiling start uses the latest saved configuration.
    pub(super) fn release_hotkey_hold_on_stop(&mut self) {
        if let HotkeyHold::Held {
            pending_shortcuts,
            pending_workspaces,
        } = std::mem::take(&mut self.hotkey_hold)
        {
            if let Some(next) = pending_workspaces {
                self.hotkey_workspaces = next;
            }
            if let Some(next) = pending_shortcuts {
                self.shortcuts = next;
            }
        }
    }

    fn reinstall_changed_hotkeys(
        &mut self,
        next_workspaces: Vec<WorkspaceId>,
        next_shortcuts: Shortcuts,
    ) {
        let sender = &self.actions_sender;
        let delivery = &mut self.hotkey_delivery;
        let outcome = apply_with_rollback(
            &mut self.hotkey_workspaces,
            &mut self.shortcuts,
            &mut self.hotkeys,
            next_workspaces,
            next_shortcuts,
            HotkeyPlan::current,
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
    /// register again, with any table or workspace list saved meanwhile, the moment it stops.
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
                self.hotkey_hold = HotkeyHold::Held {
                    pending_shortcuts: None,
                    pending_workspaces: None,
                };
            }
            return;
        }
        let HotkeyHold::Held {
            pending_shortcuts,
            pending_workspaces,
        } = std::mem::take(&mut self.hotkey_hold)
        else {
            return;
        };
        if self.hotkeys.is_some() {
            return;
        }
        log_event!("hotkeys: resumed after recording");
        if pending_shortcuts.is_some() || pending_workspaces.is_some() {
            let next_workspaces =
                pending_workspaces.unwrap_or_else(|| self.hotkey_workspaces.clone());
            let next_shortcuts = pending_shortcuts.unwrap_or_else(|| self.shortcuts.clone());
            self.reinstall_changed_hotkeys(next_workspaces, next_shortcuts);
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

    fn numbered_with_defaults() -> Vec<(WorkspaceId, DisplaySelector)> {
        WorkspaceId::numbered()
            .map(|workspace| (workspace, Workspaces::default_assignment(workspace)))
            .collect()
    }

    #[test]
    fn a_changed_table_reinstalls_only_registered_hotkeys() {
        let active = parse(r#""alt+h" = "none""#);
        let rebound = parse(r#""alt+h" = "focus right""#);
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
        let active = parse(r#""alt+h" = "none""#);
        let respelled = parse(r#""Option + H" = "none""#);
        assert_eq!(
            shortcut_change(&active, &respelled, true, false),
            ShortcutChange::Same
        );
        assert_eq!(
            shortcut_change(&Shortcuts::default(), &Shortcuts::default(), true, false),
            ShortcutChange::Same
        );
    }

    #[test]
    fn rebuilds_named_workspace_shortcuts_symmetrically_and_keeps_occupied_ones() {
        let mut base: Vec<_> = WorkspaceId::numbered().take(5).collect();
        let named_m = WorkspaceId::named('M').unwrap();
        let named_s = WorkspaceId::named('S').unwrap();

        let mut configured = numbered_with_defaults();
        configured.push((named_m, DisplaySelector::Main));
        configured.push((named_s, DisplaySelector::Secondary));

        let added = rebuild_hotkey_workspaces(&base, &configured, &[], ManagementScope::Main)
            .expect("new managed named workspace should register");
        assert!(added.contains(&named_m));
        assert!(!added.contains(&named_s));

        // Unsorted current list with the same membership must not trigger a reinstall.
        let mut unsorted = added.clone();
        unsorted.reverse();
        assert_eq!(
            rebuild_hotkey_workspaces(&unsorted, &configured, &[], ManagementScope::Main),
            None
        );

        // Removing M while it still holds windows keeps M registered.
        let without_m = numbered_with_defaults();
        assert_eq!(
            rebuild_hotkey_workspaces(&added, &without_m, &[named_m], ManagementScope::Main),
            None
        );

        // Once M is empty and removed from config, its shortcut unregisters live.
        let removed = rebuild_hotkey_workspaces(&added, &without_m, &[], ManagementScope::Main)
            .expect("removed empty named workspace should unregister");
        base.sort();
        assert_eq!(removed, base);
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
        let original = parse_custom(false, "'alt++' = 'none'\n'ctrl+alt+b' = 'balance'\n");
        let unusable = parse_custom(false, "'alt++' = 'none'\n'alt+shift+=' = 'balance'\n");
        let replacement = parse_custom(false, "'alt++' = 'none'\n'ctrl+alt+v' = 'balance'\n");

        // Start with the original keymap installed, then suspend hotkeys for recording.
        let mut active_workspaces = Vec::new();
        let mut active = original.clone();
        let initial_plan = HotkeyPlan::from_layout(&active_workspaces, &active, &layout).unwrap();
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
            &mut active_workspaces,
            &mut active,
            &mut hotkeys,
            Vec::new(),
            unusable,
            |workspaces, shortcuts| HotkeyPlan::from_layout(workspaces, shortcuts, &layout),
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
            &mut active_workspaces,
            &mut active,
            &mut hotkeys,
            Vec::new(),
            replacement.clone(),
            |workspaces, shortcuts| HotkeyPlan::from_layout(workspaces, shortcuts, &layout),
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
        let original = parse_custom(false, "'ctrl+alt+b' = 'balance'\n");
        let blocked = parse_custom(false, "'ctrl+alt+v' = 'balance'\n");

        let mut active_workspaces = Vec::new();
        let mut active = original.clone();
        let mut hotkeys: Option<HotkeyPlan> = None; // held during recording
        let mut calls = 0usize;
        let outcome = apply_with_rollback(
            &mut active_workspaces,
            &mut active,
            &mut hotkeys,
            Vec::new(),
            blocked,
            |workspaces, shortcuts| HotkeyPlan::from_layout(workspaces, shortcuts, &layout),
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
    fn named_workspace_reload_registers_live_and_rolls_back_both_workspaces_and_shortcuts_on_error()
    {
        let layout = LayoutSnapshot::for_test(
            vec![
                (0x2e, 'm', false, false),
                (0x0b, 'b', false, false),
                (0x18, '=', false, false),
                (0x18, '+', true, false),
                (0x45, '+', false, false),
            ],
            vec![],
        );
        let named_m = WorkspaceId::named('M').unwrap();
        let original_shortcuts = parse_custom(true, "'ctrl+alt+b' = 'balance'\n");
        let mut active_workspaces = Vec::new();
        let mut active_shortcuts = original_shortcuts.clone();
        let mut hotkeys =
            Some(HotkeyPlan::from_layout(&active_workspaces, &active_shortcuts, &layout).unwrap());

        // Adding named workspace M registers Option+M live through the same path.
        let added = apply_with_rollback(
            &mut active_workspaces,
            &mut active_shortcuts,
            &mut hotkeys,
            vec![named_m],
            original_shortcuts.clone(),
            |workspaces, shortcuts| HotkeyPlan::from_layout(workspaces, shortcuts, &layout),
            Ok,
            |slot| *slot = None,
        );
        assert_eq!(added, InstallOutcome::Applied);
        assert_eq!(active_workspaces, vec![named_m]);
        assert!(hotkeys.as_ref().is_some_and(|plan| plan.has_registration(
            0x2e,
            OPTION_KEY,
            &WorkerAction::SwitchWorkspace(named_m)
        )));

        // A combined reload with an unplannable shortcut reverts both workspaces and shortcuts.
        let unusable = parse_custom(false, "'alt++' = 'none'\n'alt+shift+=' = 'balance'\n");
        let reverted = apply_with_rollback(
            &mut active_workspaces,
            &mut active_shortcuts,
            &mut hotkeys,
            Vec::new(),
            unusable,
            |workspaces, shortcuts| HotkeyPlan::from_layout(workspaces, shortcuts, &layout),
            Ok,
            |slot| *slot = None,
        );
        assert!(matches!(reverted, InstallOutcome::Reverted(_)));
        assert_eq!(active_workspaces, vec![named_m]);
        assert_eq!(active_shortcuts, original_shortcuts);
        assert!(hotkeys.as_ref().is_some_and(|plan| plan.has_registration(
            0x2e,
            OPTION_KEY,
            &WorkerAction::SwitchWorkspace(named_m)
        )));
    }
}
