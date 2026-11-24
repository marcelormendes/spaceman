use std::sync::{Arc, Mutex, mpsc::Sender};

use crate::{Axis, Rect, WorkspaceId, login_item};

use super::{Action, WorkerAction};

/// Normalized preview coordinate space for HUD split rectangles.
pub const HUD_PREVIEW_RECT: Rect = Rect {
    x: 0,
    y: 0,
    width: 1500,
    height: 1100,
};

/// Actions the in-process Rune window or HUD overlay can send to the agent loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiAction {
    ToggleAutomatic,
    /// The first-run setup tour finished: automatic tiling may start.
    FinishSetup,
    OpenSettings,
    OpenAccessibility,
    CheckForUpdates,
    /// Restore every window, then install the staged update and relaunch.
    InstallUpdate,
    /// Switch to the update channel's release on offer.
    ConfirmUpdateSwitch,
    /// Open the release's download, for a copy that can't update itself.
    DownloadUpdate,
    ToggleLaunchAtLogin,
    SetHud(bool),
    SwitchWorkspace(WorkspaceId),
    Retile,
    MarkWrong,
}

impl From<UiAction> for Action {
    fn from(action: UiAction) -> Self {
        match action {
            UiAction::ToggleAutomatic => Self::ToggleAutomatic,
            UiAction::FinishSetup => Self::FinishSetup,
            UiAction::OpenSettings => Self::OpenSettings,
            UiAction::OpenAccessibility => Self::OpenAccessibility,
            UiAction::CheckForUpdates => Self::CheckForUpdates,
            UiAction::InstallUpdate => Self::InstallUpdate,
            UiAction::ConfirmUpdateSwitch => Self::ConfirmUpdateSwitch,
            UiAction::DownloadUpdate => Self::DownloadUpdate,
            UiAction::ToggleLaunchAtLogin => Self::ToggleLaunchAtLogin,
            UiAction::SetHud(enabled) => Self::SetHud(enabled),
            UiAction::SwitchWorkspace(workspace) => {
                Self::Worker(WorkerAction::SelectWorkspace(workspace))
            }
            UiAction::Retile => Self::Worker(WorkerAction::Retile),
            UiAction::MarkWrong => Self::Worker(WorkerAction::MarkWrong),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiHudKind {
    Workspace,
    Moved,
    Layout { axis: Axis },
    Fullscreen { enabled: bool },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiHudLayout {
    Split(Vec<Rect>),
    Fullscreen,
}

/// The most displays one HUD event describes.
pub const HUD_MAX_DISPLAYS: usize = 8;
/// The most tiles one HUD workspace carries.
pub const HUD_MAX_TILES: usize = 16;
/// The most floating apps one HUD workspace carries.
pub const HUD_MAX_FLOATING: usize = 8;

/// Title-free Workspace HUD event emitted on explicit user actions: every display's row of
/// workspaces, with their layouts and the bundle IDs of the apps in them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiHudEvent {
    /// The workspace the action was on: switched to, moved to, or relaid.
    pub workspace: WorkspaceId,
    pub kind: UiHudKind,
    pub displays: Vec<UiHudDisplay>,
}

/// One display's row: its visible workspace and every other one holding windows, in order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiHudDisplay {
    /// The display's ID, as `DisplayOutput::id` gives it.
    pub id: String,
    pub visible: Option<WorkspaceId>,
    /// The workspace a switch on this display just left, so the HUD can start from it.
    pub from: Option<WorkspaceId>,
    pub workspaces: Vec<UiHudWorkspace>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiHudWorkspace {
    pub workspace: WorkspaceId,
    /// Fullscreen Layout: the first tile is the window in front.
    pub fullscreen: bool,
    /// Tiles in layout order.
    pub tiles: Vec<UiHudTile>,
    /// Bundle IDs of the floating windows the workspace owns, once each.
    pub floating: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiHudTile {
    /// The tile's slot, stable while the window keeps its place.
    pub key: u64,
    /// In [`HUD_PREVIEW_RECT`] space.
    pub rect: Rect,
    pub bundle: Option<String>,
}

impl UiHudEvent {
    /// The display whose row holds `workspace`.
    #[must_use]
    pub fn display_of(&self, workspace: WorkspaceId) -> Option<&UiHudDisplay> {
        self.displays.iter().find(|display| {
            display
                .workspaces
                .iter()
                .any(|entry| entry.workspace == workspace)
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum UiLaunchAtLogin {
    #[default]
    Off,
    On,
    NeedsApproval,
    Failed(String),
}

impl UiLaunchAtLogin {
    pub(super) fn from_result(result: Result<login_item::Status, String>) -> Self {
        match result {
            Ok(login_item::Status::Enabled) => Self::On,
            Ok(login_item::Status::Disabled) => Self::Off,
            Ok(login_item::Status::RequiresApproval) => Self::NeedsApproval,
            Err(error) => Self::Failed(error),
        }
    }

    pub(super) fn as_settings_value(&self) -> String {
        match self {
            Self::On => "enabled".into(),
            Self::Off => "disabled".into(),
            Self::NeedsApproval => "requires-approval".into(),
            Self::Failed(error) => format!("error: {error}"),
        }
    }
}

/// What the in-app updater is doing (`crate::updater`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum UiUpdateStatus {
    /// Updates are off for this copy: turned off, or not in a `Spaceman.app`.
    #[default]
    Unavailable,
    /// A build from source, which its checkout updates.
    SourceBuild,
    Idle,
    Checking,
    UpToDate,
    Downloading,
    /// A verified update waits for Install & Restart.
    Ready {
        version: String,
    },
    /// The update channel setting names another channel, whose release waits
    /// for the user to switch to it.
    Switch {
        channel: String,
        version: String,
    },
    /// A release this copy can't install itself (a read-only or translocated
    /// location); the user downloads it.
    Download {
        version: String,
    },
    /// Windows are restored and the update is swapped in; Spaceman relaunches.
    Installing,
    Failed,
}

impl UiUpdateStatus {
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::SourceBuild => "source-build",
            Self::Idle => "idle",
            Self::Checking => "checking",
            Self::UpToDate => "up-to-date",
            Self::Downloading => "downloading",
            Self::Ready { .. } => "ready",
            Self::Switch { .. } => "switch-available",
            Self::Download { .. } => "download-available",
            Self::Installing => "installing",
            Self::Failed => "failed",
        }
    }

    /// The version a ready update installs.
    #[must_use]
    pub fn ready_version(&self) -> Option<&str> {
        match self {
            Self::Ready { version } => Some(version),
            _ => None,
        }
    }

    /// Whether a manual check may start now.
    #[must_use]
    pub const fn can_check(&self) -> bool {
        matches!(
            self,
            Self::Idle
                | Self::UpToDate
                | Self::Failed
                | Self::Ready { .. }
                | Self::Switch { .. }
                | Self::Download { .. }
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UiAgentSnapshot {
    pub accessibility: bool,
    pub automatic: bool,
    pub hud: bool,
    pub launch_at_login: UiLaunchAtLogin,
    pub update: UiUpdateStatus,
    pub active_workspaces: Vec<WorkspaceId>,
    pub last_error: Option<String>,
}

impl UiAgentSnapshot {
    pub(super) fn to_settings_json(&self) -> serde_json::Value {
        serde_json::json!({
            "version": crate::updater::build_version(),
            "accessibility": self.accessibility,
            "automatic": self.automatic,
            "hud": self.hud,
            "launch_at_login": self.launch_at_login.as_settings_value(),
            "update": self.update.as_str(),
            "update_version": self.update.ready_version(),
            "active_workspaces": self.active_workspaces,
            "last_error": self.last_error,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UiNews {
    pub snapshot: Option<UiAgentSnapshot>,
    pub hud: Option<UiHudEvent>,
    pub config_changed: bool,
}

#[derive(Default)]
struct SharedState {
    last: Option<UiAgentSnapshot>,
    pending_snapshot: Option<UiAgentSnapshot>,
    pending_hud: Option<UiHudEvent>,
    config_changed: bool,
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// Thread-safe bridge between the main-thread agent pump and the Rune window.
#[derive(Clone)]
pub struct UiBridge {
    actions: Sender<Action>,
    shared: Arc<Mutex<SharedState>>,
}

impl UiBridge {
    pub(super) fn new(actions: Sender<Action>) -> Self {
        Self {
            actions,
            shared: Arc::new(Mutex::new(SharedState::default())),
        }
    }

    pub fn set_waker(&self, waker: Arc<dyn Fn() + Send + Sync>) {
        let mut shared = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if shared.pending_snapshot.is_none() {
            shared.pending_snapshot = shared.last.clone();
        }
        let has_news = shared.pending_snapshot.is_some()
            || shared.pending_hud.is_some()
            || shared.config_changed;
        shared.waker = Some(Arc::clone(&waker));
        drop(shared);
        if has_news {
            waker();
        }
    }

    #[must_use]
    pub fn take_news(&self) -> UiNews {
        let mut shared = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        UiNews {
            snapshot: shared.pending_snapshot.take(),
            hud: shared.pending_hud.take(),
            config_changed: std::mem::take(&mut shared.config_changed),
        }
    }

    pub fn send(&self, action: UiAction) {
        drop(self.actions.send(action.into()));
    }

    pub(super) fn publish_hud(&self, event: UiHudEvent) {
        let waker = {
            let mut shared = self
                .shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            shared.pending_hud = Some(event);
            shared.waker.clone()
        };
        if let Some(waker) = waker {
            waker();
        }
    }

    pub(super) fn publish(&self, snapshot: UiAgentSnapshot, config_changed: bool) {
        let waker = {
            let mut shared = self
                .shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let snapshot_changed = shared.last.as_ref() != Some(&snapshot);
            if !snapshot_changed && !config_changed {
                return;
            }
            if snapshot_changed {
                shared.last = Some(snapshot.clone());
                shared.pending_snapshot = Some(snapshot);
            }
            shared.config_changed |= config_changed;
            shared.waker.clone()
        };
        if let Some(waker) = waker {
            waker();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::channel,
    };

    use super::*;

    #[test]
    fn deduplicates_identical_snapshots_and_wakes_late_subscriber() {
        let (sender, receiver) = channel();
        let bridge = UiBridge::new(sender);
        let wakes = Arc::new(AtomicUsize::new(0));

        let initial = UiAgentSnapshot {
            accessibility: true,
            automatic: true,
            hud: true,
            launch_at_login: UiLaunchAtLogin::On,
            update: UiUpdateStatus::Idle,
            active_workspaces: vec![WorkspaceId::ONE],
            last_error: None,
        };
        bridge.publish(initial.clone(), false);

        let wakes_for_hook = Arc::clone(&wakes);
        bridge.set_waker(Arc::new(move || {
            wakes_for_hook.fetch_add(1, Ordering::SeqCst);
        }));
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert_eq!(
            bridge.take_news(),
            UiNews {
                snapshot: Some(initial.clone()),
                hud: None,
                config_changed: false,
            }
        );

        // Identical snapshot without config change does not wake again.
        bridge.publish(initial.clone(), false);
        assert_eq!(wakes.load(Ordering::SeqCst), 1);

        // Config change wakes even when snapshot is unchanged.
        bridge.publish(initial, true);
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
        assert_eq!(
            bridge.take_news(),
            UiNews {
                snapshot: None,
                hud: None,
                config_changed: true,
            }
        );

        let hud_event = UiHudEvent {
            workspace: WorkspaceId::ONE,
            kind: UiHudKind::Workspace,
            displays: Vec::new(),
        };
        bridge.publish_hud(hud_event.clone());
        assert_eq!(wakes.load(Ordering::SeqCst), 3);
        assert_eq!(
            bridge.take_news(),
            UiNews {
                snapshot: None,
                hud: Some(hud_event),
                config_changed: false,
            }
        );

        bridge.send(UiAction::ToggleAutomatic);
        assert_eq!(receiver.try_recv().ok(), Some(Action::ToggleAutomatic));
        bridge.send(UiAction::OpenSettings);
        assert_eq!(receiver.try_recv().ok(), Some(Action::OpenSettings));
        bridge.send(UiAction::SwitchWorkspace(WorkspaceId::THREE));
        assert_eq!(
            receiver.try_recv().ok(),
            Some(Action::Worker(WorkerAction::SelectWorkspace(
                WorkspaceId::THREE
            )))
        );
        bridge.send(UiAction::Retile);
        assert_eq!(
            receiver.try_recv().ok(),
            Some(Action::Worker(WorkerAction::Retile))
        );
    }

    #[test]
    fn encodes_snapshot_as_control_settings_json() {
        let snapshot = UiAgentSnapshot {
            accessibility: true,
            automatic: false,
            hud: false,
            launch_at_login: UiLaunchAtLogin::NeedsApproval,
            update: UiUpdateStatus::Checking,
            active_workspaces: vec![WorkspaceId::THREE, WorkspaceId::named('M').unwrap()],
            last_error: Some("Hotkeys unavailable".into()),
        };
        let value = snapshot.to_settings_json();
        assert_eq!(value["accessibility"], true);
        assert_eq!(value["automatic"], false);
        assert_eq!(value["hud"], false);
        assert_eq!(value["launch_at_login"], "requires-approval");
        assert_eq!(value["update"], "checking");
        assert_eq!(value["active_workspaces"], serde_json::json!([3, "M"]));
        assert_eq!(value["last_error"], "Hotkeys unavailable");
    }
}
