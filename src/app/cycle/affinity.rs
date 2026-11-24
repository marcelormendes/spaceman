use super::*;
use crate::journal::PlacementSource;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FrozenWindowAffinity {
    Owned(WorkspaceId),
    PendingReplacement(WorkspaceId),
    AmbiguousReplacement,
    Floating,
    ExplicitRoute(WorkspaceId),
    /// The app's existing workspace, or a remembered placement a relaunched
    /// application's window claimed.
    Inferred(WorkspaceId),
    CurrentDisplay,
    /// Never tiled, owned, or parked (ignore rule or parking exemption).
    Ignored,
}

pub(super) struct FrozenAffinityPolicy {
    pub(super) windows: HashMap<crate::PhysicalWindowId, FrozenWindowAffinity>,
    pub(super) visible_workspaces: HashSet<WorkspaceId>,
    /// Newly observed non-resizable windows that matched a `route` rule and must
    /// be owned as floating on their target workspace.
    pub(super) floating_routes: Vec<(crate::PhysicalWindowId, WorkspaceId, crate::Rect)>,
}

impl<B: WorkspaceBackend> WorkspaceRuntime<B> {
    pub(super) fn freeze_window_affinities(
        &mut self,
        observation: &crate::NativeObservation,
        visible: &[ResolvedWorkspace],
        server_frames: &HashMap<u32, crate::Rect>,
    ) -> Result<FrozenAffinityPolicy, String> {
        let has_floating = self.workspaces.floating_entries().next().is_some();
        if has_floating {
            let floating_apps = self
                .workspaces
                .floating_entries()
                .map(|(window, _)| window.app.clone())
                .collect::<HashSet<_>>();
            for app in floating_apps {
                if !self.backend.is_process_running(&app) {
                    for window in self.workspaces.floating_windows_for_app(&app) {
                        log_event!("forget floating {:?}: app exited", window);
                        self.workspaces.forget_floating(&window);
                    }
                }
            }
        }
        self.forget_exited_apps_on_hidden_workspaces(visible);

        self.close_recalls();
        // Before any new instance below is considered: a relaunch's new
        // process can appear in the same observation that reports the old
        // one terminated.
        self.note_absent_departures(observation);
        let mut affinities = HashMap::new();
        let mut floating_routes = Vec::new();
        // Read once, and only if a Complete snapshot omits an owned float.
        let mut onscreen_frames = None;
        for application in &observation.applications {
            let crate::AppObservation::Complete { app, windows, .. } = application else {
                continue;
            };
            if has_floating {
                let floating_windows = self.workspaces.floating_windows_for_app(&app.id);
                for window in floating_windows {
                    if windows.iter().any(|observed| observed.id == window)
                        || self.parked.contains_key(&window)
                    {
                        self.omitted_windows.remove(&window);
                    } else if !self.omitted_float_is_settling(&window, &mut onscreen_frames) {
                        log_event!("forget floating {:?}: not observed and not parked", window);
                        self.workspaces.forget_floating(&window);
                        self.omitted_windows.remove(&window);
                    }
                }
            }
            self.prepare_recall(app, windows);
            // Freeze prior ownership for the whole snapshot so all windows of a
            // newly seen app follow its route rule together; later cycles still
            // see the updated ownership so subsequent secondary windows stay on
            // the active workspace.
            let app_owns_window = self.workspaces.app_owns_windows(&app.id);
            let app_owns_tiled = self.workspaces.app_owns_tiled_windows(&app.id);
            for window in windows {
                self.heal_reappeared_window(window, visible, server_frames);
                if self.parked.contains_key(&window.id)
                    && self
                        .scratchpad
                        .entry(&window.id)
                        .is_some_and(|entry| entry.shown_on.is_none())
                {
                    // Still hidden in the scratchpad (re-parked or settling); only an
                    // honored reappearance clears its parked record and lets it adopt.
                    affinities.insert(window.id.clone(), FrozenWindowAffinity::Ignored);
                    continue;
                }
                let disposition = self.config.disposition(&app.bundle_id, window);
                if disposition == WindowDisposition::Ignored {
                    if let Some(owner) = self.workspaces.workspace_of_any(&window.id) {
                        log_event!("release {:?} from workspace {}: ignored", window.id, owner);
                        self.forget_window(owner, &window.id);
                    }
                    affinities.insert(window.id.clone(), FrozenWindowAffinity::Ignored);
                    continue;
                }
                // A float rule, not a manual override, floats this window. An
                // agent restart seeds every restored float as manual because the
                // saved state does not say why it floated, so hand it back to the
                // rule: removing the rule then tiles it again (#160).
                if disposition == WindowDisposition::Floating
                    && !self.scratchpad.contains(&window.id)
                {
                    self.manual_floating.remove(&window.id);
                }
                // A window that already holds a tile keeps it while it temporarily
                // looks non-resizable (for example while an attached window-modal
                // close or save sheet disables its fullscreen button or AXSize).
                // A float rule or Toggle Float still releases the tile immediately.
                let tiled = self.workspaces.workspace_of(&window.id).is_some();
                let floating = disposition == WindowDisposition::Floating
                    || self.manual_floating.contains(&window.id)
                    || (!window.resizable && !tiled);
                if floating {
                    if let Some(owner) = self.workspaces.workspace_of(&window.id) {
                        self.forget_window(owner, &window.id);
                        self.workspaces.own_floating(window.id.clone(), owner);
                    }
                } else if self.workspaces.floating_workspace_of(&window.id).is_some() {
                    self.workspaces.forget_floating(&window.id);
                }
                let affinity = if let Some(owner) = self.workspaces.workspace_of(&window.id) {
                    FrozenWindowAffinity::Owned(owner)
                } else if !floating
                    && let Some(affinity) = {
                        let mut reserved = self.workspaces.replacement_workspaces(&window.id);
                        reserved.next().map(|workspace| {
                            if reserved.next().is_some() {
                                FrozenWindowAffinity::AmbiguousReplacement
                            } else {
                                FrozenWindowAffinity::PendingReplacement(workspace)
                            }
                        })
                    }
                {
                    affinity
                } else if floating && self.workspaces.floating_workspace_of(&window.id).is_some() {
                    FrozenWindowAffinity::Floating
                } else if floating && let Some(placement) = self.memory.recalled(&window.id) {
                    self.workspaces
                        .own_floating(window.id.clone(), placement.workspace);
                    self.note_placement(&window.id, PlacementSource::Recall, placement.workspace);
                    FrozenWindowAffinity::Floating
                } else if floating {
                    let routed = match disposition {
                        WindowDisposition::Route(target)
                            if (!app_owns_window
                                || self.config.route_matches_title(&app.bundle_id, window))
                                && self
                                    .scope
                                    .manages_selector(self.workspaces.assignment(target)) =>
                        {
                            Some(target)
                        }
                        _ => None,
                    };
                    if let Some(target) = routed {
                        floating_routes.push((window.id.clone(), target, window.frame));
                        FrozenWindowAffinity::Floating
                    } else {
                        visible
                            .iter()
                            .find(|resolved| resolved.output.contains_center(window.frame))
                            .map_or(FrozenWindowAffinity::CurrentDisplay, |resolved| {
                                log_event!(
                                    "adopt floating {:?} -> workspace {}",
                                    window.id,
                                    resolved.workspace
                                );
                                self.workspaces
                                    .own_floating(window.id.clone(), resolved.workspace);
                                self.note_placement(
                                    &window.id,
                                    PlacementSource::Active,
                                    resolved.workspace,
                                );
                                FrozenWindowAffinity::Floating
                            })
                    }
                } else {
                    match disposition {
                        WindowDisposition::Route(target) => {
                            if !self
                                .scope
                                .manages_selector(self.workspaces.assignment(target))
                            {
                                return Err(format!(
                                    "window rule routes to unmanaged workspace {}",
                                    target
                                ));
                            }
                            if !app_owns_tiled
                                || self.config.route_matches_title(&app.bundle_id, window)
                            {
                                FrozenWindowAffinity::ExplicitRoute(target)
                            } else {
                                FrozenWindowAffinity::CurrentDisplay
                            }
                        }
                        WindowDisposition::Managed => self
                            .memory
                            .recalled(&window.id)
                            .map(|placement| FrozenWindowAffinity::Inferred(placement.workspace))
                            .unwrap_or(FrozenWindowAffinity::CurrentDisplay),
                        WindowDisposition::Floating | WindowDisposition::Ignored => unreachable!(),
                    }
                };
                affinities.insert(window.id.clone(), affinity);
            }
        }
        Ok(FrozenAffinityPolicy {
            windows: affinities,
            visible_workspaces: visible.iter().map(|resolved| resolved.workspace).collect(),
            floating_routes,
        })
    }

