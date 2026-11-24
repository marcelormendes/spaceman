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

    /// Rebuilds global workspace shortcuts when `config.toml` adds, removes, or
    /// reassigns workspaces, without requiring an app restart.
    pub(super) fn refresh_hotkey_workspaces(
        &mut self,
        configured: &[(WorkspaceId, DisplaySelector)],
    ) {
        let Some(next) = rebuild_hotkey_workspaces(
            &self.hotkey_workspaces,
            configured,
            &self.occupied_named_workspaces,
            self.scope,
        ) else {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered_with_defaults() -> Vec<(WorkspaceId, DisplaySelector)> {
        WorkspaceId::numbered()
            .map(|workspace| (workspace, Workspaces::default_assignment(workspace)))
            .collect()
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
}
