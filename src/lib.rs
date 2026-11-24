#[cfg(target_os = "macos")]
pub mod app;
pub mod config;
#[cfg(target_os = "macos")]
pub mod control;
pub mod deep_link;
pub mod geometry;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod hotkeys;
pub mod journal;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod keyboard_layout;
mod layout_tree;
#[cfg(target_os = "macos")]
pub mod live;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod login_item;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub mod macos;
pub mod model;
pub mod motion;
pub mod navigation;
pub mod observation;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub mod permission_guide;
pub mod reconcile;
pub mod runtime;
pub mod scratchpad;
mod serde_map;
pub mod shortcuts;
pub mod spacing;
#[allow(unsafe_code)]
pub mod state_file;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod status_item;
pub mod swipe;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod termination;
#[cfg(target_os = "macos")]
mod updater;
pub use updater::build_version;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod url_handler;
pub mod window_memory;
pub mod workspace;

pub use geometry::{Axis, Rect};
pub use model::{AppInstanceId, BindingState, Model, PhysicalWindowId, Slot, SlotId};
pub use navigation::{Direction, NavigationTile};
pub use observation::{AppObservation, NativeObservation, ObservedApplication, ObservedWindow};
pub use reconcile::{
    Decision, Placement, Reconciliation, ReconciliationEvidence, Settings, reconcile,
};
pub use runtime::{CycleTicket, Engine, StaleCycle};
pub use workspace::{DisplaySelector, DisplayTarget, WorkspaceId, Workspaces};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResizeDirection {
    Shrink,
    Grow,
}
