use super::*;

/// Shortest gap between two re-registrations caused by undelivered presses.
const HOTKEY_HEAL_COOLDOWN: Duration = Duration::from_secs(5);

impl MainThreadAgent {
    pub(super) fn hotkey_delivered(&mut self, id: u32) {
        let Some((action, description)) = self.hotkey_binding(id) else {
            return;
        };
        log_event!("hotkey: {description}");
        self.hotkey_delivery.delivered(id, Instant::now());
        if self.automatic {
            self.queue_worker_input(QueuedInput::Action(action));
        }
    }

    pub(super) fn option_key_down(&mut self, key_code: u32, modifiers: u32) {
        if let Some(id) = self
            .hotkeys
            .as_ref()
            .and_then(|hotkeys| hotkeys.id_for(key_code, modifiers))
        {
            self.hotkey_delivery.pressed(id, Instant::now());
        }
    }

    /// A shortcut the keyboard tap saw but Carbon never delivered means macOS
    /// dropped the registrations: register them again and run the missed press.
    pub(super) fn heal_missed_hotkey(&mut self) {
        let Some(id) = self.hotkey_delivery.missed(Instant::now()) else {
            return;
        };
        let Some((action, description)) = self.hotkey_binding(id) else {
            return;
        };
        if self
            .last_hotkey_heal
            .is_some_and(|healed| healed.elapsed() < HOTKEY_HEAL_COOLDOWN)
        {
            log_event!("hotkey: {description} pressed but macOS did not deliver it");
        } else {
            log_event!(
                "hotkey: {description} pressed but macOS did not deliver it; registering shortcuts again"
            );
            self.last_hotkey_heal = Some(Instant::now());
            match HotkeyPlan::current(&self.hotkey_workspaces) {
                Ok(plan) => self.reinstall_hotkeys(plan),
                Err(error) => log_event!("hotkeys: could not plan shortcuts: {error}"),
            }
        }
        if self.automatic {
            self.queue_worker_input(QueuedInput::Action(action));
        }
    }

    /// Registers global shortcuts for any newly configured named workspaces
    /// without requiring an app restart.
    pub(super) fn refresh_hotkey_workspaces(&mut self, named: &[(WorkspaceId, DisplaySelector)]) {
        let Some(next) = merge_hotkey_workspaces(&self.hotkey_workspaces, named, self.scope) else {
            return;
        };
        self.hotkey_workspaces = next;
        if self.hotkeys.is_some() {
            match HotkeyPlan::current(&self.hotkey_workspaces) {
                Ok(plan) => self.reinstall_hotkeys(plan),
                Err(error) => log_event!("hotkeys: could not plan shortcuts: {error}"),
            }
        }
    }

    pub(super) fn reinstall_hotkeys(&mut self, plan: HotkeyPlan) {
        // Carbon cannot register the replacement chords until the old ones are released.
        self.hotkeys = None;
        self.hotkey_delivery.clear();
        match Hotkeys::install(self.actions_sender.clone(), plan) {
            Ok(hotkeys) => {
                log_event!("hotkeys: registered {} shortcuts", hotkeys.len());
                self.hotkeys = Some(hotkeys);
                self.last_error = None;
                self.ui_dirty = true;
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

    fn hotkey_binding(&self, id: u32) -> Option<(WorkerAction, String)> {
        let (action, description) = self.hotkeys.as_ref()?.binding(id)?;
        Some((action.clone(), description.to_owned()))
    }
}

fn merge_hotkey_workspaces(
    current: &[WorkspaceId],
    named: &[(WorkspaceId, DisplaySelector)],
    scope: ManagementScope,
) -> Option<Vec<WorkspaceId>> {
    let mut next = current.to_vec();
    for (workspace, selector) in named {
        if scope.manages_selector(selector) && !next.contains(workspace) {
            next.push(*workspace);
        }
    }
    next.sort();
    (next != current).then_some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_new_managed_named_workspaces_and_skips_unmanaged_or_existing() {
        let base: Vec<_> = WorkspaceId::numbered().collect();
        let named_m = WorkspaceId::named('M').unwrap();
        let named_s = WorkspaceId::named('S').unwrap();

        let merged = merge_hotkey_workspaces(
            &base,
            &[
                (named_m, DisplaySelector::Main),
                (named_s, DisplaySelector::Secondary),
            ],
            ManagementScope::Main,
        )
        .expect("new managed named workspace should update list");
        assert!(merged.contains(&named_m));
        assert!(!merged.contains(&named_s));
        assert_eq!(
            merge_hotkey_workspaces(
                &merged,
                &[(named_m, DisplaySelector::Main)],
                ManagementScope::Main
            ),
            None
        );
    }
}
