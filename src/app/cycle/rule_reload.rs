use super::*;

pub(super) struct RuleReloadContext<'a> {
    pub(super) outputs: &'a macos::DisplayOutputs,
    pub(super) visible: &'a [ResolvedWorkspace],
    pub(super) observation: &'a mut crate::NativeObservation,
    pub(super) frontmost: Option<&'a AppInstanceId>,
    pub(super) focused: Option<&'a crate::PhysicalWindowId>,
    pub(super) cancelled: &'a AtomicBool,
    pub(super) failed: &'a mut usize,
}

impl<B: WorkspaceBackend> WorkspaceRuntime<B> {
    /// Applies rule changes from a reloaded config to already-open windows
    /// (both visible and parked on hidden workspaces) without disturbing
    /// windows whose effective rule disposition did not change.
    pub(super) fn apply_rule_reload(
        &mut self,
        previous: &Config,
        context: RuleReloadContext<'_>,
    ) -> Result<(), String> {
        let RuleReloadContext {
            outputs,
            visible,
            observation,
            frontmost,
            focused,
            cancelled,
            failed,
        } = context;
        let visible_set: HashSet<WorkspaceId> =
            visible.iter().map(|resolved| resolved.workspace).collect();

        // Index currently observed windows and keep bundle IDs fresh.
        let mut observed_windows: HashMap<
            crate::PhysicalWindowId,
            (String, crate::ObservedWindow),
        > = HashMap::new();
        let mut unowned_observed: Vec<(String, crate::ObservedWindow)> = Vec::new();
        for application in &observation.applications {
            let crate::AppObservation::Complete { app, windows, .. } = application else {
                continue;
            };
            self.memory
                .bundles
                .insert(app.id.clone(), app.bundle_id.clone());
            for window in windows {
                observed_windows.insert(window.id.clone(), (app.bundle_id.clone(), window.clone()));
                if self.workspaces.workspace_of_any(&window.id).is_none() {
                    unowned_observed.push((app.bundle_id.clone(), window.clone()));
                }
            }
        }

        // Collect all owned windows (tiled and floating) across every workspace.
        let mut owned: Vec<(crate::PhysicalWindowId, WorkspaceId, bool)> = Vec::new();
        for workspace in WorkspaceId::all() {
            for window in self.workspaces.engine(workspace).bound_windows() {
                owned.push((window, workspace, true));
            }
        }
        let mut floating: Vec<(crate::PhysicalWindowId, WorkspaceId)> = self
            .workspaces
            .floating_entries()
            .map(|(window, workspace)| (window.clone(), workspace))
            .collect();
        floating.sort_by_key(|(window, workspace)| (*workspace, window.app.pid, window.window_id));
        for (window, workspace) in floating {
            owned.push((window, workspace, false));
        }

        let mut relayout_workspaces: HashSet<WorkspaceId> = HashSet::new();
        let mut restored_apps: HashSet<AppInstanceId> = HashSet::new();

        for (window, source, was_tiled) in owned {
            let (bundle_id, probe_window, observed_resizable) =
                if let Some((bundle_id, observed)) = observed_windows.get(&window) {
                    (
                        bundle_id.clone(),
                        observed.clone(),
                        Some(observed.resizable),
                    )
                } else {
                    let Some(bundle_id) = self.resolve_bundle_id(&window.app) else {
                        continue;
                    };
                    let synthetic = crate::ObservedWindow {
                        id: window.clone(),
                        frame: crate::Rect::default(),
                        resizable: true,
                        title: None,
                    };
                    (bundle_id, synthetic, None)
                };

            let old_disp = previous.disposition(&bundle_id, &probe_window);
            let new_disp = self.config.disposition(&bundle_id, &probe_window);
            if old_disp == new_disp {
                continue;
            }

            match new_disp {
                WindowDisposition::Ignored => {
                    if self.parked.contains_key(&window)
                        && !matches!(
                            self.restore_window(&window),
                            macos::ApplyOutcome::Applied | macos::ApplyOutcome::Gone
                        )
                    {
                        *failed += 1;
                        continue;
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
                        self.forget_window(source, &window);
                        self.workspaces.own_floating(window, source);
                    }
                }
                WindowDisposition::Managed => {
                    if old_disp == WindowDisposition::Floating {
                        self.manual_floating.remove(&window);
                        if !was_tiled && visible_set.contains(&source) {
                            self.workspaces.forget_floating(&window);
                        }
                    }
                }
                WindowDisposition::Route(target) => {
                    self.ensure_managed_workspace(target)?;
                    if old_disp == WindowDisposition::Floating {
                        self.manual_floating.remove(&window);
                    }
                    let stay_floating = if was_tiled {
                        false
                    } else {
                        self.manual_floating.contains(&window)
                            || observed_resizable == Some(false)
                            || (observed_resizable.is_none()
                                && old_disp != WindowDisposition::Floating)
                    };
                    if source == target {
                        if !was_tiled && !stay_floating {
                            self.workspaces.forget_floating(&window);
                            if !visible_set.contains(&source) {
                                self.workspaces
                                    .engine_mut(target)
                                    .insert_bound_window(window);
                                self.exit_fullscreen_if_split(target);
                            }
                        }
                        continue;
                    }

                    let source_output = output_for(&self.workspaces, source, outputs);
                    let target_output = output_for(&self.workspaces, target, outputs);
                    let target_is_visible = visible_set.contains(&target);

                    if !target_is_visible {
                        if !self.parked.contains_key(&window) {
                            let parked = if source_output == target_output {
                                self.park_window(&window, target_output, outputs)
                            } else {
                                self.park_window_across(
                                    &window,
                                    source_output,
                                    target_output,
                                    outputs,
                                )
                            };
                            match parked {
                                macos::ApplyOutcome::Applied => {}
                                macos::ApplyOutcome::Gone => {
                                    self.forget_window(source, &window);
                                    continue;
                                }
                                _ => {
                                    *failed += 1;
                                    continue;
                                }
                            }
                            if focused == Some(&window) || frontmost == Some(&window.app) {
                                self.suppressed_focus = Some(window.clone());
                                self.suppressed_app = Some(window.app.clone());
                            }
                        } else if source_output != target_output
                            && let Some(original) = self.parked.get(&window).copied()
                            && source_output.contains_center(original)
                        {
                            let relocated = crate::app::move_workspace::translate_frame_to_output(
                                original,
                                source_output,
                                target_output,
                            );
                            if !self.record_recovery(&window, relocated) {
                                *failed += 1;
                                continue;
                            }
                        }
                        self.transfer_for_rule(&window, target, was_tiled, stay_floating);
                        self.clear_fullscreen_restore_if_inactive(source);
                        self.clear_relocation_evidence(source, target, &window);
                        log_event!(
                            "rule reload: moved {:?} workspace {} -> {}",
                            window,
                            source,
                            target
                        );
                    } else {
                        let was_parked = self.parked.contains_key(&window);
                        if was_parked {
                            match self.restore_window(&window) {
                                macos::ApplyOutcome::Applied => {}
                                macos::ApplyOutcome::Gone => {
                                    self.forget_window(source, &window);
                                    continue;
                                }
                                _ => {
                                    *failed += 1;
                                    continue;
                                }
                            }
                        }
                        self.transfer_for_rule(&window, target, was_tiled, stay_floating);
                        if stay_floating {
                            if source_output != target_output
                                && !self.carry_floating_window(
                                    &window,
                                    source_output,
                                    target_output,
                                )
                            {
                                *failed += 1;
                            }
                        } else {
                            relayout_workspaces.insert(target);
                        }
                        if was_parked {
                            restored_apps.insert(window.app.clone());
                        }
                        self.clear_fullscreen_restore_if_inactive(source);
                        self.clear_relocation_evidence(source, target, &window);
                        log_event!(
                            "rule reload: moved {:?} workspace {} -> {} (visible)",
                            window,
                            source,
                            target
                        );
                    }
                }
            }
        }

        // Unowned observed windows (for example, previously ignored) whose new
        // rule routes them to a workspace should all move, not only the first.
        for (bundle_id, window) in unowned_observed {
            if !window.resizable {
                continue;
            }
            let old_disp = previous.disposition(&bundle_id, &window);
            let new_disp = self.config.disposition(&bundle_id, &window);
            if old_disp == new_disp {
                continue;
            }
            let WindowDisposition::Route(target) = new_disp else {
                continue;
            };
            self.ensure_managed_workspace(target)?;
            let target_output = output_for(&self.workspaces, target, outputs);
            let target_is_visible = visible_set.contains(&target);
            if !target_is_visible {
                let source_output = visible
                    .iter()
                    .find(|resolved| resolved.output.contains_center(window.frame))
                    .map_or(target_output, |resolved| resolved.output);
                let parked = if source_output == target_output {
                    self.park_window(&window.id, target_output, outputs)
                } else {
                    self.park_window_across(&window.id, source_output, target_output, outputs)
                };
                if !matches!(parked, macos::ApplyOutcome::Applied) {
                    *failed += 1;
                    continue;
                }
                if focused == Some(&window.id) || frontmost == Some(&window.id.app) {
                    self.suppressed_focus = Some(window.id.clone());
                    self.suppressed_app = Some(window.id.app.clone());
                }
            } else {
                relayout_workspaces.insert(target);
            }
            self.workspaces
                .engine_mut(target)
                .insert_bound_window(window.id.clone());
            self.exit_fullscreen_if_split(target);
            log_event!(
                "rule reload: routed unowned {:?} -> workspace {}",
                window.id,
                target
            );
        }

        for target in relayout_workspaces {
            let target_output = output_for(&self.workspaces, target, outputs);
            let layout = self.layout_workspace(target, target_output, cancelled);
            *failed += layout.failed;
            let settings = self.layout_settings(target_output);
            super::update_observed_frames(
                observation,
                &self.workspaces.engine(target).placements(&settings),
            );
        }

        // A window restored from a hidden workspace onto a visible one was not
        // in the pre-restore snapshot. Mark its app unavailable for this single
        // cycle so reconciliation keeps its slot until the next snapshot sees it.
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

        Ok(())
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
