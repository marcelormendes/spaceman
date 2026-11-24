use super::*;

/// Bound retries for transient park/restore/observation failures so a
/// permanently broken window never loops forever.
const MAX_RELOAD_ATTEMPTS: u8 = 3;

pub(in crate::app) struct PendingRuleReload {
    /// Baseline config for windows not in `deferred`.
    previous: Config,
    /// Earlier baseline per window that failed or deferred a prior reload pass,
    /// so a newer reload advances `previous` for completed windows without
    /// losing the unfinished windows' original baseline.
    deferred: HashMap<crate::PhysicalWindowId, Config>,
    attempts: u8,
    /// When true, only windows in `deferred` are visited (pure retry cycle).
    retry_only: bool,
}

impl PendingRuleReload {
    pub(in crate::app) fn new(previous: Config) -> Self {
        Self {
            previous,
            deferred: HashMap::new(),
            attempts: 0,
            retry_only: false,
        }
    }

    pub(in crate::app) fn advance_for_new_reload(&mut self, applied: Config) {
        if self.retry_only {
            // Earlier cycle already applied `applied` to every non-deferred window.
            self.previous = applied;
        }
        self.attempts = 0;
        self.retry_only = false;
    }

    fn includes(&self, window: &crate::PhysicalWindowId) -> bool {
        !self.retry_only || self.deferred.contains_key(window)
    }

    fn baseline_for(&self, window: &crate::PhysicalWindowId) -> &Config {
        self.deferred.get(window).unwrap_or(&self.previous)
    }
}

pub(super) struct RuleReloadContext<'a> {
    pub(super) outputs: &'a macos::DisplayOutputs,
    pub(super) visible: &'a [ResolvedWorkspace],
    pub(super) observation: &'a mut crate::NativeObservation,
    pub(super) frontmost: Option<&'a AppInstanceId>,
    pub(super) focused: Option<&'a crate::PhysicalWindowId>,
    pub(super) cancelled: &'a AtomicBool,
    pub(super) failed: &'a mut usize,
}

struct ReloadPass<'a> {
    outputs: &'a macos::DisplayOutputs,
    visible_list: &'a [ResolvedWorkspace],
    visible: HashSet<WorkspaceId>,
    frontmost: Option<&'a AppInstanceId>,
    focused: Option<&'a crate::PhysicalWindowId>,
    failed: &'a mut usize,
    relayout: HashSet<WorkspaceId>,
    restored_apps: HashSet<AppInstanceId>,
    retry_windows: HashSet<crate::PhysicalWindowId>,
}

struct ObservedIndex {
    windows: HashMap<crate::PhysicalWindowId, (String, crate::ObservedWindow)>,
    unowned: Vec<(String, crate::ObservedWindow)>,
    unavailable_apps: HashSet<AppInstanceId>,
}

struct OwnedWindow {
    window: crate::PhysicalWindowId,
    source: WorkspaceId,
    was_tiled: bool,
}

