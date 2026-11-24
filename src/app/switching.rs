use super::parking::RESTORE_CONFIRM_TOLERANCE;
use super::*;

enum MoveWindowOutcome {
    Moved(CycleStatus),
    Retired(CycleStatus),
}

const REVEAL_SETTLE_LIMIT: Duration = Duration::from_millis(300);
/// Poll interval while waiting for incoming windows to arrive. The on-screen
/// window list costs about half a millisecond, so polling this often is cheap
/// and takes most of the wait out of a switch.
const REVEAL_POLL_INTERVAL: Duration = Duration::from_millis(3);
/// How long a pending reveal waits before the restore write is re-issued.
const REVEAL_REWRITE_INTERVAL: Duration = Duration::from_millis(100);

use super::commands::{PendingFocus, WarpContext, WorkspaceFocusOutcome};

mod repeat;

pub(super) use repeat::RepeatSwitch;

impl<B: WorkspaceBackend> WorkspaceRuntime<B> {
    pub(super) fn switch(
        &mut self,
        workspace: WorkspaceId,
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<CycleStatus, String> {
        self.switch_with(workspace, outputs, cancelled, RepeatSwitch::KeepFocus)
    }

    pub(super) fn switch_with(
        &mut self,
        workspace: WorkspaceId,
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
        repeat: RepeatSwitch,
    ) -> Result<CycleStatus, String> {
        self.synchronize_foreground(&outputs);
        let selector = self.ensure_managed_workspace(workspace)?;
        let output = output_for(&self.workspaces, workspace, &outputs);
        let display_id = outputs.resolve(&selector).id.clone();
        let previous = self
            .visible_workspaces(&outputs)
            .into_iter()
            .find(|resolved| resolved.display_id == display_id)
            .map_or(workspace, |resolved| resolved.workspace);
        if previous == workspace {
            // A repeat press steps from the window the previous press focused,
            // which Accessibility may not report yet. Every other request
            // trusts the reported focus exactly as before.
            let focused = if repeat == RepeatSwitch::CycleFocus {
                self.current_focus()
            } else {
                self.backend.focused_window()
            };
            // A repeat press of the shortcut for the workspace already up and
            // focused walks its tiles. With focus somewhere else (another
            // display's workspace, an unmanaged window) the press keeps its old
            // job of bringing focus back to this workspace, and only the press
            // after that cycles.
            if repeat == RepeatSwitch::CycleFocus
                && let Some(focused) = focused
                    .as_ref()
                    .filter(|focused| self.workspaces.workspace_of_any(focused) == Some(workspace))
                && self.cycle_focus_on_repeat(workspace, focused, &outputs)?
            {
                let mut status = self.status(0, &outputs);
                status.hud = Some(self.hud_for(workspace, UiHudKind::Workspace, None, &outputs));
                return Ok(status);
            }
            let windows = self.workspaces.windows(workspace);
            if self.workspaces.uses_fullscreen_layout(workspace)
                && let Some(focused) = focused
                    .as_ref()
                    .filter(|focused| self.workspaces.workspace_of(focused) == Some(workspace))
            {
                self.workspaces.remember_focus(workspace, focused);
                self.record_focused_workspace(workspace);
                self.pending_fullscreen_focus_restore.remove(&workspace);
                let frame = self.focus_window_frame(focused, workspace, output);
                self.follow_pointer_to(&display_id, output, frame, &outputs);
                self.persist_workspace_state_if_changed()?;
                let mut status = self.status(0, &outputs);
                status.hud = Some(self.hud_for(workspace, UiHudKind::Workspace, None, &outputs));
                return Ok(status);
            }
            let frontmost = self.backend.frontmost_application();
            let prior_stale = self.take_pending_focus_stale();
            // A tile of this workspace that already has focus keeps it; only
            // focus elsewhere moves to the remembered tile.
            let outcome = self.focus_workspace_entry(
                workspace,
                &windows,
                focused,
                frontmost,
                true,
                prior_stale,
            );
            self.follow_pointer_for_workspace(workspace, output, outcome, &outputs);
            // Selecting a workspace that is already up still moves focus there
            // when it lives on another display, so it belongs in the history.
            self.record_focused_workspace(workspace);
            let mut status = self.status(0, &outputs);
            status.hud = Some(self.hud_for(workspace, UiHudKind::Workspace, None, &outputs));
            return Ok(status);
        }
        // The focus before the switch, trusting an in-flight focus command over
        // a stale Accessibility report before clearing pending_focus for the
        // incoming workspace. Keep the raw reported focus separately so
        // focus_workspace_entry still suppresses the stale window AX reports.
        let reported_focused_before = self.backend.focused_window();
        let focused_before = self.resolve_focus(reported_focused_before.clone());
        let prior_stale = self.take_pending_focus_stale();
        let switch_started = Instant::now();
        let mut outgoing = self.workspaces.windows(previous);
        self.order_back_to_front(&mut outgoing, |window| window.window_id);
        let incoming = self.workspaces.windows(workspace);
        log_event!(
            "switch: workspace {} -> {} (outgoing {}, incoming {})",
            previous,
            workspace,
            outgoing.len(),
            incoming.len()
        );
        let frontmost_before = self.backend.frontmost_application();
        if let Some(focused) = focused_before
            .as_ref()
            .filter(|focused| outgoing.contains(focused))
        {
            self.workspaces.remember_focus(previous, focused);
        }
        // Reveal incoming windows first: position writes do not raise, so they
        // arrive and get laid out beneath the outgoing windows. Hiding the
        // outgoing workspace afterwards exposes a fully formed workspace and
        // the bare desktop never shows. Failures before parking leave the
        // screen untouched.
        let incoming_originals: HashMap<_, _> = incoming
            .iter()
            .filter_map(|window| {
                self.parked
                    .get(window)
                    .copied()
                    .map(|frame| (window.clone(), frame))
            })
            .collect();
        let mut restored = Vec::new();
        for window in &incoming {
            if cancelled.load(Ordering::Acquire) {
                log_event!("switch cancelled while revealing incoming windows");
                self.rollback_switch(Vec::new(), &restored, &incoming_originals, output, &outputs);
                return Err("workspace switch cancelled".into());
            }
            if !self.parked.contains_key(window) {
                continue;
            }
            restored.push(window.clone());
            match self.restore_window(window) {
                macos::ApplyOutcome::Applied => {}
                macos::ApplyOutcome::Gone => {
                    self.forget_window(workspace, window);
                }
                outcome => {
                    log_event!(
                        "switch rollback: could not restore {:?} ({:?})",
                        window,
                        outcome
                    );
                    self.rollback_switch(
                        Vec::new(),
                        &restored,
                        &incoming_originals,
                        output,
                        &outputs,
                    );
                    return Err(format!(
                        "Workspace {} could not restore all of its windows",
                        workspace
                    ));
                }
            }
        }
        if !self.wait_for_reveal(&incoming, cancelled) {
            self.rollback_switch(Vec::new(), &restored, &incoming_originals, output, &outputs);
            if cancelled.load(Ordering::Acquire) {
                return Err("workspace switch cancelled".into());
            }
            return Err(format!(
                "Workspace {workspace} stayed inactive because its windows did not finish restoring"
            ));
        }
        let layout = self.layout_workspace(workspace, output, cancelled);
        if cancelled.load(Ordering::Acquire) {
            log_event!("switch cancelled while laying out incoming windows");
            self.rollback_switch(Vec::new(), &incoming, &incoming_originals, output, &outputs);
            return Err("workspace switch cancelled".into());
        }
        if layout.blocked {
            log_event!("switch rollback: workspace {} layout blocked", workspace);
            self.rollback_switch(Vec::new(), &incoming, &incoming_originals, output, &outputs);
            return Err(format!(
                "Workspace {} stayed inactive because its windows could not be laid out",
                workspace
            ));
        }
        let mut parked_outgoing = Vec::new();
        for window in &outgoing {
            if cancelled.load(Ordering::Acquire) {
                log_event!("switch cancelled while parking outgoing windows");
                self.rollback_switch(
                    parked_outgoing,
                    &incoming,
                    &incoming_originals,
                    output,
                    &outputs,
                );
                return Err("workspace switch cancelled".into());
            }
            match self.park_window(window, output, &outputs) {
                macos::ApplyOutcome::Applied => parked_outgoing.push(window.clone()),
                macos::ApplyOutcome::Gone => {
                    self.forget_window(previous, window);
                }
                outcome => {
                    log_event!(
                        "switch rollback: could not park {:?} ({:?})",
                        window,
                        outcome
                    );
                    self.rollback_switch(
                        parked_outgoing,
                        &incoming,
                        &incoming_originals,
                        output,
                        &outputs,
                    );
                    return Err(format!(
                        "Workspace {} stayed active because a window could not be parked",
                        previous
                    ));
                }
            }
        }
        let focus_already_in_workspace = reported_focused_before
            .as_ref()
            .is_some_and(|window| self.workspaces.workspace_of_any(window) == Some(workspace))
            || frontmost_before
                .as_ref()
                .is_some_and(|app| self.workspaces.workspace_for_app(app) == Some(workspace));
        self.workspaces.switch(workspace);
        self.record_focused_workspace(workspace);
        self.foreground_lanes.insert(display_id, selector);
        // Re-read incoming windows after reveal/layout so a tile retired as Gone
        // during restore is never chosen over a surviving tile (#308).
        let live_incoming = self.workspaces.windows(workspace);
        let outcome = self.focus_workspace_entry(
            workspace,
            &live_incoming,
            reported_focused_before,
            frontmost_before,
            true,
            prior_stale,
        );
        if self.warp_context == WarpContext::Explicit
            || (self.warp_context == WarpContext::Direct && !focus_already_in_workspace)
        {
            self.follow_pointer_for_workspace(workspace, output, outcome, &outputs);
        }
        self.persist_workspace_state_if_changed()?;
        log_event!(
            "switch complete: workspace {} in {}ms",
            workspace,
            switch_started.elapsed().as_millis()
        );
        let mut status = self.status(layout.failed, &outputs);
        status.hud = Some(self.hud_for(workspace, UiHudKind::Workspace, Some(previous), &outputs));
        Ok(status)
    }

    /// Focus the workspace's entry window and suppress focus follow for the
    /// focus left behind. That focus stays put when the workspace is empty,
    /// and macOS activates the entry window asynchronously, so for a few
    /// cycles Accessibility can still report the old window or app; following
    /// it would snap the display straight back. Suppression keeps the earlier
    /// stale focus when Accessibility reports none, so switching on through
    /// empty workspaces keeps it too.
    ///
    /// With `keep_reported`, a window of the workspace that already has focus
    /// is the entry window: a hidden workspace revealed because one of its
    /// windows took focus (a notification click, the Dock, Cmd+Tab) must not
    /// hand focus to its remembered tile instead.
    pub(super) fn focus_workspace_entry(
        &mut self,
        workspace: WorkspaceId,
        windows: &[crate::PhysicalWindowId],
        focused: Option<crate::PhysicalWindowId>,
        frontmost: Option<crate::AppInstanceId>,
        keep_reported: bool,
        mut prior_stale: Vec<Option<crate::PhysicalWindowId>>,
    ) -> WorkspaceFocusOutcome {
        let reported_window = focused.as_ref().filter(|window| windows.contains(window));
        let reported_inside = reported_window.is_some();
        let workspace_focus = reported_window
            .filter(|_| keep_reported)
            .cloned()
            .or_else(|| {
                self.workspaces
                    .preferred_float_focus(workspace)
                    .filter(|window| windows.contains(window))
            })
            .or_else(|| {
                self.workspaces
                    .preferred_focus(workspace)
                    .filter(|window| windows.contains(window))
            })
            .or_else(|| windows.first().cloned());
        let stale_window = focused.or_else(|| self.suppressed_focus.clone());
        let stale_app = frontmost.or_else(|| self.suppressed_app.clone());
        let outcome = if let Some(window) = workspace_focus {
            if self.backend.focus_window(&window) {
                self.pending_fullscreen_focus_restore.remove(&workspace);
                if !reported_inside {
                    prior_stale.retain(|entry| entry.as_ref() != Some(&window));
                    if stale_window.as_ref() != Some(&window)
                        && !prior_stale.contains(&stale_window)
                    {
                        prior_stale.push(stale_window.clone());
                    }
                    self.pending_focus = Some(PendingFocus {
                        target: window.clone(),
                        stale: prior_stale,
                        at: Instant::now(),
                    });
                }
                WorkspaceFocusOutcome::Focused(window)
            } else {
                // Keys may still be in the app focus left; the pending restore
                // stays so the next cycle tries again.
                log_event!(
                    "switch focus failed: workspace {} window {:?}",
                    workspace,
                    window
                );
                WorkspaceFocusOutcome::Failed
            }
        } else {
            WorkspaceFocusOutcome::Empty
        };
        // Focus already inside the workspace (focus follow switched to it) is
        // where the display is going, not focus left behind. Only the focus
        // Accessibility reports now proves that: an earlier suppressed window
        // that lives here must not drop the guard on the frontmost app.
        if reported_inside {
            self.clear_suppressed_focus();
        } else {
            self.suppressed_focus = stale_window.filter(|window| !windows.contains(window));
            self.suppressed_app = stale_app;
        }
        outcome
    }

    pub(super) fn clear_suppressed_focus(&mut self) {
        self.suppressed_focus = None;
        self.suppressed_app = None;
    }

    /// Suppress focus follow when parking the window (or app) that currently
    /// holds focus so the next cycle does not jump to the hidden workspace.
    pub(super) fn suppress_parked_focus(
        &mut self,
        window: &crate::PhysicalWindowId,
        focused: Option<&crate::PhysicalWindowId>,
        frontmost: Option<&AppInstanceId>,
    ) {
        if focused == Some(window) || frontmost == Some(&window.app) {
            self.suppressed_focus = Some(window.clone());
            self.suppressed_app = Some(window.app.clone());
        }
    }

    /// Bounded wait until every incoming window's restore is confirmed by the
    /// window server (pending restores keep their recovery frames). Slow
    /// applications apply moves asynchronously; hiding the outgoing workspace
    /// before they arrive would flash the bare desktop. Polling is
    /// confirmation-only and starts immediately, so apps that move
    /// synchronously finish without a sleep; the restore write is re-issued
    /// only for windows that still have not arrived after
    /// [`REVEAL_REWRITE_INTERVAL`].
    pub(super) fn wait_for_reveal(
        &mut self,
        incoming: &[crate::PhysicalWindowId],
        cancelled: &AtomicBool,
    ) -> bool {
        let started = Instant::now();
        let mut last_rewrite = started;
        loop {
            let pending: Vec<_> = incoming
                .iter()
                .filter(|window| self.parked.contains_key(window))
                .cloned()
                .collect();
            if pending.is_empty() {
                return !cancelled.load(Ordering::Acquire);
            }
            if started.elapsed() >= REVEAL_SETTLE_LIMIT || cancelled.load(Ordering::Acquire) {
                log_event!(
                    "reveal interrupted: {} window(s) still arriving; keeping outgoing workspace",
                    pending.len()
                );
                return false;
            }
            let Some(frames) = self.reveal_frames(&pending) else {
                thread::sleep(REVEAL_POLL_INTERVAL);
                continue;
            };
            let mut removed = Vec::new();
            for window in &pending {
                let Some(original) = self.parked.get(window).copied() else {
                    continue;
                };
                let target = self.restore_target(original);
                if frames.get(&window.window_id).is_some_and(|actual| {
                    actual.position_approximately_equals(target, RESTORE_CONFIRM_TOLERANCE)
                }) {
                    self.commit_unparked(window);
                    removed.push((window.clone(), original));
                }
            }
            if last_rewrite.elapsed() >= REVEAL_REWRITE_INTERVAL {
                last_rewrite = Instant::now();
                for window in &pending {
                    let Some(frame) = self.parked.get(window).copied() else {
                        continue;
                    };
                    if matches!(
                        self.write_restore_position(window, frame),
                        macos::ApplyOutcome::Gone
                    ) {
                        removed.push((window.clone(), frame));
                    }
                }
            }
            self.persist_removals_or_rollback(removed);
            if incoming
                .iter()
                .any(|window| self.parked.contains_key(window))
            {
                thread::sleep(REVEAL_POLL_INTERVAL);
            }
        }
    }

    /// Window-server frames to confirm pending reveals against. A revealed
    /// window is on screen, so the on-screen list (about 5x cheaper than the
    /// full one) settles every window it contains; the full list is read only
    /// when a pending window is missing from it (another Space, ordered out),
    /// so such windows confirm exactly as before.
    fn reveal_frames(
        &mut self,
        pending: &[crate::PhysicalWindowId],
    ) -> Option<HashMap<u32, crate::Rect>> {
        if let Some(frames) = self.backend.onscreen_window_frames()
            && pending
                .iter()
                .all(|window| frames.contains_key(&window.window_id))
        {
            return Some(frames);
        }
        self.backend.window_server_frames()
    }

    /// Park the window furthest back first. Hiding the front window first
    /// uncovers the windows stacked beneath it (every Fullscreen Layout tile
    /// shares one frame) until they are parked too, which reads as a flash.
    pub(super) fn order_back_to_front<T>(
        &mut self,
        items: &mut [T],
        window_id: impl Fn(&T) -> u32,
    ) {
        if items.len() < 2 {
            return;
        }
        let Some(order) = self.backend.onscreen_window_order() else {
            return;
        };
        let rank: HashMap<u32, usize> = order
            .iter()
            .enumerate()
            .map(|(rank, id)| (*id, rank))
            .collect();
        // Off-screen windows uncover nothing; they sort first.
        items.sort_by_key(|item| {
            std::cmp::Reverse(rank.get(&window_id(item)).copied().unwrap_or(usize::MAX))
        });
    }

    /// Undo a partially applied switch: bring the outgoing windows back and
    /// hide any incoming windows that were already revealed.
    fn rollback_switch(
        &mut self,
        parked_outgoing: Vec<crate::PhysicalWindowId>,
        revealed: &[crate::PhysicalWindowId],
        incoming_originals: &HashMap<crate::PhysicalWindowId, crate::Rect>,
        output: crate::Rect,
        outputs: &macos::DisplayOutputs,
    ) {
        self.suppressed_focus = self.backend.focused_window();
        self.suppressed_app = self.backend.frontmost_application();
        for window in parked_outgoing {
            self.restore_window(&window);
        }
        for window in revealed {
            if let Some(&original) = incoming_originals.get(window)
                && !self.mark_parked(window.clone(), original)
            {
                log_event!("rollback mark_parked failed for {:?}", window);
            }
            // A pending restore may land after rollback. Supersede that write
            // even when its recovery entry has not yet been retired.
            let _: macos::ApplyOutcome = self.park_window(window, output, outputs);
        }
    }

    /// Remember the workspace that just took focus. The first record after a
    /// launch seeds the previous workspace from the focused display's persisted
    /// lane history, so Option+Tab works straight away as it did before.
    pub(super) fn record_focused_workspace(&mut self, workspace: WorkspaceId) {
        if self.last_focused_workspace == Some(workspace) {
            return;
        }
        if self.last_focused_workspace.is_none() {
            let selector = self.workspaces.assignment(workspace).clone();
            self.previous_focused_workspace = self
                .workspaces
                .previous_for(&selector)
                .filter(|previous| *previous != workspace);
        } else {
            self.previous_focused_workspace = self.last_focused_workspace;
        }
        self.last_focused_workspace = Some(workspace);
    }

    /// Global back-and-forth: move focus to the workspace focused before this
    /// one, wherever it lives. A workspace already on screen is focused in
    /// place, so the display the focus came from keeps showing its workspace
    /// (pressing it again bounces straight back). Only a hidden target switches
    /// its own display to it.
    pub(super) fn workspace_back_and_forth(
        &mut self,
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<CycleStatus, String> {
        self.synchronize_foreground(&outputs);
        let focused = self.backend.focused_window();
        // Accessibility can still report the window a switch or a cross-display
        // focus just left behind; history names the workspace that took focus.
        let reported = focused
            .as_ref()
            .filter(|window| self.suppressed_focus.as_ref() != Some(*window))
            .and_then(|window| self.workspaces.workspace_of_any(window));
        if let Some(reported) = reported {
            // A click on another display is a genuine focus change even before
            // the next observation cycle records it.
            self.record_focused_workspace(reported);
        }
        let current = reported.or(self.last_focused_workspace);
        let target = self
            .previous_focused_workspace
            .filter(|target| Some(*target) != current && self.workspaces.is_enabled(*target))
            .ok_or("No previous workspace")?;
        if self.is_visible(target, &outputs) {
            return self.focus_visible_workspace(target, current, &outputs);
        }
        self.switch(target, outputs, cancelled)
    }

    /// Focus a workspace that is already on screen instead of switching any
    /// display. An empty one has nothing to focus, so it only shows its HUD and
    /// leaves the history pointing where it was.
    fn focus_visible_workspace(
        &mut self,
        workspace: WorkspaceId,
        from: Option<WorkspaceId>,
        outputs: &macos::DisplayOutputs,
    ) -> Result<CycleStatus, String> {
        let windows = self.workspaces.windows(workspace);
        let output = output_for(&self.workspaces, workspace, outputs);
        if !windows.is_empty() {
            let focused = self.backend.focused_window();
            let frontmost = self.backend.frontmost_application();
            let prior_stale = self.take_pending_focus_stale();
            let outcome = self.focus_workspace_entry(
                workspace,
                &windows,
                focused,
                frontmost,
                false,
                prior_stale,
            );
            self.follow_pointer_for_workspace(workspace, output, outcome, outputs);
            self.record_focused_workspace(workspace);
            log_event!(
                "back-and-forth: focused visible workspace {} without switching",
                workspace
            );
        } else {
            let target_display_id = outputs
                .resolve(self.workspaces.assignment(workspace))
                .id
                .as_str();
            self.follow_pointer_to(target_display_id, output, None, outputs);
        }
        let mut status = self.status(0, outputs);
        status.hud = Some(self.hud_for(workspace, UiHudKind::Workspace, from, outputs));
        Ok(status)
    }

    /// Left goes to the next workspace in the display's menu section, right to
    /// the previous one. Nothing happens past either end.
    pub(super) fn swipe(
        &mut self,
        swipe: Swipe,
        point: (i32, i32),
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<CycleStatus, String> {
        self.synchronize_foreground(&outputs);
        let display_id = outputs.display_for_point(point).id.clone();
        let current = self
            .visible_workspaces(&outputs)
            .into_iter()
            .find(|resolved| resolved.display_id == display_id)
            .map(|resolved| resolved.workspace);
        let listed = self.listed_workspaces(&display_id, current, &outputs);
        let target = listed
            .iter()
            .position(|workspace| Some(*workspace) == current)
            .and_then(|index| match swipe {
                Swipe::Left => listed.get(index + 1),
                Swipe::Right => index.checked_sub(1).and_then(|index| listed.get(index)),
            });
        match target {
            Some(&target) => self.with_warp_context(WarpContext::Suppressed, |rt| {
                rt.switch(target, outputs, cancelled)
            }),
            None => Ok(self.status(0, &outputs)),
        }
    }

    pub(super) fn move_focused(
        &mut self,
        target: WorkspaceId,
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<CycleStatus, String> {
        let window = self
            .backend
            .focused_window()
            .ok_or("Focused window is unavailable")?;
        self.move_window(window, target, outputs, cancelled)
    }

    pub(super) fn move_focused_follow(
        &mut self,
        target: WorkspaceId,
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<CycleStatus, String> {
        let window = self
            .backend
            .focused_window()
            .ok_or("Focused window is unavailable")?;
        let was_visible = self.is_visible(target, &outputs);
        let moved = self.move_window(window.clone(), target, outputs.clone(), cancelled)?;
        if was_visible {
            return Ok(moved);
        }
        self.workspaces.remember_focus(target, &window);
        let mut switched = self.switch(target, outputs, cancelled)?;
        switched.failed += moved.failed;
        Ok(switched)
    }

    pub(super) fn move_app(
        &mut self,
        bundle_id: &str,
        target: WorkspaceId,
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<CycleStatus, String> {
        let mut windows = self.windows_for_bundle(bundle_id);
        if windows.is_empty() {
            self.with_warp_context(WarpContext::Suppressed, |rt| {
                rt.cycle(outputs.clone(), cancelled)
            })?;
            windows = self.windows_for_bundle(bundle_id);
        }
        let mut status: Option<CycleStatus> = None;
        let mut retired_failed = 0;
        for window in windows {
            match self.with_warp_context(WarpContext::Suppressed, |rt| {
                rt.move_window_outcome(window, target, outputs.clone(), cancelled)
            })? {
                MoveWindowOutcome::Moved(mut moved) => {
                    moved.failed += status.map_or(retired_failed, |earlier| earlier.failed);
                    retired_failed = 0;
                    status = Some(moved);
                }
                MoveWindowOutcome::Retired(mut retired) => {
                    if let Some(earlier) = status.take() {
                        retired.failed += earlier.failed;
                        retired.hud = earlier.hud;
                        status = Some(retired);
                    } else {
                        retired_failed += retired.failed;
                    }
                }
            }
        }
        status.ok_or_else(|| format!("No managed window found for {bundle_id}"))
    }

    /// Every window of the bundle Spaceman owns: its tiles, then its floats.
    fn windows_for_bundle(&self, bundle_id: &str) -> Vec<crate::PhysicalWindowId> {
        let mut floating: Vec<_> = self
            .workspaces
            .floating_entries()
            .map(|(window, _)| window.clone())
            .collect();
        floating.sort();
        WorkspaceId::all()
            .flat_map(|workspace| self.workspaces.engine(workspace).bound_windows())
            .chain(floating)
            .filter(|window| self.bundle_id(&window.app) == Some(bundle_id))
            .collect()
    }

    pub(super) fn move_window(
        &mut self,
        window: crate::PhysicalWindowId,
        target: WorkspaceId,
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<CycleStatus, String> {
        match self.move_window_outcome(window, target, outputs, cancelled)? {
            MoveWindowOutcome::Moved(status) | MoveWindowOutcome::Retired(status) => Ok(status),
        }
    }

    fn move_window_outcome(
        &mut self,
        window: crate::PhysicalWindowId,
        target: WorkspaceId,
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<MoveWindowOutcome, String> {
        self.ensure_managed_workspace(target)?;
        let source = if let Some(source) = self.workspaces.workspace_of_any(&window) {
            source
        } else {
            self.with_warp_context(WarpContext::Suppressed, |rt| {
                rt.cycle(outputs.clone(), cancelled)
            })?;
            self.workspaces
                .workspace_of_any(&window)
                .ok_or("Window is not managed by Spaceman")?
        };
        if source == target {
            return Ok(MoveWindowOutcome::Moved(self.status(0, &outputs)));
        }
        let is_floating = self.workspaces.floating_workspace_of(&window).is_some();
        if is_floating {
            self.settle_pending_summon_for_move(&window);
        }
        let source_output = output_for(&self.workspaces, source, &outputs);
        let target_output = output_for(&self.workspaces, target, &outputs);
        let window_output = if is_floating {
            self.window_display_output(&window, &outputs)
                .unwrap_or(source_output)
        } else {
            source_output
        };
        let target_is_active = self.is_visible(target, &outputs);
        if !target_is_active {
            let parked = if window_output == target_output {
                self.park_window(&window, target_output, &outputs)
            } else {
                self.park_window_across(&window, window_output, target_output, &outputs)
            };
            if parked == macos::ApplyOutcome::Gone
                && self.retire_confirmed_gone_window(source, &window)
            {
                let mut failed = 0;
                if !is_floating && self.is_visible(source, &outputs) {
                    failed += self
                        .layout_workspace(source, source_output, cancelled)
                        .failed;
                }
                self.persist_workspace_state_if_changed()?;
                return Ok(MoveWindowOutcome::Retired(self.status(failed, &outputs)));
            }
            if !matches!(parked, macos::ApplyOutcome::Applied) {
                return Err("Window could not be parked in the target workspace".into());
            }
        }
        if is_floating {
            self.workspaces.own_floating(window.clone(), target);
        } else {
            self.workspaces
                .move_window(&window, target)
                .ok_or("Window could not be moved")?;
            self.exit_fullscreen_if_split(target);
        }
        self.clear_fullscreen_restore_if_inactive(source);
        self.clear_relocation_evidence(source, target, &window);
        self.note_hand_move(&window, source, target);
        if !target_is_active {
            self.suppressed_app = Some(window.app.clone());
            self.suppressed_focus = Some(window.clone());
        }
        let mut failed = 0;
        if !is_floating && self.is_visible(source, &outputs) {
            let layout = self.layout_workspace(source, source_output, cancelled);
            failed += layout.failed;
        }
        if target_is_active {
            let was_parked = self.parked.contains_key(&window);
            if was_parked {
                if is_floating && let Some(&recovery) = self.parked.get(&window) {
                    let displays: Vec<_> = outputs.frames().collect();
                    let recovery_output =
                        Self::display_for_frame(recovery, &displays).unwrap_or(source_output);
                    if recovery_output != target_output {
                        let relocated = super::move_workspace::translate_frame_to_output(
                            recovery,
                            recovery_output,
                            target_output,
                        );
                        let _: bool = self.record_recovery(&window, relocated);
                    }
                }
                if !matches!(
                    self.restore_window(&window),
                    macos::ApplyOutcome::Applied | macos::ApplyOutcome::Gone
                ) {
                    failed += 1;
                }
            }
            if !is_floating {
                let layout = self.layout_workspace(target, target_output, cancelled);
                failed += layout.failed;
            } else if !was_parked
                && window_output != target_output
                && !self.carry_floating_window(&window, window_output, target_output)
            {
                failed += 1;
            }
            if self.backend.focus_window(&window) {
                // Focus followed the window to the workspace on screen
                // elsewhere; Option+Tab goes back to where it came from.
                self.record_focused_workspace(target);
                let frame = self.focus_window_frame(&window, target, target_output);
                let target_display_id = outputs
                    .resolve(self.workspaces.assignment(target))
                    .id
                    .as_str();
                self.follow_pointer_to(target_display_id, target_output, frame, &outputs);
            } else {
                log_event!("move focus follow failed for {:?}", window);
            }
        }
        self.persist_workspace_state_if_changed()?;
        let mut status = self.status(failed, &outputs);
        status.hud = Some(self.hud_for(target, UiHudKind::Moved, None, &outputs));
        Ok(MoveWindowOutcome::Moved(status))
    }

    /// A floating window has no tile to carry it, so moving it to a workspace
    /// on screen on another display moves it there directly, keeping its size
    /// and its place relative to the display. A window already there stays.
    pub(super) fn carry_floating_window(
        &mut self,
        window: &crate::PhysicalWindowId,
        from: crate::Rect,
        to: crate::Rect,
    ) -> bool {
        let frame = match self.backend.frame(window) {
            Ok(frame) => self.backend.window_server_frame(window).unwrap_or(frame),
            Err(macos::WindowAccessError::Gone) => return true,
            Err(macos::WindowAccessError::Unavailable) => return false,
        };
        if to.contains_center(frame) {
            return true;
        }
        let target = super::move_workspace::translate_frame_to_output(frame, from, to);
        let outcome = self.backend.set_position(window, target);
        log_event!(
            "move floating: {:?} to {:?} ({:?})",
            window,
            target,
            outcome
        );
        matches!(
            outcome,
            macos::ApplyOutcome::Applied | macos::ApplyOutcome::Gone
        )
    }
}
