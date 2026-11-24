use super::{state_writer::StateWriter, *};

#[derive(serde::Deserialize, serde::Serialize)]
pub(super) struct ParkedWindows(
    #[serde(with = "crate::serde_map")] pub(super) HashMap<crate::PhysicalWindowId, crate::Rect>,
);

#[derive(serde::Serialize)]
struct ParkedWindowsRef<'a>(
    #[serde(with = "crate::serde_map")] &'a HashMap<crate::PhysicalWindowId, crate::Rect>,
);

pub(super) trait WorkspaceBackend {
    fn snapshot(
        &mut self,
        cycle: u64,
        outputs: &macos::DisplayOutputs,
    ) -> Result<crate::NativeObservation, String>;
    fn is_process_running(&mut self, app: &AppInstanceId) -> bool;
    /// Bundle ID for a running app whose windows have not been observed in this
    /// session yet (for example, parked on a hidden workspace across a restart).
    fn bundle_id(&mut self, _app: &AppInstanceId) -> Option<String> {
        None
    }
    /// True only when a visible app successfully reports an empty AXWindows list.
    fn confirms_no_windows(&mut self, app: &AppInstanceId) -> bool;
    /// True only when the window's app is hidden or the window is minimized.
    fn confirms_set_aside(&mut self, window: &crate::PhysicalWindowId) -> bool;
    fn apply_observed(
        &mut self,
        placements: &[crate::Placement],
        settings: &Settings,
        cancelled: &AtomicBool,
    ) -> Vec<macos::ApplyResult> {
        self.apply_managed(placements, settings, cancelled)
    }
    fn frame(
        &mut self,
        window: &crate::PhysicalWindowId,
    ) -> Result<crate::Rect, macos::WindowAccessError>;
    /// Authoritative window-server frame, immune to stale Accessibility reads.
    fn window_server_frame(&mut self, window: &crate::PhysicalWindowId) -> Option<crate::Rect>;
    /// Authoritative window-server frames for all windows in one snapshot.
    /// Use for per-cycle checks instead of repeated single-window queries.
    fn window_server_frames(&mut self) -> Option<HashMap<u32, crate::Rect>>;
    /// Window-server frames for windows currently on screen; proves a window
    /// still exists when an Accessibility snapshot omits it.
    fn onscreen_window_frames(&mut self) -> Option<HashMap<u32, crate::Rect>>;
    /// On-screen window-server IDs, front-most first.
    fn onscreen_window_order(&mut self) -> Option<Vec<u32>>;
    fn set_position(
        &mut self,
        window: &crate::PhysicalWindowId,
        target: crate::Rect,
    ) -> macos::ApplyOutcome;
    /// Move `window` to a parked position; see [`macos::set_parked_position`].
    fn set_parked_position(
        &mut self,
        window: &crate::PhysicalWindowId,
        target: crate::Rect,
    ) -> macos::ApplyOutcome {
        self.set_position(window, target)
    }
    fn focused_window(&mut self) -> Option<crate::PhysicalWindowId>;
    fn frontmost_application(&mut self) -> Option<AppInstanceId>;
    /// Diagnostics only: recent user input timing for focus-follow logs.
    fn recent_input(&mut self) -> Option<macos::RecentInput> {
        None
    }
    fn raise_window(&mut self, window: &crate::PhysicalWindowId) -> bool;
    fn focus_window(&mut self, window: &crate::PhysicalWindowId) -> bool;
    fn pointer_location(&mut self) -> Option<(i32, i32)> {
        None
    }
    fn mouse_down(&mut self) -> bool {
        false
    }
    fn warp_pointer(&mut self, _point: (i32, i32)) -> bool {
        false
    }
    fn apply_managed(
        &mut self,
        placements: &[crate::Placement],
        settings: &Settings,
        cancelled: &AtomicBool,
    ) -> Vec<macos::ApplyResult>;
    fn apply_managed_during_drag(
        &mut self,
        placements: &[crate::Placement],
        settings: &Settings,
        dragged: &crate::PhysicalWindowId,
        cancelled: &AtomicBool,
    ) -> Vec<macos::ApplyResult>;
    fn persist_parked(
        &mut self,
        parked: &HashMap<crate::PhysicalWindowId, crate::Rect>,
    ) -> Result<(), String>;
    fn persist_workspace_state(
        &mut self,
        state: &crate::workspace::WorkspaceState,
    ) -> Result<(), String>;
    fn persist_window_memory(
        &mut self,
        memory: &crate::window_memory::WindowMemory,
    ) -> Result<(), String>;
    fn persist_workspace_monitor(
        &mut self,
        workspace: WorkspaceId,
        selector: &DisplaySelector,
    ) -> Result<(), String>;
    /// Best-effort diagnostics; a journal failure never affects management.
    fn journal(&mut self, line: &crate::journal::JournalLine);
    /// Waits briefly for state written in the background to reach the disk.
    fn flush_state(&mut self) {}
}