impl<B: WorkspaceBackend> WorkspaceRuntime<B> {
    /// Applies pending config-reload rule changes to open windows (visible and
    /// hidden), retrying only windows that hit transient failures or incomplete
    /// observations.
    pub(super) fn apply_pending_rule_reload(&mut self, context: RuleReloadContext<'_>) {
        let Some(mut pending) = self.pending_rule_reload.take() else {
            return;
        };
        let RuleReloadContext {
            outputs,
            visible,
            observation,
            frontmost,
            focused,
            cancelled,
            failed,
        } = context;

        let index = self.index_observed_for_reload(observation);
        let owned = self.collect_owned_windows();
        let mut pass = ReloadPass {
            outputs,
            visible_list: visible,
            visible: visible.iter().map(|resolved| resolved.workspace).collect(),
            frontmost,
            focused,
            failed,
            relayout: HashSet::new(),
            restored_apps: HashSet::new(),
            retry_windows: HashSet::new(),
        };

        for entry in owned {
            if !pending.includes(&entry.window) {
                continue;
            }
            if index.unavailable_apps.contains(&entry.window.app) {
                pass.retry_windows.insert(entry.window);
                continue;
            }
            let baseline = pending.baseline_for(&entry.window);
            self.apply_owned_rule_change(baseline, entry, &index, &mut pass);
        }

        self.route_unowned_windows(&pending, index.unowned, &mut pass);

        let ReloadPass {
            relayout,
            restored_apps,
            retry_windows,
            failed,
            ..
        } = pass;

        for target in relayout {
            let target_output = output_for(&self.workspaces, target, outputs);
            let layout = self.layout_workspace(target, target_output, cancelled);
            *failed += layout.failed;
            let settings = self.layout_settings(target_output);
            super::update_observed_frames(
                observation,
                &self.workspaces.engine(target).placements(&settings),
            );
        }
        mark_restored_apps_unavailable(observation, restored_apps);

        if !retry_windows.is_empty() {
            if pending.attempts + 1 < MAX_RELOAD_ATTEMPTS {
                let deferred = retry_windows
                    .into_iter()
                    .map(|window| {
                        let baseline = pending.baseline_for(&window).clone();
                        (window, baseline)
                    })
                    .collect();
                pending.attempts += 1;
                pending.deferred = deferred;
                pending.retry_only = true;
                self.pending_rule_reload = Some(pending);
            } else {
                let mut left: Vec<_> = retry_windows.into_iter().collect();
                left.sort_by_key(|window| (window.app.pid, window.window_id));
                log_event!(
                    "rule reload: gave up after {} attempt(s); {} window(s) unchanged: {:?}",
                    MAX_RELOAD_ATTEMPTS,
                    left.len(),
                    left
                );
            }
        }
    }

    fn index_observed_for_reload(
        &mut self,
        observation: &crate::NativeObservation,
    ) -> ObservedIndex {
        let mut index = ObservedIndex {
            windows: HashMap::new(),
            unowned: Vec::new(),
            unavailable_apps: HashSet::new(),
        };
        for application in &observation.applications {
            match application {
                crate::AppObservation::Complete { app, windows, .. } => {
                    self.memory
                        .bundles
                        .insert(app.id.clone(), app.bundle_id.clone());
                    for window in windows {
                        index
                            .windows
                            .insert(window.id.clone(), (app.bundle_id.clone(), window.clone()));
                        if self.workspaces.workspace_of_any(&window.id).is_none()
                            && !self.scratchpad.contains(&window.id)
                        {
                            index.unowned.push((app.bundle_id.clone(), window.clone()));
                        }
                    }
                }
                crate::AppObservation::Unavailable { app, .. } => {
                    index.unavailable_apps.insert(app.clone());
                }
                crate::AppObservation::Terminated { .. } => {}
            }
        }
        index
    }

    fn collect_owned_windows(&self) -> Vec<OwnedWindow> {
        let mut owned = Vec::new();
        for workspace in WorkspaceId::all() {
            for window in self.workspaces.engine(workspace).bound_windows() {
                owned.push(OwnedWindow {
                    window,
                    source: workspace,
                    was_tiled: true,
                });
            }
        }
        let mut floating: Vec<_> = self
            .workspaces
            .floating_entries()
            .map(|(window, workspace)| (window.clone(), workspace))
            .collect();
        floating.sort_by_key(|(window, workspace)| (*workspace, window.app.pid, window.window_id));
        for (window, source) in floating {
            if self.scratchpad.contains(&window) {
                continue;
            }
            owned.push(OwnedWindow {
                window,
                source,
                was_tiled: false,
            });
        }
        owned
    }