    /// Route newly observed non-resizable windows that matched a `route` rule,
    /// keeping them floating on their target workspace.
    pub(super) fn apply_floating_routes(
        &mut self,
        routes: &[(crate::PhysicalWindowId, WorkspaceId, crate::Rect)],
        visible: &[ResolvedWorkspace],
        outputs: &macos::DisplayOutputs,
        focus: (Option<&crate::PhysicalWindowId>, Option<&AppInstanceId>),
        failed: &mut usize,
    ) {
        if routes.is_empty() {
            return;
        }
        let mut moved = false;
        for (window, target, frame) in routes {
            if self.workspaces.workspace_of_any(window).is_some() {
                continue;
            }
            let target_output = output_for(&self.workspaces, *target, outputs);
            let source_output = visible
                .iter()
                .find(|resolved| resolved.output.contains_center(*frame))
                .map_or(target_output, |resolved| resolved.output);
            if visible.iter().any(|resolved| resolved.workspace == *target) {
                if source_output != target_output
                    && !self.carry_floating_window(window, source_output, target_output)
                {
                    *failed += 1;
                    continue;
                }
                log_event!("route floating {:?} -> workspace {}", window, target);
                self.workspaces.own_floating(window.clone(), *target);
                self.note_placement(window, PlacementSource::Rule, *target);
                moved = true;
            } else {
                let outcome = if source_output == target_output {
                    self.park_window(window, target_output, outputs)
                } else {
                    self.park_window_across(window, source_output, target_output, outputs)
                };
                match outcome {
                    macos::ApplyOutcome::Applied => {
                        log_event!(
                            "route floating {:?} -> workspace {} (parked)",
                            window,
                            target
                        );
                        self.suppress_parked_focus(window, focus.0, focus.1);
                        self.workspaces.own_floating(window.clone(), *target);
                        self.note_placement(window, PlacementSource::Rule, *target);
                        moved = true;
                    }
                    macos::ApplyOutcome::Gone => {}
                    _ => {
                        *failed += 1;
                    }
                }
            }
        }
        if moved && let Err(error) = self.persist_workspace_state_if_changed() {
            log_event!("failed to persist routed floating windows: {error}");
        }
    }