pub(super) struct MacWorkspaceBackend {
    writer: StateWriter,
}

impl MacWorkspaceBackend {
    pub(super) fn new() -> Self {
        Self {
            writer: StateWriter::spawn(),
        }
    }

    fn write_later(
        &self,
        path: Option<PathBuf>,
        value: &impl serde::Serialize,
        state: &'static str,
    ) -> Result<(), String> {
        let path = path.ok_or("HOME is unavailable")?;
        let bytes = serde_json::to_vec(value)
            .map_err(|error| format!("failed to serialize {state} state: {error}"))?;
        self.writer.write(path, bytes, state);
        Ok(())
    }
}

impl WorkspaceBackend for MacWorkspaceBackend {
    fn snapshot(
        &mut self,
        cycle: u64,
        outputs: &macos::DisplayOutputs,
    ) -> Result<crate::NativeObservation, String> {
        macos::snapshot_in_outputs(cycle, outputs).map_err(|error| error.to_string())
    }

    fn confirms_no_windows(&mut self, app: &AppInstanceId) -> bool {
        macos::confirms_no_windows(app)
    }

    fn confirms_set_aside(&mut self, window: &crate::PhysicalWindowId) -> bool {
        macos::confirms_set_aside(window)
    }

    fn is_process_running(&mut self, app: &AppInstanceId) -> bool {
        macos::is_process_incarnation_running(app)
    }

    fn bundle_id(&mut self, app: &AppInstanceId) -> Option<String> {
        macos::application_bundle_id(app)
    }

    fn frame(
        &mut self,
        window: &crate::PhysicalWindowId,
    ) -> Result<crate::Rect, macos::WindowAccessError> {
        macos::frame(window)
    }

    fn window_server_frame(&mut self, window: &crate::PhysicalWindowId) -> Option<crate::Rect> {
        macos::window_server_frame(window)
    }

    fn window_server_frames(&mut self) -> Option<HashMap<u32, crate::Rect>> {
        macos::window_server_frames()
    }

    fn onscreen_window_frames(&mut self) -> Option<HashMap<u32, crate::Rect>> {
        macos::onscreen_window_frames()
    }

    fn onscreen_window_order(&mut self) -> Option<Vec<u32>> {
        macos::onscreen_window_order()
    }

    fn set_position(
        &mut self,
        window: &crate::PhysicalWindowId,
        target: crate::Rect,
    ) -> macos::ApplyOutcome {
        macos::set_position_only(window, target)
    }

    fn set_parked_position(
        &mut self,
        window: &crate::PhysicalWindowId,
        target: crate::Rect,
    ) -> macos::ApplyOutcome {
        macos::set_parked_position(window, target)
    }

    fn focused_window(&mut self) -> Option<crate::PhysicalWindowId> {
        macos::focused_window()
    }

    fn frontmost_application(&mut self) -> Option<AppInstanceId> {
        macos::frontmost_application()
    }

    fn recent_input(&mut self) -> Option<macos::RecentInput> {
        Some(macos::recent_input())
    }

    fn raise_window(&mut self, window: &crate::PhysicalWindowId) -> bool {
        macos::raise_window(window)
    }

    fn focus_window(&mut self, window: &crate::PhysicalWindowId) -> bool {
        macos::focus_window(window)
    }

    fn pointer_location(&mut self) -> Option<(i32, i32)> {
        macos::mouse_position()
    }

    fn mouse_down(&mut self) -> bool {
        macos::mouse_down()
    }

    fn warp_pointer(&mut self, point: (i32, i32)) -> bool {
        macos::warp_pointer(point)
    }

    fn apply_managed(
        &mut self,
        placements: &[crate::Placement],
        settings: &Settings,
        cancelled: &AtomicBool,
    ) -> Vec<macos::ApplyResult> {
        macos::apply_managed(placements, settings, None, || {
            !cancelled.load(Ordering::Acquire)
        })
    }

    fn apply_managed_during_drag(
        &mut self,
        placements: &[crate::Placement],
        settings: &Settings,
        dragged: &crate::PhysicalWindowId,
        cancelled: &AtomicBool,
    ) -> Vec<macos::ApplyResult> {
        macos::apply_managed(placements, settings, Some(dragged), || {
            !cancelled.load(Ordering::Acquire)
        })
    }

    fn persist_parked(
        &mut self,
        parked: &HashMap<crate::PhysicalWindowId, crate::Rect>,
    ) -> Result<(), String> {
        persist_parked(parked)
    }

    fn persist_workspace_state(
        &mut self,
        state: &crate::workspace::WorkspaceState,
    ) -> Result<(), String> {
        self.write_later(workspace_state_path(), state, "workspace")
    }