    fn apply_owned_rule_change(
        &mut self,
        previous: &Config,
        entry: OwnedWindow,
        index: &ObservedIndex,
        pass: &mut ReloadPass<'_>,
    ) {
        let OwnedWindow {
            window,
            source,
            was_tiled,
        } = entry;
        let Some((bundle_id, probe_window, observed_resizable)) =
            self.probe_owned_window(&window, previous, index)
        else {
            return;
        };
        let old_disp = previous.disposition(&bundle_id, &probe_window);
        let new_disp = self.config.disposition(&bundle_id, &probe_window);
        if old_disp == new_disp {
            return;
        }

        match new_disp {
            WindowDisposition::Ignored => {
                if self.parked.contains_key(&window)
                    && !matches!(
                        self.restore_window(&window),
                        macos::ApplyOutcome::Applied | macos::ApplyOutcome::Gone
                    )
                {
                    *pass.failed += 1;
                    pass.retry_windows.insert(window);
                    return;
                }
                log_event!(
                    "rule reload: release {:?} from workspace {}: ignored",
                    window,
                    source
                );
                self.forget_window(source, &window);
            }
            WindowDisposition::Floating => {
                if was_tiled {
                    log_event!("rule reload: float {:?} on workspace {}", window, source);
                    // Detach from the tiled layout while preserving parking
                    // recovery state for hidden windows.
                    self.workspaces.forget_window(source, &window);
                    self.clear_fullscreen_restore_if_inactive(source);
                    self.clear_relocation_evidence(source, source, &window);
                    self.workspaces.own_floating(window, source);
                }
            }
            WindowDisposition::Managed => {
                if old_disp == WindowDisposition::Floating {
                    self.manual_floating.remove(&window);
                    if !was_tiled && observed_resizable != Some(false) {
                        self.unfloat_on_workspace(&window, source, &pass.visible);
                    }
                }
            }
            WindowDisposition::Route(target) => {
                if let Err(error) = self.ensure_managed_workspace(target) {
                    log_event!("rule reload: skipped {:?}: {error}", window);
                    *pass.failed += 1;
                    pass.retry_windows.insert(window);
                    return;
                }
                if old_disp == WindowDisposition::Floating {
                    self.manual_floating.remove(&window);
                }
                let stay_floating = !was_tiled
                    && (observed_resizable == Some(false)
                        || self.manual_floating.contains(&window)
                        || (observed_resizable.is_none()
                            && old_disp != WindowDisposition::Floating));
                if source == target {
                    if !was_tiled && !stay_floating {
                        self.unfloat_on_workspace(&window, source, &pass.visible);
                    }
                    return;
                }
                if pass.visible.contains(&target) {
                    self.relocate_to_visible(
                        &window,
                        source,
                        target,
                        was_tiled,
                        stay_floating,
                        pass,
                    );
                } else {
                    self.relocate_to_hidden(
                        &window,
                        source,
                        target,
                        was_tiled,
                        stay_floating,
                        pass,
                    );
                }
            }
        }
    }

    fn probe_owned_window(
        &mut self,
        window: &crate::PhysicalWindowId,
        previous: &Config,
        index: &ObservedIndex,
    ) -> Option<(String, crate::ObservedWindow, Option<bool>)> {
        if let Some((bundle_id, observed)) = index.windows.get(window) {
            return Some((
                bundle_id.clone(),
                observed.clone(),
                Some(observed.resizable),
            ));
        }
        let bundle_id = self.resolve_bundle_id(&window.app)?;
        // Unobserved (parked) windows have no title in memory; only evaluate
        // them when neither config has a title rule for this bundle.
        if previous.has_title_rule_for(&bundle_id) || self.config.has_title_rule_for(&bundle_id) {
            return None;
        }
        let synthetic = crate::ObservedWindow {
            id: window.clone(),
            frame: crate::Rect::default(),
            resizable: true,
            title: None,
        };
        Some((bundle_id, synthetic, None))
    }

    fn unfloat_on_workspace(
        &mut self,
        window: &crate::PhysicalWindowId,
        workspace: WorkspaceId,
        visible: &HashSet<WorkspaceId>,
    ) {
        self.workspaces.forget_floating(window);
        if !visible.contains(&workspace) {
            self.workspaces
                .engine_mut(workspace)
                .insert_bound_window(window.clone());
            self.exit_fullscreen_if_split(workspace);
        }
    }