    pub(super) fn apply_inserted_routes(
        &mut self,
        resolved: &ResolvedWorkspace,
        routes: &[(crate::PhysicalWindowId, WorkspaceId)],
        affinity_policy: &FrozenAffinityPolicy,
        outputs: &macos::DisplayOutputs,
        focus: (Option<&crate::PhysicalWindowId>, Option<&AppInstanceId>),
        failed: &mut usize,
    ) -> HashSet<WorkspaceId> {
        let mut routed_targets = HashSet::new();
        for (window, target) in routes {
            if !affinity_policy.visible_workspaces.contains(target) {
                let target_output = output_for(&self.workspaces, *target, outputs);
                match self.park_window_across(window, resolved.output, target_output, outputs) {
                    macos::ApplyOutcome::Applied => {
                        self.suppress_parked_focus(window, focus.0, focus.1);
                    }
                    macos::ApplyOutcome::Gone => {
                        self.forget_window(resolved.workspace, window);
                        continue;
                    }
                    _ => {
                        *failed += 1;
                        continue;
                    }
                }
            }
            if self.workspaces.move_window(window, *target).is_none() {
                continue;
            }
            routed_targets.insert(*target);
        }
        routed_targets
    }

    /// Hidden workspaces are not reconciled, so an app that quit while its
    /// windows were parked would otherwise keep their tiles until the
    /// workspace was shown again.
    fn forget_exited_apps_on_hidden_workspaces(&mut self, visible: &[ResolvedWorkspace]) {
        let mut running = HashMap::new();
        for workspace in WorkspaceId::all() {
            if visible
                .iter()
                .any(|resolved| resolved.workspace == workspace)
            {
                continue;
            }
            let apps = self
                .workspaces
                .app_instances(workspace)
                .cloned()
                .collect::<HashSet<_>>();
            for app in apps {
                let alive = *running
                    .entry(app.clone())
                    .or_insert_with(|| self.backend.is_process_running(&app));
                if alive {
                    continue;
                }
                let windows = self
                    .workspaces
                    .engine(workspace)
                    .model()
                    .workspace
                    .slots
                    .iter()
                    .filter(|slot| slot.belongs_to(&app))
                    .map(|slot| slot.identity_window().clone())
                    .collect::<Vec<_>>();
                self.note_departure(&app);
                self.workspaces.forget_app(workspace, &app);
                for window in &windows {
                    self.forget_window(workspace, window);
                }
                self.known_apps[workspace.index()].remove(&app);
                log_event!(
                    "workspace {workspace}: forgot {app:?}: app exited while hidden ({} tile(s))",
                    windows.len()
                );
                let parked = windows
                    .into_iter()
                    .filter(|window| self.parked.contains_key(window))
                    .collect();
                self.restore_windows(parked);
            }
        }
    }
}
