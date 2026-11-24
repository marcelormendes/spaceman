mod affinity;
mod omission;
mod readback;
mod rule_reload;
pub(super) use omission::Omission;
#[cfg(test)]
pub(super) use readback::ASYNC_BOUNCE_SETTLE;
pub(super) use readback::{APPLY_ATTEMPT_LIMIT, AppliedFrame, record_apply_attempt};
pub(super) use rule_reload::PendingRuleReload;

use affinity::{FrozenAffinityPolicy, FrozenWindowAffinity};

use super::*;
use crate::journal::PlacementSource;

/// How long a bound window may be missing from its app's Accessibility snapshot
/// while the window server still shows it on screen before the omission is
/// believed. Electron apps rebuild their accessibility tree under load (Slack
/// starting a huddle) and briefly report a partial window list; one such
/// snapshot must not retire a slot.
pub(super) const OMISSION_SETTLE: Duration = Duration::from_secs(2);

struct ReconcileDisplayContext<'a> {
    outputs: &'a macos::DisplayOutputs,
    observation: &'a mut crate::NativeObservation,
    affinity_policy: &'a FrozenAffinityPolicy,
    server_frames: Option<&'a HashMap<u32, crate::Rect>>,
    onscreen_frames: Option<&'a HashMap<u32, crate::Rect>>,
    cancelled: &'a AtomicBool,
    failed: &'a mut usize,
    close_recheck: &'a mut Option<Instant>,
}

impl<B: WorkspaceBackend> WorkspaceRuntime<B> {
    fn restore_visible(
        &mut self,
        visible: &[ResolvedWorkspace],
        cancelled: &AtomicBool,
        failed: &mut usize,
    ) {
        if self.parked.is_empty() {
            return;
        }
        let windows: Vec<_> = visible
            .iter()
            .flat_map(|resolved| self.workspaces.windows(resolved.workspace))
            .collect();
        for window in windows {
            if cancelled.load(Ordering::Acquire) {
                return;
            }
            if !self.parked.contains_key(&window) {
                continue;
            }
            // Throttle retries of restores awaiting window-server confirmation
            // so a slow app is not written to and logged about every cycle.
            if self
                .recently_parked
                .get(&window)
                .is_some_and(|attempted_at| attempted_at.elapsed() < RESTORE_RETRY_INTERVAL)
            {
                continue;
            }
            if !matches!(
                self.restore_window(&window),
                macos::ApplyOutcome::Applied | macos::ApplyOutcome::Gone
            ) {
                *failed += 1;
            }
        }
    }