    fn relocate_to_hidden(
        &mut self,
        window: &crate::PhysicalWindowId,
        source: WorkspaceId,
        target: WorkspaceId,
        was_tiled: bool,
        stay_floating: bool,
        pass: &mut ReloadPass<'_>,
    ) {
        let source_output = output_for(&self.workspaces, source, pass.outputs);
        let target_output = output_for(&self.workspaces, target, pass.outputs);
        let was_parked = self.parked.contains_key(window);
        if !was_parked {
            let outcome = if source_output == target_output {
                self.park_window(window, target_output, pass.outputs)
            } else {
                self.park_window_across(window, source_output, target_output, pass.outputs)
            };
            match outcome {
                macos::ApplyOutcome::Applied => {}
                macos::ApplyOutcome::Gone => {
                    self.forget_window(source, window);
                    return;
                }
                _ => {
                    *pass.failed += 1;
                    pass.retry_windows.insert(window.clone());
                    return;
                }
            }
            self.suppress_parked_focus(window, pass.focused, pass.frontmost);
        } else if source_output != target_output
            && let Some(original) = self.parked.get(window).copied()
            && !target_output.contains_center(original)
        {
            let from_output = pass
                .outputs
                .frames()
                .find(|display| display.contains_center(original))
                .unwrap_or(source_output);
            let relocated = crate::app::move_workspace::translate_frame_to_output(
                original,
                from_output,
                target_output,
            );
            if !self.record_recovery(window, relocated) {
                *pass.failed += 1;
                pass.retry_windows.insert(window.clone());
                return;
            }
        }
        self.transfer_for_rule(window, target, was_tiled, stay_floating);
        self.clear_fullscreen_restore_if_inactive(source);
        self.clear_relocation_evidence(source, target, window);
        log_event!(
            "rule reload: moved {:?} workspace {} -> {}",
            window,
            source,
            target
        );
    }

    fn relocate_to_visible(
        &mut self,
        window: &crate::PhysicalWindowId,
        source: WorkspaceId,
        target: WorkspaceId,
        was_tiled: bool,
        stay_floating: bool,
        pass: &mut ReloadPass<'_>,
    ) {
        let source_output = output_for(&self.workspaces, source, pass.outputs);
        let target_output = output_for(&self.workspaces, target, pass.outputs);
        let was_parked = self.parked.contains_key(window);
        if was_parked {
            let mut relocated_from = None;
            if source_output != target_output
                && let Some(original) = self.parked.get(window).copied()
                && !target_output.contains_center(original)
            {
                let from_output = pass
                    .outputs
                    .frames()
                    .find(|display| display.contains_center(original))
                    .unwrap_or(source_output);
                let relocated = crate::app::move_workspace::translate_frame_to_output(
                    original,
                    from_output,
                    target_output,
                );
                if !self.record_recovery(window, relocated) {
                    *pass.failed += 1;
                    pass.retry_windows.insert(window.clone());
                    return;
                }
                relocated_from = Some(original);
            }
            match self.restore_window(window) {
                macos::ApplyOutcome::Applied => {}
                macos::ApplyOutcome::Gone => {
                    self.forget_window(source, window);
                    return;
                }
                _ => {
                    if let Some(original) = relocated_from
                        && !self.record_recovery(window, original)
                    {
                        self.parked.insert(window.clone(), original);
                    }
                    *pass.failed += 1;
                    pass.retry_windows.insert(window.clone());
                    return;
                }
            }
        }
        if !was_parked
            && stay_floating
            && source_output != target_output
            && !self.carry_floating_window(window, source_output, target_output)
        {
            *pass.failed += 1;
            pass.retry_windows.insert(window.clone());
            return;
        }
        self.transfer_for_rule(window, target, was_tiled, stay_floating);
        if !stay_floating {
            pass.relayout.insert(target);
        }
        if was_parked {
            pass.restored_apps.insert(window.app.clone());
        }
        self.clear_fullscreen_restore_if_inactive(source);
        self.clear_relocation_evidence(source, target, window);
        log_event!(
            "rule reload: moved {:?} workspace {} -> {} (visible)",
            window,
            source,
            target
        );
    }

