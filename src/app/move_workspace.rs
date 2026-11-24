use super::*;

struct MoveRollback<'a> {
    moved_ws: WorkspaceId,
    moved_relocated: bool,
    moved_floating_originals: &'a [(crate::PhysicalWindowId, crate::Rect)],
    parked_displaced: Vec<crate::PhysicalWindowId>,
    revealed_source: &'a [crate::PhysicalWindowId],
    source_incoming_originals: &'a HashMap<crate::PhysicalWindowId, crate::Rect>,
    source_output: crate::Rect,
    outputs: &'a macos::DisplayOutputs,
}

impl<B: WorkspaceBackend> WorkspaceRuntime<B> {
    pub(super) fn move_workspace_to_display(
        &mut self,
        target: DisplayTarget,
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<CycleStatus, String> {
        self.synchronize_foreground(&outputs);
        if outputs.displays().len() <= 1 {
            return Err("Only one display is connected".into());
        }
        let visible = self.visible_workspaces(&outputs);
        let (source_resolved, focused_window) =
            self.focused_visible_workspace(&visible, &outputs)?;
        let moved_ws = source_resolved.workspace;
        let source_display_id = source_resolved.display_id;
        let source_output = source_resolved.output;
        let old_selector = self.ensure_managed_workspace(moved_ws)?;
        // The moved workspace is the focused one, and it keeps focus on the
        // target display. Record it before assignments change so Option+Tab's
        // history names the workspace focused before it, never the workspace
        // this move displaces on the target display.
        self.record_focused_workspace(moved_ws);

        let ordered = outputs.ordered();
        let source_index = ordered
            .iter()
            .position(|display| display.id == source_display_id)
            .ok_or("Focused display is unavailable")?;
        let source_display_name = ordered[source_index].name.clone();
        let target_display = match &target {
            DisplayTarget::Next => ordered[(source_index + 1) % ordered.len()],
            DisplayTarget::Prev => ordered[(source_index + ordered.len() - 1) % ordered.len()],
            DisplayTarget::Selector(selector) => outputs
                .resolve_exact(selector)
                .ok_or_else(|| format!("Display {:?} is not connected", selector.as_str()))?,
        };
        if target_display.id == source_display_id {
            return Ok(self.status(0, &outputs));
        }
        let target_selector = outputs.selector_for(target_display);
        if !self.scope.manages_selector(&target_selector) {
            return Err(format!(
                "Display {} is outside the managed display scope",
                target_display.name
            ));
        }
        let target_display_id = target_display.id.clone();
        let target_display_name = target_display.name.clone();
        let target_output = target_display.frame;

        let displaced = visible
            .iter()
            .find(|resolved| resolved.display_id == target_display_id)
            .map(|resolved| resolved.workspace);
        let source_replacement =
            self.pick_source_replacement(moved_ws, &old_selector, &source_display_id, &outputs);

        let frontmost_before = self.backend.frontmost_application();
        let moved_windows_before = self.workspaces.windows(moved_ws);
        if let Some(focused) = focused_window
            .as_ref()
            .filter(|focused| moved_windows_before.contains(focused))
        {
            self.workspaces.remember_focus(moved_ws, focused);
        }

        let move_started = Instant::now();
        let source_incoming = source_replacement
            .map(|workspace| self.workspaces.windows(workspace))
            .unwrap_or_default();
        let source_incoming_originals: HashMap<_, _> = source_incoming
            .iter()
            .filter_map(|window| {
                self.parked
                    .get(window)
                    .copied()
                    .map(|frame| (window.clone(), frame))
            })
            .collect();
        let mut displaced_outgoing = displaced
            .map(|workspace| self.workspaces.windows(workspace))
            .unwrap_or_default();
        self.order_back_to_front(&mut displaced_outgoing, |window| window.window_id);
        let moved_floating: Vec<_> = self
            .workspaces
            .floating_entries()
            .filter(|(_, owner)| *owner == moved_ws)
            .map(|(window, _)| window.clone())
            .collect();
        log_event!(
            "move-workspace: workspace {} ({} -> {}, source replacement {:?}, displaced {:?})",
            moved_ws,
            source_display_name,
            target_display_name,
            source_replacement,
            displaced
        );

        // Phase 1: reveal and lay out the source display's replacement workspace
        // beneath moved_ws before moving anything off the source display.
        let mut restored_source = Vec::new();
        for window in &source_incoming {
            if cancelled.load(Ordering::Acquire) {
                self.rollback_move_workspace(MoveRollback {
                    moved_ws,
                    moved_relocated: false,
                    moved_floating_originals: &[],
                    parked_displaced: Vec::new(),
                    revealed_source: &restored_source,
                    source_incoming_originals: &source_incoming_originals,
                    source_output,
                    outputs: &outputs,
                });
                return Err("workspace move cancelled".into());
            }
            if !self.parked.contains_key(window) {
                continue;
            }
            restored_source.push(window.clone());
            match self.restore_window(window) {
                macos::ApplyOutcome::Applied => {}
                macos::ApplyOutcome::Gone => {
                    if let Some(replacement) = source_replacement {
                        self.forget_window(replacement, window);
                    }
                }
                outcome => {
                    log_event!(
                        "move-workspace rollback: could not restore {:?} ({:?})",
                        window,
                        outcome
                    );
                    self.rollback_move_workspace(MoveRollback {
                        moved_ws,
                        moved_relocated: false,
                        moved_floating_originals: &[],
                        parked_displaced: Vec::new(),
                        revealed_source: &restored_source,
                        source_incoming_originals: &source_incoming_originals,
                        source_output,
                        outputs: &outputs,
                    });
                    return Err(format!(
                        "Workspace {moved_ws} stayed on {source_display_name} because the replacement workspace could not restore all of its windows"
                    ));
                }
            }
        }
        if !self.wait_for_reveal(&source_incoming, cancelled) {
            self.rollback_move_workspace(MoveRollback {
                moved_ws,
                moved_relocated: false,
                moved_floating_originals: &[],
                parked_displaced: Vec::new(),
                revealed_source: &restored_source,
                source_incoming_originals: &source_incoming_originals,
                source_output,
                outputs: &outputs,
            });
            if cancelled.load(Ordering::Acquire) {
                return Err("workspace move cancelled".into());
            }
            return Err(format!(
                "Workspace {moved_ws} stayed on {source_display_name} because replacement windows did not finish restoring"
            ));
        }

        let mut failed = 0;
        if let Some(replacement) = source_replacement {
            let source_layout = self.layout_workspace(replacement, source_output, cancelled);
            if cancelled.load(Ordering::Acquire) || source_layout.blocked {
                self.rollback_move_workspace(MoveRollback {
                    moved_ws,
                    moved_relocated: false,
                    moved_floating_originals: &[],
                    parked_displaced: Vec::new(),
                    revealed_source: &source_incoming,
                    source_incoming_originals: &source_incoming_originals,
                    source_output,
                    outputs: &outputs,
                });
                if cancelled.load(Ordering::Acquire) {
                    return Err("workspace move cancelled".into());
                }
                return Err(format!(
                    "Workspace {moved_ws} stayed on {source_display_name} because replacement windows could not be laid out"
                ));
            }
            failed += source_layout.failed;
        }

        // Phase 2: lay out moved_ws on the target display (beneath any
        // displaced workspace) and relocate its on-screen floating windows.
        let moved_layout = self.layout_workspace(moved_ws, target_output, cancelled);
        if cancelled.load(Ordering::Acquire) || moved_layout.blocked {
            self.rollback_move_workspace(MoveRollback {
                moved_ws,
                moved_relocated: true,
                moved_floating_originals: &[],
                parked_displaced: Vec::new(),
                revealed_source: &source_incoming,
                source_incoming_originals: &source_incoming_originals,
                source_output,
                outputs: &outputs,
            });
            if cancelled.load(Ordering::Acquire) {
                return Err("workspace move cancelled".into());
            }
            return Err(format!(
                "Workspace {moved_ws} stayed on {source_display_name} because its windows could not be laid out on {target_display_name}"
            ));
        }
        failed += moved_layout.failed;

        let mut moved_floating_originals = Vec::new();
        for window in &moved_floating {
            if cancelled.load(Ordering::Acquire) {
                self.rollback_move_workspace(MoveRollback {
                    moved_ws,
                    moved_relocated: true,
                    moved_floating_originals: &moved_floating_originals,
                    parked_displaced: Vec::new(),
                    revealed_source: &source_incoming,
                    source_incoming_originals: &source_incoming_originals,
                    source_output,
                    outputs: &outputs,
                });
                return Err("workspace move cancelled".into());
            }
            let frame = match self.backend.frame(window) {
                Ok(frame) => self.backend.window_server_frame(window).unwrap_or(frame),
                Err(macos::WindowAccessError::Gone) => {
                    self.forget_window(moved_ws, window);
                    continue;
                }
                Err(macos::WindowAccessError::Unavailable) => {
                    self.rollback_move_workspace(MoveRollback {
                        moved_ws,
                        moved_relocated: true,
                        moved_floating_originals: &moved_floating_originals,
                        parked_displaced: Vec::new(),
                        revealed_source: &source_incoming,
                        source_incoming_originals: &source_incoming_originals,
                        source_output,
                        outputs: &outputs,
                    });
                    return Err(format!(
                        "Workspace {moved_ws} stayed on {source_display_name} because a floating window could not be moved"
                    ));
                }
            };
            if !source_output.contains_center(frame) {
                continue;
            }
            let target_frame = translate_frame_to_output(frame, source_output, target_output);
            moved_floating_originals.push((window.clone(), frame));
            match self.backend.set_position(window, target_frame) {
                macos::ApplyOutcome::Applied => {}
                macos::ApplyOutcome::Gone => {
                    moved_floating_originals.pop();
                    self.forget_window(moved_ws, window);
                }
                _ => {
                    self.rollback_move_workspace(MoveRollback {
                        moved_ws,
                        moved_relocated: true,
                        moved_floating_originals: &moved_floating_originals,
                        parked_displaced: Vec::new(),
                        revealed_source: &source_incoming,
                        source_incoming_originals: &source_incoming_originals,
                        source_output,
                        outputs: &outputs,
                    });
                    return Err(format!(
                        "Workspace {moved_ws} stayed on {source_display_name} because a floating window could not be moved"
                    ));
                }
            }
        }

        // Phase 3: park the displaced workspace's windows on the target display.
        let mut parked_displaced = Vec::new();
        for window in &displaced_outgoing {
            if cancelled.load(Ordering::Acquire) {
                self.rollback_move_workspace(MoveRollback {
                    moved_ws,
                    moved_relocated: true,
                    moved_floating_originals: &moved_floating_originals,
                    parked_displaced,
                    revealed_source: &source_incoming,
                    source_incoming_originals: &source_incoming_originals,
                    source_output,
                    outputs: &outputs,
                });
                return Err("workspace move cancelled".into());
            }
            match self.park_window(window, target_output, &outputs) {
                macos::ApplyOutcome::Applied => parked_displaced.push(window.clone()),
                macos::ApplyOutcome::Gone => {
                    if let Some(displaced_ws) = displaced {
                        self.forget_window(displaced_ws, window);
                    }
                }
                outcome => {
                    log_event!(
                        "move-workspace rollback: could not park {:?} ({:?})",
                        window,
                        outcome
                    );
                    self.rollback_move_workspace(MoveRollback {
                        moved_ws,
                        moved_relocated: true,
                        moved_floating_originals: &moved_floating_originals,
                        parked_displaced,
                        revealed_source: &source_incoming,
                        source_incoming_originals: &source_incoming_originals,
                        source_output,
                        outputs: &outputs,
                    });
                    return Err(format!(
                        "Workspace {moved_ws} stayed on {source_display_name} because a window could not be parked"
                    ));
                }
            }
        }

        // Phase 4: persist the assignment in config.toml before committing
        // in-memory state so a config save failure rolls back cleanly.
        if let Err(error) = self
            .backend
            .persist_workspace_monitor(moved_ws, &target_selector)
        {
            log_event!(
                "move-workspace rollback: could not persist config for workspace {}: {}",
                moved_ws,
                error
            );
            self.rollback_move_workspace(MoveRollback {
                moved_ws,
                moved_relocated: true,
                moved_floating_originals: &moved_floating_originals,
                parked_displaced,
                revealed_source: &source_incoming,
                source_incoming_originals: &source_incoming_originals,
                source_output,
                outputs: &outputs,
            });
            return Err(format!(
                "Workspace {moved_ws} stayed on {source_display_name} because config could not be saved: {error}"
            ));
        }

        self.config
            .set_workspace_monitor(moved_ws, target_selector.clone());
        self.workspaces
            .move_to_display(moved_ws, target_selector.clone(), source_replacement);
        self.foreground_lanes
            .insert(target_display_id, target_selector);
        if let Some(replacement) = source_replacement {
            self.foreground_lanes.insert(
                source_display_id,
                self.workspaces.assignment(replacement).clone(),
            );
            if self.workspaces.uses_fullscreen_layout(replacement)
                && let Some(front) = self.workspaces.preferred_focus(replacement)
            {
                self.backend.raise_window(&front);
            }
        } else {
            self.foreground_lanes.remove(&source_display_id);
        }

        let moved_windows = self.workspaces.windows(moved_ws);
        let outcome = self.focus_workspace_entry(
            moved_ws,
            &moved_windows,
            focused_window,
            frontmost_before,
            false,
        );
        self.follow_pointer_for_workspace(moved_ws, target_output, outcome, &outputs);
        self.persist_workspace_state_if_changed()?;
        log_event!(
            "move-workspace complete: workspace {} -> {} in {}ms",
            moved_ws,
            target_display_name,
            move_started.elapsed().as_millis()
        );
        let mut status = self.status(failed, &outputs);
        status.hud = Some(self.hud_for(moved_ws, UiHudKind::Workspace, displaced, &outputs));
        Ok(status)
    }

    fn focused_visible_workspace(
        &mut self,
        visible: &[ResolvedWorkspace],
        outputs: &macos::DisplayOutputs,
    ) -> Result<(ResolvedWorkspace, Option<crate::PhysicalWindowId>), String> {
        let focused = self.backend.focused_window();
        if let Some(window) = focused.as_ref() {
            if let Some(workspace) = self.workspaces.workspace_of_any(window)
                && let Some(resolved) = visible.iter().find(|entry| entry.workspace == workspace)
            {
                return Ok((resolved.clone(), Some(window.clone())));
            }
            if let Some(frame) = self
                .backend
                .window_server_frame(window)
                .or_else(|| self.backend.frame(window).ok())
                && let Some(resolved) = visible
                    .iter()
                    .find(|entry| entry.output.contains_center(frame))
            {
                return Ok((resolved.clone(), Some(window.clone())));
            }
        }
        visible
            .iter()
            .find(|entry| entry.display_id == outputs.main().id)
            .or_else(|| visible.first())
            .cloned()
            .map(|resolved| (resolved, focused))
            .ok_or_else(|| "No active workspace is available".into())
    }

    fn pick_source_replacement(
        &self,
        moved_ws: WorkspaceId,
        old_selector: &DisplaySelector,
        source_display_id: &str,
        outputs: &macos::DisplayOutputs,
    ) -> Option<WorkspaceId> {
        let eligible = |candidate: WorkspaceId| {
            candidate != moved_ws
                && self.workspaces.is_enabled(candidate)
                && self
                    .scope
                    .manages_selector(self.workspaces.assignment(candidate))
                && outputs.resolve(self.workspaces.assignment(candidate)).id == source_display_id
        };
        if let Some(previous) = self.workspaces.previous_for(old_selector)
            && eligible(previous)
        {
            return Some(previous);
        }
        if let Some(source_display) = outputs
            .displays()
            .iter()
            .find(|display| display.id == source_display_id)
        {
            let source_selector = outputs.selector_for(source_display);
            if &source_selector != old_selector
                && let Some(active) = self.workspaces.active_for(&source_selector)
                && eligible(active)
            {
                return Some(active);
            }
        }
        WorkspaceId::all()
            .find(|&candidate| {
                eligible(candidate) && self.workspaces.assignment(candidate) == old_selector
            })
            .or_else(|| WorkspaceId::all().find(|&candidate| eligible(candidate)))
    }

    fn rollback_move_workspace(&mut self, state: MoveRollback<'_>) {
        self.suppressed_focus = self.backend.focused_window();
        self.suppressed_app = self.backend.frontmost_application();
        for window in state.parked_displaced {
            self.restore_window(&window);
        }
        if state.moved_relocated {
            let no_cancel = AtomicBool::new(false);
            let _ = self.layout_workspace(state.moved_ws, state.source_output, &no_cancel);
            for (window, original) in state.moved_floating_originals {
                let _: macos::ApplyOutcome = self.backend.set_position(window, *original);
            }
        }
        for window in state.revealed_source {
            if let Some(&original) = state.source_incoming_originals.get(window)
                && !self.mark_parked(window.clone(), original)
            {
                log_event!("rollback mark_parked failed for {:?}", window);
            }
            let _: macos::ApplyOutcome =
                self.park_window(window, state.source_output, state.outputs);
        }
    }
}

pub(super) fn translate_frame_to_output(
    frame: crate::Rect,
    source_output: crate::Rect,
    target_output: crate::Rect,
) -> crate::Rect {
    let offset_x = frame.x - source_output.x;
    let offset_y = frame.y - source_output.y;
    let max_x = target_output.x + (target_output.width - frame.width).max(0);
    let max_y = target_output.y + (target_output.height - frame.height).max(0);
    crate::Rect {
        x: (target_output.x + offset_x).clamp(target_output.x, max_x),
        y: (target_output.y + offset_y).clamp(target_output.y, max_y),
        ..frame
    }
}