    pub(super) fn cycle(
        &mut self,
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<CycleStatus, String> {
        let mut failed = 0;
        if cancelled.load(Ordering::Acquire) {
            return Err("workspace operation cancelled".into());
        }
        self.note_outputs(&outputs);
        // Parking is durable before workspace ownership. A crash between those
        // writes must not strand an unowned window behind an otherwise valid state.
        let unowned = self
            .parked
            .keys()
            .filter(|window| self.workspaces.workspace_of_any(window).is_none())
            .cloned()
            .collect::<Vec<_>>();
        let pending = self.restore_windows(unowned);
        if pending > 0 {
            return Err(format!(
                "Recovery pending for {pending} unowned parked window(s)"
            ));
        }
        self.synchronize_foreground(&outputs);
        self.follow_displays(&outputs);
        let visible = self.visible_workspaces(&outputs);
        let frontmost = self.backend.frontmost_application();
        let focused = self.backend.focused_window();
        let focused_is_frontmost = focused.as_ref().is_some_and(|focused| {
            frontmost
                .as_ref()
                .is_none_or(|frontmost| &focused.app == frontmost)
        });
        let focus_workspace = if focused_is_frontmost {
            focused
                .as_ref()
                .and_then(|focused| self.workspaces.workspace_of_any(focused))
        } else {
            frontmost
                .as_ref()
                .and_then(|app| self.workspaces.workspace_for_app(app))
        };
        // Suppression lasts until Accessibility reports a different focus. When
        // the focused window is missing or lags the frontmost app, the app is
        // the evidence; with neither reported nothing has changed.
        let suppressed = if focused_is_frontmost {
            focused.as_ref() == self.suppressed_focus.as_ref()
        } else if let Some(frontmost) = frontmost.as_ref() {
            self.suppressed_app.as_ref() == Some(frontmost)
        } else {
            true
        };
        if !suppressed {
            self.clear_suppressed_focus();
            if let Some(workspace) = focus_workspace {
                if !visible
                    .iter()
                    .any(|resolved| resolved.workspace == workspace)
                {
                    let input = self
                        .backend
                        .recent_input()
                        .map_or_else(|| "input unknown".into(), |input| input.to_string());
                    log_event!(
                        "focus follow: {:?} (frontmost {:?}) -> workspace {} ({})",
                        focused,
                        frontmost,
                        workspace,
                        input
                    );
                    return self.switch(workspace, outputs, cancelled);
                }
                // Focus moved to a workspace already on screen, such as a click
                // on another display. Option+Tab goes back from here.
                self.record_focused_workspace(workspace);
            }
        }
        for resolved in &visible {
            self.foreground_lanes
                .insert(resolved.display_id.clone(), resolved.selector.clone());
        }
        // One window-server snapshot serves every authoritative-frame check in
        // this cycle; per-window queries would each copy the full window list.
        let server_snapshot = self.backend.window_server_frames();
        let empty_server_frames = HashMap::new();
        let server_frames = server_snapshot.as_ref().unwrap_or(&empty_server_frames);
        self.recover_frontmost_stale_parked_window(
            &outputs,
            frontmost.as_ref(),
            focused.as_ref(),
            server_frames,
        );
        self.restore_visible(&visible, cancelled, &mut failed);
        self.park_inactive(&outputs, &visible, server_frames, cancelled, &mut failed);
        if cancelled.load(Ordering::Acquire) {
            return Err("workspace operation cancelled".into());
        }
        let mut observation = self
            .backend
            .snapshot(0, &outputs)
            .map_err(|error| format!("Observation error: {error}"))?;
        self.apply_pending_rule_reload(rule_reload::RuleReloadContext {
            outputs: &outputs,
            visible: &visible,
            observation: &mut observation,
            frontmost: frontmost.as_ref(),
            focused: focused.as_ref(),
            cancelled,
            failed: &mut failed,
        });
        let affinity_policy =
            self.freeze_window_affinities(&observation, &visible, server_frames)?;
        // Taken after the Accessibility snapshot so a window listed here was
        // on screen no earlier than the snapshot that may have omitted it.
        let onscreen_snapshot = self.backend.onscreen_window_frames();
        let mut close_recheck = None;
        for resolved in &visible {
            if !self.scope.manages_selector(&resolved.selector) {
                continue;
            }
            self.reconcile_display(
                resolved,
                ReconcileDisplayContext {
                    outputs: &outputs,
                    observation: &mut observation,
                    affinity_policy: &affinity_policy,
                    server_frames: server_snapshot.as_ref(),
                    onscreen_frames: onscreen_snapshot.as_ref(),
                    cancelled,
                    failed: &mut failed,
                    close_recheck: &mut close_recheck,
                },
            )?;
        }
        let mut attempted_focus_restore = HashSet::new();
        for resolved in &visible {
            if !self
                .pending_fullscreen_focus_restore
                .contains(&resolved.workspace)
            {
                continue;
            }
            attempted_focus_restore.insert(resolved.workspace);
            let Some(window) = self.workspaces.preferred_focus(resolved.workspace) else {
                self.pending_fullscreen_focus_restore
                    .remove(&resolved.workspace);
                continue;
            };
            let restored = if focus_workspace == Some(resolved.workspace) {
                self.backend.focus_window(&window)
            } else {
                self.backend.raise_window(&window)
            };
            self.pending_fullscreen_focus_restore
                .remove(&resolved.workspace);
            if !restored {
                failed += 1;
            }
        }
        if focused_is_frontmost
            && let Some(focused) = focused.as_ref()
            && let Some(workspace) = self.workspaces.workspace_of(focused)
        {
            self.workspaces
                .engine_mut(workspace)
                .remember_insertion_focus(focused);
        }
        if focused_is_frontmost
            && let Some(focused) = focused.as_ref()
            && let Some(workspace) = self.workspaces.workspace_of(focused)
            && self.workspaces.uses_fullscreen_layout(workspace)
            && !attempted_focus_restore.contains(&workspace)
            && visible
                .iter()
                .any(|resolved| resolved.workspace == workspace)
        {
            self.workspaces.remember_focus(workspace, focused);
        }
        self.record_window_memory(server_frames);
        // Creation can wake the worker before CG orders the window on screen.
        // Only a recent create/activation on the main thread enables retries.
        let missing_frontmost_window = frontmost.filter(|app| {
            let windows = &affinity_policy.windows;
            match focused.as_ref().filter(|window| &window.app == app) {
                Some(window) => !windows.contains_key(window),
                None => !windows.keys().any(|window| &window.app == app),
            }
        });
        self.journal.observation = Some(observation);
        self.persist_workspace_state_if_changed()?;
        let mut status = self.status_for_visible(failed, &visible, &outputs);
        status.close_recheck = close_recheck;
        status.missing_frontmost_window = missing_frontmost_window;
        Ok(status)
    }

    /// A bound window the window server still shows on screen inside this
    /// output, but which the app's Accessibility snapshot omitted, has not
    /// gone anywhere: the snapshot is incomplete for that app. Downgrade the
    /// app to `Unavailable` so reconciliation keeps its slots instead of
    /// retiring one and re-adopting the window as new a cycle later. The
    /// omission is believed once it outlasts [`OMISSION_SETTLE`].
    fn guard_omitted_windows(
        &mut self,
        workspace: WorkspaceId,
        output: crate::Rect,
        observation: &mut crate::NativeObservation,
        onscreen_frames: Option<&HashMap<u32, crate::Rect>>,
    ) {
        let Some(onscreen_frames) = onscreen_frames else {
            return;
        };
        let bound = self.workspaces.engine(workspace).bound_windows();
        if bound.is_empty() && self.omitted_windows.is_empty() {
            return;
        }
        let observed: HashSet<_> = live::observed_frames(observation).into_keys().collect();
        self.omitted_windows
            .retain(|window, _| match self.workspaces.workspace_of(window) {
                Some(owner) => owner != workspace || !observed.contains(window),
                None => false,
            });
        let now = Instant::now();
        for app_observation in &mut observation.applications {
            let crate::AppObservation::Complete { app, .. } = app_observation else {
                continue;
            };
            let settling = bound
                .iter()
                .filter(|window| window.app == app.id && !observed.contains(*window))
                .filter(|window| {
                    onscreen_frames
                        .get(&window.window_id)
                        .is_some_and(|frame| output.contains_center(*frame))
                })
                .filter(|window| {
                    let omission = self
                        .omitted_windows
                        .entry((*window).clone())
                        .or_insert_with(|| {
                            log_event!(
                                "observation omitted {:?}: window server shows it on screen; keeping its slot",
                                window
                            );
                            Omission::new(now)
                        });
                    omission.is_settling(now)
                })
                .count();
            if settling == 0 {
                continue;
            }
            *app_observation = crate::AppObservation::Unavailable {
                app: app.id.clone(),
                error: format!(
                    "accessibility snapshot omitted {settling} on-screen bound window(s)"
                ),
            };
        }
    }

    /// Apps the snapshot omitted entirely keep their slots while the window
    /// server still lists one of their bound windows. The window server also
    /// lists windows that are ordered out (a closed window its app keeps
    /// alive, a minimized window), and keeping those slots leaves an empty
    /// tile. So while another window of this workspace is observed, meaning
    /// the display is not showing another Space, an ordered-out window keeps
    /// its app's slots only until the omission outlasts [`OMISSION_SETTLE`].
    /// A fresh empty AXWindows list can confirm a last-window close earlier.
    /// Unresolved replacement candidates keep the plain window-server rule.
    fn apps_with_window_server_evidence(
        &mut self,
        workspace: WorkspaceId,
        observation: &crate::NativeObservation,
        server_frames: Option<&HashMap<u32, crate::Rect>>,
        onscreen_frames: Option<&HashMap<u32, crate::Rect>>,
        close_recheck: &mut Option<Instant>,
    ) -> HashSet<AppInstanceId> {
        let mut apps = HashSet::new();
        let Some(server_frames) = server_frames else {
            return apps;
        };
        let present: HashSet<_> = observation
            .applications
            .iter()
            .map(|app| app.app_id().clone())
            .collect();
        let observed = live::observed_frames(observation);
        let mut observed_tiles = observed
            .keys()
            .filter(|window| self.workspaces.workspace_of(window) == Some(workspace))
            .peekable();
        let onscreen_frames = onscreen_frames.filter(|_| observed_tiles.peek().is_some());
        // The newer CG snapshot must still show a sibling. A Space switch
        // between the AX and CG reads can order out the entire workspace.
        let sibling_on_screen = onscreen_frames
            .is_some_and(|frames| observed_tiles.any(|tile| frames.contains_key(&tile.window_id)));
        let windows: Vec<_> = self
            .workspaces
            .engine(workspace)
            .model()
            .workspace
            .slots
            .iter()
            .flat_map(|slot| match &slot.binding {
                BindingState::Bound(window) => vec![(window.clone(), true)],
                BindingState::Unresolved {
                    previous,
                    candidates,
                    ..
                } => std::iter::once((previous.clone(), true))
                    .chain(candidates.iter().map(|window| (window.clone(), false)))
                    .collect(),
            })
            .filter(|(window, _)| {
                !present.contains(&window.app) && server_frames.contains_key(&window.window_id)
            })
            .collect();
        let now = Instant::now();
        for (window, settles) in windows {
            let Some(onscreen_frames) = onscreen_frames.filter(|_| settles) else {
                // Off-screen time spent on another Space must not count
                // toward the settle once the display shows this workspace.
                self.omitted_windows.remove(&window);
                apps.insert(window.app);
                continue;
            };
            if onscreen_frames.contains_key(&window.window_id) {
                self.omitted_windows.remove(&window);
                apps.insert(window.app);
                continue;
            }
            if self.ordered_out_window_is_settling(
                workspace,
                &window,
                sibling_on_screen && !observation.mouse_down,
                now,
                close_recheck,
            ) {
                apps.insert(window.app);
            }
        }
        apps
    }

    /// Bookkeeping shared by every path that moves a window between workspaces.
    pub(super) fn clear_relocation_evidence(
        &mut self,
        source: WorkspaceId,
        target: WorkspaceId,
        window: &crate::PhysicalWindowId,
    ) {
        self.constrained[source.index()].remove(window);
        self.constrained[target.index()].remove(window);
        self.applied[source.index()].remove(window);
        self.applied[target.index()].remove(window);
        self.apply_attempts[source.index()].remove(window);
        self.apply_attempts[target.index()].remove(window);
    }

    fn reconcile_display(
        &mut self,
        resolved: &ResolvedWorkspace,
        context: ReconcileDisplayContext<'_>,
    ) -> Result<(), String> {
        let ReconcileDisplayContext {
            outputs,
            observation,
            affinity_policy,
            server_frames,
            onscreen_frames,
            cancelled,
            failed,
            close_recheck,
        } = context;
        let workspace = resolved.workspace;
        let output = resolved.output;
        let ticket = {
            let engine = self.workspaces.engine_mut(workspace);
            engine.invalidate();
            engine.begin_cycle()
        };
        let mut scoped_observation = observation.clone();
        scoped_observation.cycle = ticket.cycle;
        let settings = self.layout_settings(output);
        for app in &mut scoped_observation.applications {
            if let crate::AppObservation::Complete {
                app: _,
                focused,
                windows,
            } = app
            {
                windows.retain(|window| {
                    let affinity = affinity_policy
                        .windows
                        .get(&window.id)
                        .copied()
                        .unwrap_or(FrozenWindowAffinity::CurrentDisplay);
                    match affinity {
                        FrozenWindowAffinity::Owned(owner)
                        | FrozenWindowAffinity::PendingReplacement(owner) => owner == workspace,
                        FrozenWindowAffinity::AmbiguousReplacement
                        | FrozenWindowAffinity::Floating
                        | FrozenWindowAffinity::Ignored => false,
                        FrozenWindowAffinity::Inferred(target)
                            if affinity_policy.visible_workspaces.contains(&target) =>
                        {
                            target == workspace
                        }
                        FrozenWindowAffinity::ExplicitRoute(_)
                        | FrozenWindowAffinity::Inferred(_)
                        | FrozenWindowAffinity::CurrentDisplay => {
                            output.contains_center(window.frame)
                                && self
                                    .workspaces
                                    .workspace_of(&window.id)
                                    .is_none_or(|owner| owner == workspace)
                        }
                    }
                });
                if focused
                    .as_ref()
                    .is_some_and(|focused| !windows.iter().any(|window| &window.id == focused))
                {
                    *focused = None;
                }
            }
        }
        self.guard_omitted_windows(workspace, output, &mut scoped_observation, onscreen_frames);
        let apps_with_window_server_evidence = self.apps_with_window_server_evidence(
            workspace,
            &scoped_observation,
            server_frames,
            onscreen_frames,
            close_recheck,
        );
        live::add_absent_applications_with(
            &mut scoped_observation,
            &mut self.known_apps[workspace.index()],
            |app| self.backend.is_process_running(app),
            |app| server_frames.is_none() || apps_with_window_server_evidence.contains(app),
        );
        let reconciliation = self
            .workspaces
            .engine_mut(workspace)
            .accept(ticket, &scoped_observation, &settings)
            .map_err(|error| format!("stale workspace cycle: {error:?}"))?;
        self.log_binding_decisions(workspace, &reconciliation.decisions, affinity_policy);
        let routes: Vec<_> = reconciliation
            .decisions
            .iter()
            .filter_map(|decision| match decision {
                Decision::WindowInserted { window, .. } => affinity_policy
                    .windows
                    .get(window)
                    .copied()
                    .and_then(|affinity| match affinity {
                        FrozenWindowAffinity::ExplicitRoute(target)
                        | FrozenWindowAffinity::Inferred(target)
                            if target != workspace =>
                        {
                            Some((window.clone(), target))
                        }
                        _ => None,
                    }),
                _ => None,
            })
            .collect();
        let mut routed_targets = HashSet::new();
        for (window, target) in &routes {
            if !affinity_policy.visible_workspaces.contains(target) {
                let target_output = output_for(&self.workspaces, *target, outputs);
                match self.park_window(window, target_output, outputs) {
                    macos::ApplyOutcome::Applied => {}
                    macos::ApplyOutcome::Gone => {
                        self.forget_window(workspace, window);
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
        for decision in &reconciliation.decisions {
            let Decision::WindowInserted { window, .. } = decision else {
                continue;
            };
            let source = match affinity_policy.windows.get(window) {
                Some(FrozenWindowAffinity::ExplicitRoute(_)) => PlacementSource::Rule,
                Some(FrozenWindowAffinity::Inferred(_)) => PlacementSource::Recall,
                Some(FrozenWindowAffinity::CurrentDisplay) | None => PlacementSource::Active,
                Some(_) => continue,
            };
            if let Some(owner) = self.workspaces.workspace_of(window) {
                self.note_placement(window, source, owner);
            }
        }
        let inserted_here = reconciliation.decisions.iter().any(|decision| {
            matches!(
                decision,
                Decision::WindowInserted { window, .. }
                    if !routes.iter().any(|(routed, _)| routed == window)
            )
        });
        let bound: HashSet<_> = self
            .workspaces
            .engine(workspace)
            .bound_windows()
            .into_iter()
            .collect();
        if inserted_here {
            self.exit_fullscreen_if_split(workspace);
        }
        for target in &routed_targets {
            self.exit_fullscreen_if_split(*target);
        }
        self.workspaces.prune_intents(workspace);
        self.clear_fullscreen_restore_if_inactive(workspace);
        for target in &routed_targets {
            self.clear_fullscreen_restore_if_inactive(*target);
        }
        self.constrained[workspace.index()].retain(|window, _| bound.contains(window));
        self.applied[workspace.index()].retain(|window, _| bound.contains(window));
        self.apply_attempts[workspace.index()].retain(|window, _| bound.contains(window));
        let workspaces = &self.workspaces;
        self.minimum_sizes
            .retain(|window, _| workspaces.workspace_of(window).is_some());
        let observed_frames = live::observed_frames(&scoped_observation);
        let filter_unchanged = routes.is_empty();
        let engine = self.workspaces.engine_mut(workspace);
        engine.fit_minimums(&settings, &self.minimum_sizes);
        let mut placements = engine.placements(&settings);
        for placement in &placements {
            engine.record_placement_target(placement.slot, placement.frame);
        }
        self.apply_fullscreen_layout(workspace, &mut placements, &settings);
        if !scoped_observation.mouse_down {
            for placement in &placements {
                let Some(window) = placement.window.as_ref() else {
                    continue;
                };
                let Some(&observed) = observed_frames.get(window) else {
                    continue;
                };
                let target = placement.frame;
                if observed.approximately_equals(target, settings.frame_tolerance) {
                    let settling = self.applied[workspace.index()]
                        .get(window)
                        .is_some_and(|applied| applied.is_settling(target));
                    if !settling {
                        self.applied[workspace.index()].remove(window);
                        self.constrained[workspace.index()].remove(window);
                        self.apply_attempts[workspace.index()].remove(window);
                    }
                    continue;
                }
                if self.constrained[workspace.index()]
                    .get(window)
                    .is_some_and(|(constrained_target, _)| *constrained_target != target)
                {
                    self.constrained[workspace.index()].remove(window);
                }
                if self.apply_attempts[workspace.index()]
                    .get(window)
                    .is_some_and(|(attempted_target, _)| *attempted_target != target)
                {
                    self.apply_attempts[workspace.index()].remove(window);
                }
                if self.applied[workspace.index()]
                    .remove(window)
                    .is_some_and(|applied| applied.target == target)
                {
                    self.constrained[workspace.index()].insert(window.clone(), (target, observed));
                    record_apply_attempt(
                        &mut self.apply_attempts[workspace.index()],
                        workspace,
                        window,
                        target,
                    );
                }
                if self.apply_attempts[workspace.index()]
                    .get(window)
                    .is_some_and(|(attempted_target, count)| {
                        *attempted_target == target && *count >= APPLY_ATTEMPT_LIMIT
                    })
                {
                    self.constrained[workspace.index()]
                        .entry(window.clone())
                        .or_insert((target, observed));
                }
            }
        }
        if filter_unchanged {
            placements.retain(|placement| {
                live::needs_application(placement, &observed_frames, settings.frame_tolerance)
            });
        }
        placements.retain(|placement| {
            let Some(window) = placement.window.as_ref() else {
                return false;
            };
            if self.apply_attempts[workspace.index()]
                .get(window)
                .is_some_and(|(target, count)| {
                    *target == placement.frame && *count >= APPLY_ATTEMPT_LIMIT
                })
            {
                return false;
            }
            !self.constrained[workspace.index()]
                .get(window)
                .zip(observed_frames.get(window))
                .is_some_and(|((target, accepted), observed)| {
                    *target == placement.frame
                        && observed.approximately_equals(*accepted, settings.frame_tolerance)
                })
        });
        if !placements.is_empty() {
            let results = self
                .backend
                .apply_observed(&placements, &settings, cancelled);
            self.record_results(workspace, &results);
            let initially_failed = failed_results(&results);
            *failed += self
                .refit_minimum_sizes(workspace, output, results, cancelled)
                .map_or(initially_failed, |status| status.failed);
        }
        for target in routed_targets {
            if affinity_policy.visible_workspaces.contains(&target) {
                let target_output = output_for(&self.workspaces, target, outputs);
                let layout = self.layout_workspace(target, target_output, cancelled);
                *failed += layout.failed;
                let settings = self.layout_settings(target_output);
                update_observed_frames(
                    observation,
                    &self.workspaces.engine(target).placements(&settings),
                );
            }
        }
        Ok(())
    }

    /// Slot-identity changes are the events that explain "my window moved"; the
    /// steady-state decisions (unavailable, deferred) would only add noise.
    fn log_binding_decisions(
        &self,
        workspace: WorkspaceId,
        decisions: &[Decision],
        affinity_policy: &FrozenAffinityPolicy,
    ) {
        for decision in decisions {
            match decision {
                Decision::WindowInserted { slot, window } => {
                    let bundle = self.bundle_id(&window.app).unwrap_or("unknown");
                    let affinity = affinity_policy.windows.get(window);
                    log_event!(
                        "workspace {workspace}: bound {window:?} ({bundle}, {affinity:?}) to slot {slot:?}"
                    );
                }
                Decision::WindowRemoved { slot, window } => {
                    let bundle = self.bundle_id(&window.app).unwrap_or("unknown");
                    log_event!(
                        "workspace {workspace}: retired slot {slot:?} ({window:?} / {bundle} absent)"
                    );
                }
                Decision::NativeTabReplaced {
                    slot,
                    previous,
                    current,
                } => {
                    log_event!("workspace {workspace}: slot {slot:?} {previous:?} -> {current:?}");
                }
                Decision::ApplicationTerminated { app } => {
                    log_event!("workspace {workspace}: {app:?} terminated");
                }
                Decision::NativeTabAmbiguous { .. }
                | Decision::ObservationUnavailable { .. }
                | Decision::MouseDownDeferred => {}
            }
        }
    }
}

fn update_observed_frames(
    observation: &mut crate::NativeObservation,
    placements: &[crate::Placement],
) {
    let frames: HashMap<_, _> = placements
        .iter()
        .filter_map(|placement| {
            placement
                .window
                .as_ref()
                .map(|window| (window, placement.frame))
        })
        .collect();
    for application in &mut observation.applications {
        if let crate::AppObservation::Complete { windows, .. } = application {
            for window in windows {
                if let Some(frame) = frames.get(&window.id) {
                    window.frame = *frame;
                }
            }
        }
    }
}