    /// Unowned observed windows (for example, previously ignored) whose new
    /// rule routes them to a workspace should all move, not only the first.
    fn route_unowned_windows(
        &mut self,
        pending: &PendingRuleReload,
        unowned: Vec<(String, crate::ObservedWindow)>,
        pass: &mut ReloadPass<'_>,
    ) {
        for (bundle_id, window) in unowned {
            if !pending.includes(&window.id) || self.scratchpad.contains(&window.id) {
                continue;
            }
            let old_disp = pending
                .baseline_for(&window.id)
                .disposition(&bundle_id, &window);
            let new_disp = self.config.disposition(&bundle_id, &window);
            if old_disp == new_disp {
                continue;
            }
            let WindowDisposition::Route(target) = new_disp else {
                continue;
            };
            if let Err(error) = self.ensure_managed_workspace(target) {
                log_event!("rule reload: skipped {:?}: {error}", window.id);
                *pass.failed += 1;
                pass.retry_windows.insert(window.id);
                continue;
            }
            let stay_floating = !window.resizable;
            let target_output = output_for(&self.workspaces, target, pass.outputs);
            let source_output = pass
                .visible_list
                .iter()
                .find(|resolved| resolved.output.contains_center(window.frame))
                .map_or(target_output, |resolved| resolved.output);
            if pass.visible.contains(&target) {
                if stay_floating {
                    if source_output != target_output
                        && !self.carry_floating_window(&window.id, source_output, target_output)
                    {
                        *pass.failed += 1;
                        pass.retry_windows.insert(window.id);
                        continue;
                    }
                    self.workspaces.own_floating(window.id.clone(), target);
                } else {
                    self.workspaces
                        .engine_mut(target)
                        .insert_bound_window(window.id.clone());
                    self.exit_fullscreen_if_split(target);
                    pass.relayout.insert(target);
                }
            } else {
                let parked = if source_output == target_output {
                    self.park_window(&window.id, target_output, pass.outputs)
                } else {
                    self.park_window_across(&window.id, source_output, target_output, pass.outputs)
                };
                if !matches!(parked, macos::ApplyOutcome::Applied) {
                    *pass.failed += 1;
                    pass.retry_windows.insert(window.id);
                    continue;
                }
                self.suppress_parked_focus(&window.id, pass.focused, pass.frontmost);
                if stay_floating {
                    self.workspaces.own_floating(window.id.clone(), target);
                } else {
                    self.workspaces
                        .engine_mut(target)
                        .insert_bound_window(window.id.clone());
                    self.exit_fullscreen_if_split(target);
                }
            }
            log_event!(
                "rule reload: routed unowned {:?} -> workspace {}",
                window.id,
                target
            );
        }
    }

    fn resolve_bundle_id(&mut self, app: &AppInstanceId) -> Option<String> {
        if let Some(bundle_id) = self.bundle_id(app) {
            return Some(bundle_id.to_owned());
        }
        let bundle_id = self.backend.bundle_id(app)?;
        self.memory.bundles.insert(app.clone(), bundle_id.clone());
        Some(bundle_id)
    }

    fn transfer_for_rule(
        &mut self,
        window: &crate::PhysicalWindowId,
        target: WorkspaceId,
        was_tiled: bool,
        stay_floating: bool,
    ) {
        if stay_floating {
            self.workspaces.own_floating(window.clone(), target);
        } else if was_tiled {
            let _ = self.workspaces.move_window(window, target);
            self.exit_fullscreen_if_split(target);
        } else {
            self.workspaces.forget_floating(window);
            self.workspaces
                .engine_mut(target)
                .insert_bound_window(window.clone());
            self.exit_fullscreen_if_split(target);
        }
    }
}

/// A window restored from a hidden workspace onto a visible one was not in the
/// pre-restore snapshot. Mark its app unavailable for this single cycle so
/// reconciliation keeps its slot until the next snapshot sees it.
fn mark_restored_apps_unavailable(
    observation: &mut crate::NativeObservation,
    restored_apps: HashSet<AppInstanceId>,
) {
    for app_id in restored_apps {
        let mut found = false;
        for application in &mut observation.applications {
            if application.app_id() == &app_id {
                *application = crate::AppObservation::Unavailable {
                    app: app_id.clone(),
                    error: "window restored during rule reload".into(),
                };
                found = true;
            }
        }
        if !found {
            observation
                .applications
                .push(crate::AppObservation::Unavailable {
                    app: app_id,
                    error: "window restored during rule reload".into(),
                });
        }
    }
}
