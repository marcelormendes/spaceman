//! Display changes. Parked positions are chosen against the connected displays;
//! when a display is added or removed they may land on screen, and recovery
//! frames recorded on a removed display would restore off every screen.

use super::*;

/// The displays the runtime last saw, and the hidden windows still to re-park
/// since they changed.
#[derive(Default)]
pub(super) struct DisplayWatch {
    known: Option<macos::DisplayOutputs>,
    repark_pending: HashSet<crate::PhysicalWindowId>,
    /// Workspaces whose display assignment changed on config reload, mapped to
    /// the selector they moved from until the next cycle relocates their
    /// recovery frames and floating windows.
    reassigned: HashMap<WorkspaceId, DisplaySelector>,
}

impl<B: WorkspaceBackend> WorkspaceRuntime<B> {
    /// Record that a config reload moved `workspace` off `previous`. Keeps the
    /// earliest source selector if multiple edits arrive before the next cycle.
    pub(super) fn note_reassigned_workspace(
        &mut self,
        workspace: WorkspaceId,
        previous: DisplaySelector,
    ) {
        self.displays
            .reassigned
            .entry(workspace)
            .or_insert(previous);
    }

    /// Translate parked recovery frames and carry visible floating windows for
    /// workspaces whose display assignment changed on config reload.
    pub(super) fn relocate_reassigned_workspaces(
        &mut self,
        outputs: &macos::DisplayOutputs,
        visible: &[ResolvedWorkspace],
        failed: &mut usize,
    ) {
        if self.displays.reassigned.is_empty() {
            return;
        }
        let visible_workspaces: HashSet<_> =
            visible.iter().map(|resolved| resolved.workspace).collect();
        let reassigned = std::mem::take(&mut self.displays.reassigned);
        for (workspace, old_selector) in reassigned {
            let old_output = outputs.resolve(&old_selector).frame;
            let new_output = output_for(&self.workspaces, workspace, outputs);
            if old_output == new_output {
                continue;
            }
            let is_visible = visible_workspaces.contains(&workspace);
            for window in self.workspaces.windows(workspace) {
                if let Some(original) = self.parked.get(&window).copied() {
                    if old_output.contains_center(original) {
                        let relocated = super::move_workspace::translate_frame_to_output(
                            original, old_output, new_output,
                        );
                        if !self.record_recovery(&window, relocated) {
                            *failed += 1;
                        }
                    }
                } else if is_visible
                    && self.workspaces.floating_workspace_of(&window) == Some(workspace)
                    && !self.carry_floating_window(&window, old_output, new_output)
                {
                    *failed += 1;
                }
            }
        }
    }

    /// How tiles fill `output`: its frame with the configured gaps, padding and width cap.
    pub(super) fn layout_settings(&self, output: crate::Rect) -> Settings {
        Settings::for_output(output).spaced(self.config.spacing())
    }

    /// Record the current displays. When they differ from the last ones, every
    /// parked window is due a re-park against them.
    pub(super) fn note_outputs(&mut self, outputs: &macos::DisplayOutputs) {
        if self
            .displays
            .known
            .as_ref()
            .is_some_and(|known| !known.frames().eq(outputs.frames()))
        {
            log_event!("displays changed: re-parking hidden windows");
            self.displays.repark_pending = self.parked.keys().cloned().collect();
        }
        self.displays.known = Some(outputs.clone());
    }

    /// Note the displays, then re-park each hidden window still due one. Runs
    /// before focus follow and heal, which would otherwise take a window the
    /// new display uncovered for one the user brought back. A write that fails
    /// (the app is briefly inaccessible) stays due and is retried next time.
    pub(super) fn follow_displays(&mut self, outputs: &macos::DisplayOutputs) {
        self.note_outputs(outputs);
        if self.displays.repark_pending.is_empty() {
            return;
        }
        let visible: HashSet<_> = self
            .visible_workspaces(outputs)
            .into_iter()
            .map(|resolved| resolved.workspace)
            .collect();
        let displays: Vec<_> = outputs.frames().collect();
        for window in std::mem::take(&mut self.displays.repark_pending) {
            // Unowned windows are restored by the cycle, visible ones revealed.
            let Some(owner) = self.workspaces.workspace_of_any(&window) else {
                continue;
            };
            if !self.parked.contains_key(&window)
                || visible.contains(&owner)
                || self.parking_exempt.contains(&window)
            {
                continue;
            }
            if !self.repark_for_displays(&window, owner, outputs, &displays) {
                self.displays.repark_pending.insert(window);
            }
        }
    }

    /// Whether `window` is parked for `displays` now.
    fn repark_for_displays(
        &mut self,
        window: &crate::PhysicalWindowId,
        owner: WorkspaceId,
        outputs: &macos::DisplayOutputs,
        displays: &[crate::Rect],
    ) -> bool {
        let Some(current) = self.backend.window_server_frame(window) else {
            return false;
        };
        let output = output_for(&self.workspaces, owner, outputs);
        let target = crate::geometry::parked_frame(output, displays, current);
        if (target.x, target.y) == (current.x, current.y) {
            return true;
        }
        let outcome = self.backend.set_position(window, target);
        log_event!("displays changed: re-park {window:?} at {target:?} ({outcome:?})");
        if outcome != macos::ApplyOutcome::Applied {
            return false;
        }
        self.recently_parked.insert(window.clone(), Instant::now());
        true
    }

    /// Whether `window` still awaits its re-park after a display change. Until
    /// then a new display may show it, which is no reappearance to honor.
    pub(super) fn awaiting_display_repark(&self, window: &crate::PhysicalWindowId) -> bool {
        self.displays.repark_pending.contains(window)
    }

    /// Where a recovery frame is restored: itself while it shows on a connected
    /// display, otherwise moved onto the main display (its own is gone).
    pub(super) fn restore_target(&self, frame: crate::Rect) -> crate::Rect {
        let Some(outputs) = &self.displays.known else {
            return frame;
        };
        let displays: Vec<_> = outputs.frames().collect();
        crate::geometry::onto_displays(frame, &displays, outputs.main().frame)
    }
}