    fn persist_window_memory(
        &mut self,
        memory: &crate::window_memory::WindowMemory,
    ) -> Result<(), String> {
        self.write_later(window_memory_path(), memory, "window memory")
    }

    fn persist_workspace_monitor(
        &mut self,
        workspace: WorkspaceId,
        selector: &DisplaySelector,
    ) -> Result<(), String> {
        let path = crate::config::config_path().ok_or("HOME is unavailable")?;
        crate::config::save_workspace_monitor_at(&path, workspace, selector).map(|_| ())
    }

    fn journal(&mut self, line: &crate::journal::JournalLine) {
        static FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let result = crate::journal::default_paths()
            .ok_or_else(|| "HOME is unavailable".to_owned())
            .and_then(|(path, rotated)| crate::journal::append(&path, &rotated, line));
        match result {
            Ok(()) => FAILED.store(false, Ordering::Relaxed),
            // Log the first failure of a run, not every event.
            Err(error) if !FAILED.swap(true, Ordering::Relaxed) => {
                log_event!("arrangement journal unavailable: {error}");
            }
            Err(_) => {}
        }
    }

    fn flush_state(&mut self) {
        if !self.writer.flush(STATE_FLUSH_TIMEOUT) {
            log_event!("state writes still pending after {STATE_FLUSH_TIMEOUT:?}");
        }
    }
}

const STATE_FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

fn window_memory_path() -> Option<PathBuf> {
    crate::config::application_support_path("window-memory.json")
}

/// Window memory is advisory: unreadable or invalid memory is logged and
/// replaced by an empty memory rather than blocking startup or recovery.
pub(super) fn load_window_memory() -> crate::window_memory::WindowMemory {
    let Some(path) = window_memory_path() else {
        return crate::window_memory::WindowMemory::default();
    };
    let memory = load_json::<crate::window_memory::WindowMemory>(&path, "window memory").and_then(
        |memory| {
            let memory = memory.unwrap_or_default();
            memory
                .validate()
                .map(|()| memory)
                .map_err(|error| format!("window memory is invalid: {error}"))
        },
    );
    memory.unwrap_or_else(|error| {
        log_event!("ignoring {error}");
        crate::window_memory::WindowMemory::default()
    })
}

fn parked_path() -> Option<PathBuf> {
    crate::config::application_support_path("parked-windows.json")
}

fn workspace_state_path() -> Option<PathBuf> {
    crate::config::application_support_path("workspace-state.json")
}

pub(super) fn load_parked() -> Result<HashMap<crate::PhysicalWindowId, crate::Rect>, String> {
    let path = parked_path().ok_or("HOME is unavailable")?;
    load_json::<ParkedWindows>(&path, "parked-window")
        .map(|parked| parked.map_or_else(HashMap::new, |parked| parked.0))
}

fn persist_parked(parked: &HashMap<crate::PhysicalWindowId, crate::Rect>) -> Result<(), String> {
    let path = parked_path().ok_or("HOME is unavailable")?;
    let started = Instant::now();
    let result = persist_json(&path, &ParkedWindowsRef(parked), "parked-window");
    let elapsed = started.elapsed();
    if elapsed >= Duration::from_millis(500) {
        log_event!("slow parked-window state write: {}ms", elapsed.as_millis());
    }
    result
}

pub(super) fn load_workspace_state() -> Result<Option<crate::workspace::WorkspaceState>, String> {
    let path = workspace_state_path().ok_or("HOME is unavailable")?;
    let state = load_json::<crate::workspace::WorkspaceState>(&path, "workspace")?;
    if let Some(state) = &state {
        state
            .validate()
            .map_err(|error| format!("workspace state is invalid: {error}"))?;
    }
    Ok(state)
}

fn load_json<T: serde::de::DeserializeOwned>(
    path: &std::path::Path,
    state: &str,
) -> Result<Option<T>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to read {state} state: {error}")),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("{state} state is invalid: {error}"))
}

pub(super) fn recover_startup_metadata(
    config: Result<Config, String>,
    workspace_state: Result<Option<crate::workspace::WorkspaceState>, String>,
) -> (Config, Option<crate::workspace::WorkspaceState>) {
    let config = match config {
        Ok(config) => config,
        Err(error) => {
            log_event!("ignoring invalid Spaceman config during recovery: {error}");
            return (Config::default(), None);
        }
    };
    let workspace_state = workspace_state.unwrap_or_else(|error| {
        log_event!("ignoring invalid Spaceman workspace state during recovery: {error}");
        None
    });
    (config, workspace_state)
}

fn persist_json(
    path: &std::path::Path,
    value: &impl serde::Serialize,
    state: &str,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("failed to serialize {state} state: {error}"))?;
    crate::state_file::write_atomic(path, &bytes)
        .map_err(|error| format!("failed to write {state} state: {error}"))
}
