use std::{
    collections::{HashMap, HashSet},
    fmt,
    str::FromStr,
};

use serde::{Deserialize, Serialize};

use crate::{Engine, PhysicalWindowId};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkspaceId(u8);

impl WorkspaceId {
    pub const NUMBERED_COUNT: usize = 10;
    pub const COUNT: usize = Self::NUMBERED_COUNT + 26;
    pub const ONE: Self = Self(1);
    pub const TWO: Self = Self(2);
    pub const THREE: Self = Self(3);
    pub const FOUR: Self = Self(4);
    pub const SIX: Self = Self(6);

    pub fn new(value: u8) -> Result<Self, String> {
        (1..=Self::NUMBERED_COUNT as u8)
            .contains(&value)
            .then_some(Self(value))
            .ok_or_else(|| format!("workspace must be 1 through 10 or A through Z, got {value}"))
    }

    pub fn named(letter: char) -> Result<Self, String> {
        let letter = letter.to_ascii_uppercase();
        letter
            .is_ascii_uppercase()
            .then_some(Self(11 + (letter as u8 - b'A')))
            .ok_or_else(|| format!("workspace must be 1 through 10 or A through Z, got {letter:?}"))
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        value.parse::<u8>().ok().map(Self::new).unwrap_or_else(|| {
            let mut characters = value.chars();
            match (characters.next(), characters.next()) {
                (Some(letter), None) => Self::named(letter),
                _ => Err(format!(
                    "workspace must be 1 through 10 or A through Z, got {value:?}"
                )),
            }
        })
    }

    #[must_use]
    pub const fn number(self) -> Option<u8> {
        if self.0 <= Self::NUMBERED_COUNT as u8 {
            Some(self.0)
        } else {
            None
        }
    }

    #[must_use]
    pub const fn letter(self) -> Option<char> {
        if self.0 > Self::NUMBERED_COUNT as u8 {
            Some((b'A' + self.0 - 11) as char)
        } else {
            None
        }
    }

    pub fn all() -> impl Iterator<Item = Self> {
        (0..Self::COUNT).map(Self::from_index)
    }

    pub fn numbered() -> impl Iterator<Item = Self> {
        Self::all().take(Self::NUMBERED_COUNT)
    }

    pub(crate) fn from_index(index: usize) -> Self {
        Self(
            u8::try_from(index.saturating_add(1))
                .unwrap_or(Self::COUNT as u8)
                .clamp(1, Self::COUNT as u8),
        )
    }

    #[must_use]
    pub(crate) const fn index(self) -> usize {
        (self.0 - 1) as usize
    }

    const fn named_letter(self) -> char {
        (b'A' + self.0.saturating_sub(11)) as char
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.number() {
            Some(number) => write!(formatter, "{number}"),
            None => write!(formatter, "{}", self.named_letter()),
        }
    }
}

impl FromStr for WorkspaceId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for WorkspaceId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.number() {
            Some(number) => serializer.serialize_u8(number),
            None => serializer.serialize_char(self.named_letter()),
        }
    }
}

impl<'de> Deserialize<'de> for WorkspaceId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Value {
            Number(u8),
            Name(String),
        }
        match Value::deserialize(deserializer)? {
            Value::Number(value) => Self::new(value),
            Value::Name(value) => Self::parse(&value),
        }
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum DisplaySelector {
    Main,
    Secondary,
    Preferred(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DisplayTarget {
    Next,
    Prev,
    Selector(DisplaySelector),
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct WorkspaceState {
    active: Vec<(DisplaySelector, WorkspaceId)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    previous: Vec<(DisplaySelector, WorkspaceId)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    axes: Vec<crate::Axis>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    layouts: Vec<crate::layout_tree::LayoutTree>,
    slots: Vec<Vec<crate::Slot>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    floating: Vec<(crate::PhysicalWindowId, WorkspaceId)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    last_focused: Vec<Option<crate::SlotId>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    fullscreen_layout: Vec<bool>,
    /// Legacy per-window maximize intent. A populated entry migrates the
    /// workspace into fullscreen layout mode.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    maximized: Vec<Option<crate::SlotId>>,
}

impl WorkspaceState {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.slots.len() > WorkspaceId::COUNT
            || self.layouts.len() > WorkspaceId::COUNT
            || self.axes.len() > WorkspaceId::COUNT
            || self.last_focused.len() > WorkspaceId::COUNT
            || self.fullscreen_layout.len() > WorkspaceId::COUNT
            || self.maximized.len() > WorkspaceId::COUNT
        {
            return Err("workspace state contains more than 36 workspaces".into());
        }
        for (index, slots) in self.slots.iter().enumerate() {
            let next_slot_id = slots
                .iter()
                .map(|slot| slot.id.0)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or("workspace slot ID cannot be incremented")?;
            crate::Model {
                workspace: crate::model::Workspace {
                    axis: self.axes.get(index).copied().unwrap_or_default(),
                    slots: slots.clone(),
                },
                continuity: HashMap::new(),
                next_slot_id,
            }
            .validate()
            .map_err(|error| format!("workspace {}: {error}", index + 1))?;
            if let Some(layout) = self.layouts.get(index) {
                layout
                    .validate(slots)
                    .map_err(|error| format!("workspace {}: {error}", index + 1))?;
            }
        }
        if self.layouts.len() > self.slots.len() {
            return Err("workspace layouts have no matching slot state".into());
        }
        for (name, intents) in [
            ("last-focused", &self.last_focused),
            ("maximized", &self.maximized),
        ] {
            for (index, slot) in intents.iter().enumerate() {
                if let Some(slot) = slot
                    && !self
                        .slots
                        .get(index)
                        .is_some_and(|slots| slots.iter().any(|candidate| candidate.id == *slot))
                {
                    return Err(format!(
                        "workspace {} {name} slot {} does not exist",
                        index + 1,
                        slot.0
                    ));
                }
            }
        }
        let mut tiled = HashSet::new();
        for window in self
            .slots
            .iter()
            .flatten()
            .map(crate::Slot::identity_window)
        {
            if !tiled.insert(window) {
                return Err("tiled window is owned by multiple workspaces".into());
            }
        }
        let mut floating = HashSet::new();
        for (window, _) in &self.floating {
            if tiled.contains(window) {
                return Err("window is both tiled and floating".into());
            }
            if !floating.insert(window) {
                return Err("floating window is owned by multiple workspaces".into());
            }
        }
        Ok(())
    }

    pub(crate) fn named_workspaces_in_use(&self) -> HashSet<WorkspaceId> {
        let mut used = self
            .active
            .iter()
            .chain(&self.previous)
            .map(|(_, workspace)| *workspace)
            .chain(self.floating.iter().map(|(_, workspace)| *workspace))
            .filter(|workspace| workspace.letter().is_some())
            .collect::<HashSet<_>>();
        for (index, slots) in self.slots.iter().enumerate() {
            let workspace = WorkspaceId::from_index(index);
            if workspace.letter().is_some() && !slots.is_empty() {
                used.insert(workspace);
            }
        }
        used
    }
}

impl DisplaySelector {
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "main" => Self::Main,
            "secondary" => Self::Secondary,
            value => Self::Preferred(value.to_owned()),
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Main => "main",
            Self::Secondary => "secondary",
            Self::Preferred(value) => value.as_str(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Workspaces {
    engines: [Engine; WorkspaceId::COUNT],
    assignments: [DisplaySelector; WorkspaceId::COUNT],
    active: HashMap<DisplaySelector, WorkspaceId>,
    previous: HashMap<DisplaySelector, WorkspaceId>,
    floating_owners: HashMap<PhysicalWindowId, WorkspaceId>,
    enabled: HashSet<WorkspaceId>,
    last_focused: [Option<crate::SlotId>; WorkspaceId::COUNT],
    fullscreen_layout: [bool; WorkspaceId::COUNT],
}

impl Default for Workspaces {
    fn default() -> Self {
        Self {
            engines: std::array::from_fn(|_| Engine::default()),
            assignments: std::array::from_fn(|index| {
                Self::default_assignment(WorkspaceId::from_index(index))
            }),
            active: HashMap::from([
                (DisplaySelector::Main, WorkspaceId::ONE),
                (DisplaySelector::Secondary, WorkspaceId::SIX),
            ]),
            previous: HashMap::new(),
            floating_owners: HashMap::new(),
            enabled: WorkspaceId::numbered().collect(),
            last_focused: std::array::from_fn(|_| None),
            fullscreen_layout: [false; WorkspaceId::COUNT],
        }
    }
}

impl Workspaces {
    #[must_use]
    pub(crate) fn state(&self) -> WorkspaceState {
        let workspace_count = self
            .engines
            .iter()
            .rposition(|engine| !engine.model().workspace.slots.is_empty())
            .map_or(WorkspaceId::NUMBERED_COUNT, |index| index + 1)
            .max(WorkspaceId::NUMBERED_COUNT);
        WorkspaceState {
            active: sorted(self.active.clone()),
            previous: sorted(self.previous.clone()),
            axes: Vec::new(),
            layouts: self
                .engines
                .iter()
                .take(workspace_count)
                .map(|engine| engine.layout().clone())
                .collect(),
            slots: self
                .engines
                .iter()
                .take(workspace_count)
                .map(|engine| engine.model().workspace.slots.clone())
                .collect(),
            floating: sorted(self.floating_owners.clone()),
            last_focused: trimmed(&self.last_focused, Option::is_some),
            fullscreen_layout: trimmed(&self.fullscreen_layout, |enabled| *enabled),
            maximized: Vec::new(),
        }
    }

    pub(crate) fn restore(&mut self, state: WorkspaceState) {
        self.enabled.extend(state.named_workspaces_in_use());
        let WorkspaceState {
            active,
            previous,
            axes,
            layouts,
            slots,
            floating,
            last_focused,
            fullscreen_layout,
            maximized,
        } = state;
        for (index, slots) in slots.into_iter().take(WorkspaceId::COUNT).enumerate() {
            let workspace = WorkspaceId::from_index(index);
            for mut slot in slots {
                if let crate::BindingState::Unresolved { since, .. } = &mut slot.binding {
                    // Reconciliation cycles are process-local, so restart the
                    // bounded settling window when restoring persisted state.
                    *since = 0;
                }
                self.engine_mut(workspace).insert_slot(slot);
            }
            let layout = layouts.get(index).cloned().unwrap_or_else(|| {
                crate::layout_tree::LayoutTree::weighted_flat(
                    axes.get(index).copied().unwrap_or_default(),
                    self.engine(workspace).model().workspace.slots.iter(),
                )
            });
            self.engine_mut(workspace).restore_layout(layout);
        }
        for (selector, workspace) in active {
            if self.assignment(workspace) == &selector {
                self.active.insert(selector, workspace);
            }
        }
        for (selector, workspace) in previous {
            if self.assignment(workspace) == &selector
                && self.active_for(&selector) != Some(workspace)
            {
                self.previous.insert(selector, workspace);
            }
        }
        for (window, workspace) in floating {
            if self.workspace_of(&window).is_none() {
                self.floating_owners.insert(window, workspace);
            }
        }
        for index in 0..WorkspaceId::COUNT {
            let workspace = WorkspaceId::from_index(index);
            let valid_slot = |slots: &[Option<crate::SlotId>]| {
                slots
                    .get(index)
                    .copied()
                    .flatten()
                    .filter(|slot| self.has_slot(workspace, *slot))
            };
            let legacy_maximized = valid_slot(&maximized);
            self.last_focused[index] = legacy_maximized.or_else(|| valid_slot(&last_focused));
            self.set_fullscreen_layout(
                workspace,
                fullscreen_layout.get(index).copied().unwrap_or(false)
                    || legacy_maximized.is_some(),
            );
        }
    }

    #[must_use]
    pub fn default_assignment(workspace: WorkspaceId) -> DisplaySelector {
        match workspace.number() {
            Some(number) if number >= 6 => DisplaySelector::Secondary,
            _ => DisplaySelector::Main,
        }
    }

    #[must_use]
    pub fn assignment(&self, workspace: WorkspaceId) -> &DisplaySelector {
        &self.assignments[workspace.index()]
    }

    #[must_use]
    pub fn active_for(&self, display: &DisplaySelector) -> Option<WorkspaceId> {
        self.active.get(display).copied()
    }

    pub fn active_lanes(&self) -> impl Iterator<Item = (&DisplaySelector, WorkspaceId)> {
        self.active
            .iter()
            .map(|(display, workspace)| (display, *workspace))
    }

    pub fn set_assignment(&mut self, workspace: WorkspaceId, display: DisplaySelector) {
        self.enable(workspace);
        let previous = self.assignments[workspace.index()].clone();
        if previous == display {
            return;
        }
        self.assignments[workspace.index()] = display.clone();
        self.active.entry(display).or_insert(workspace);
        if self.active.get(&previous) == Some(&workspace) {
            if let Some(replacement) = WorkspaceId::all().find(|candidate| {
                self.is_enabled(*candidate) && self.assignment(*candidate) == &previous
            }) {
                self.active.insert(previous.clone(), replacement);
            } else {
                self.active.remove(&previous);
            }
        }
        if self.previous.get(&previous).is_some_and(|candidate| {
            *candidate == workspace || self.active.get(&previous) == Some(candidate)
        }) {
            self.previous.remove(&previous);
        }
    }

    /// Moves `workspace` to `target`, making it active there (recording any
    /// displaced active workspace on `target` as that display's previous
    /// workspace) and promoting `source_replacement` on the lane `workspace`
    /// left.
    pub fn move_to_display(
        &mut self,
        workspace: WorkspaceId,
        target: DisplaySelector,
        source_replacement: Option<WorkspaceId>,
    ) {
        self.enable(workspace);
        let source = self.assignments[workspace.index()].clone();
        if source == target {
            return;
        }
        self.assignments[workspace.index()] = target.clone();

        let displaced = self.active.insert(target.clone(), workspace);
        match displaced {
            Some(previous) if previous != workspace => {
                self.previous.insert(target, previous);
            }
            _ => {
                if self.previous.get(&target) == Some(&workspace) {
                    self.previous.remove(&target);
                }
            }
        }

        if self.active.get(&source) == Some(&workspace) {
            let replacement = source_replacement
                .filter(|candidate| {
                    *candidate != workspace
                        && self.is_enabled(*candidate)
                        && self.assignment(*candidate) == &source
                })
                .or_else(|| {
                    self.previous.get(&source).copied().filter(|candidate| {
                        *candidate != workspace
                            && self.is_enabled(*candidate)
                            && self.assignment(*candidate) == &source
                    })
                })
                .or_else(|| {
                    WorkspaceId::all().find(|candidate| {
                        self.is_enabled(*candidate) && self.assignment(*candidate) == &source
                    })
                });
            match replacement {
                Some(replacement) => {
                    self.active.insert(source.clone(), replacement);
                }
                None => {
                    self.active.remove(&source);
                }
            }
        }
        if self.previous.get(&source).is_some_and(|candidate| {
            *candidate == workspace || self.active.get(&source) == Some(candidate)
        }) {
            self.previous.remove(&source);
        }
    }

    pub fn enable(&mut self, workspace: WorkspaceId) {
        self.enabled.insert(workspace);
    }

    #[must_use]
    pub fn is_enabled(&self, workspace: WorkspaceId) -> bool {
        self.enabled.contains(&workspace)
    }

    pub fn switch(&mut self, workspace: WorkspaceId) -> WorkspaceId {
        let display = self.assignment(workspace).clone();
        let active = self.active_for(&display).unwrap_or(workspace);
        if active != workspace {
            self.previous.insert(display.clone(), active);
            self.active.insert(display, workspace);
        }
        active
    }

    #[must_use]
    pub fn previous_for(&self, display: &DisplaySelector) -> Option<WorkspaceId> {
        self.previous.get(display).copied()
    }

    #[must_use]
    pub fn engine(&self, workspace: WorkspaceId) -> &Engine {
        &self.engines[workspace.index()]
    }

    pub fn engine_mut(&mut self, workspace: WorkspaceId) -> &mut Engine {
        &mut self.engines[workspace.index()]
    }

    pub fn move_window(
        &mut self,
        window: &PhysicalWindowId,
        target: WorkspaceId,
    ) -> Option<WorkspaceId> {
        let source = self.workspace_of(window)?;
        let source_index = source.index();
        let target_index = target.index();
        if source_index == target_index {
            return None;
        }
        let (source_engine, target_engine) = if source_index < target_index {
            let (before, after) = self.engines.split_at_mut(target_index);
            (&mut before[source_index], &mut after[0])
        } else {
            let (before, after) = self.engines.split_at_mut(source_index);
            (&mut after[0], &mut before[target_index])
        };
        let slot = source_engine.take_slot(window)?;
        target_engine.insert_slot(slot);
        self.prune_intents(source);
        Some(source)
    }

    #[must_use]
    pub fn workspace_of(&self, window: &PhysicalWindowId) -> Option<WorkspaceId> {
        self.engines
            .iter()
            .position(|engine| {
                engine
                    .model()
                    .workspace
                    .slots
                    .iter()
                    .any(|slot| slot.identity_window() == window)
            })
            .map(WorkspaceId::from_index)
    }

    #[must_use]
    pub(crate) fn floating_workspace_of(&self, window: &PhysicalWindowId) -> Option<WorkspaceId> {
        self.floating_owners.get(window).copied()
    }

    #[must_use]
    pub(crate) fn workspace_of_any(&self, window: &PhysicalWindowId) -> Option<WorkspaceId> {
        self.workspace_of(window)
            .or_else(|| self.floating_workspace_of(window))
    }

    pub(crate) fn own_floating(&mut self, window: PhysicalWindowId, workspace: WorkspaceId) {
        self.floating_owners.insert(window, workspace);
    }

    pub(crate) fn windows(&self, workspace: WorkspaceId) -> Vec<PhysicalWindowId> {
        let mut windows = self.engine(workspace).bound_windows();
        windows.extend(
            self.floating_entries()
                .filter(|(_, owner)| *owner == workspace)
                .map(|(window, _)| window.clone()),
        );
        windows
    }

    pub(crate) fn app_instances(
        &self,
        workspace: WorkspaceId,
    ) -> impl Iterator<Item = &crate::AppInstanceId> {
        self.engine(workspace)
            .model()
            .workspace
            .slots
            .iter()
            .map(|slot| &slot.identity_window().app)
    }

    pub(crate) fn app_owns_windows(&self, app: &crate::AppInstanceId) -> bool {
        WorkspaceId::all().any(|workspace| self.app_instances(workspace).any(|owner| owner == app))
            || self.floating_owners.keys().any(|window| &window.app == app)
    }

    pub(crate) fn forget_app(
        &mut self,
        workspace: WorkspaceId,
        app: &crate::AppInstanceId,
    ) -> bool {
        let removed = self.engine_mut(workspace).forget_app(app);
        if removed {
            self.prune_intents(workspace);
        }
        removed
    }

    pub(crate) fn floating_windows_for_app(
        &self,
        app: &crate::AppInstanceId,
    ) -> Vec<PhysicalWindowId> {
        self.floating_entries()
            .filter(|(window, _)| &window.app == app)
            .map(|(window, _)| window.clone())
            .collect()
    }

    pub(crate) fn floating_entries(
        &self,
    ) -> impl Iterator<Item = (&PhysicalWindowId, WorkspaceId)> {
        self.floating_owners
            .iter()
            .map(|(window, workspace)| (window, *workspace))
    }

    pub(crate) fn forget_floating(&mut self, window: &PhysicalWindowId) {
        self.floating_owners.remove(window);
    }

    /// The one enabled workspace holding the app's windows, tiled or floating;
    /// none when they are spread over several.
    #[must_use]
    pub fn workspace_for_app(&self, app: &crate::AppInstanceId) -> Option<WorkspaceId> {
        let mut matches = WorkspaceId::all().filter(|workspace| {
            self.is_enabled(*workspace)
                && (self
                    .engine(*workspace)
                    .model()
                    .workspace
                    .slots
                    .iter()
                    .any(|slot| slot.belongs_to(app))
                    || self
                        .floating_entries()
                        .any(|(window, owner)| owner == *workspace && &window.app == app))
        });
        let workspace = matches.next()?;
        matches.next().is_none().then_some(workspace)
    }

    /// Candidate evidence is reserved, not owned: only reconciliation may bind
    /// it. More than one reserving workspace means adoption must be deferred.
    pub(crate) fn replacement_workspaces<'a>(
        &'a self,
        window: &'a PhysicalWindowId,
    ) -> impl Iterator<Item = WorkspaceId> + 'a {
        WorkspaceId::all().filter(|workspace| {
            self.engine(*workspace)
                .model()
                .workspace
                .slots
                .iter()
                .any(|slot| {
                    matches!(&slot.binding, crate::BindingState::Unresolved { candidates, .. }
                    if candidates.contains(window))
                })
        })
    }

    pub fn forget_window(&mut self, workspace: WorkspaceId, window: &PhysicalWindowId) {
        self.engine_mut(workspace).take_slot(window);
        self.prune_intents(workspace);
    }

    /// Remember `window` as the workspace's focused tile. A window without a
    /// tile (a float) leaves the remembered tile alone.
    pub(crate) fn remember_focus(&mut self, workspace: WorkspaceId, window: &PhysicalWindowId) {
        if let Some(slot) = self.engine(workspace).slot_for_window(window) {
            self.last_focused[workspace.index()] = Some(slot);
        }
    }

    pub(crate) fn preferred_focus(&self, workspace: WorkspaceId) -> Option<PhysicalWindowId> {
        let slot = self.last_focused[workspace.index()]?;
        self.engine(workspace)
            .model()
            .slot(slot)?
            .bound_window()
            .cloned()
    }

    pub(crate) fn uses_fullscreen_layout(&self, workspace: WorkspaceId) -> bool {
        self.fullscreen_layout[workspace.index()]
    }

    pub(crate) fn set_fullscreen_layout(&mut self, workspace: WorkspaceId, enabled: bool) {
        self.fullscreen_layout[workspace.index()] =
            enabled && !self.engine(workspace).model().workspace.slots.is_empty();
    }

    pub(crate) fn prune_intents(&mut self, workspace: WorkspaceId) {
        let index = workspace.index();
        self.last_focused[index] =
            self.last_focused[index].filter(|slot| self.has_slot(workspace, *slot));
        self.set_fullscreen_layout(workspace, self.fullscreen_layout[index]);
    }

    fn has_slot(&self, workspace: WorkspaceId, slot: crate::SlotId) -> bool {
        self.engine(workspace).model().slot(slot).is_some()
    }

    pub(crate) fn inactive_windows_except(
        &self,
        visible: &HashSet<WorkspaceId>,
    ) -> Vec<(WorkspaceId, PhysicalWindowId)> {
        WorkspaceId::all()
            .filter(|workspace| self.is_enabled(*workspace) && !visible.contains(workspace))
            .flat_map(|workspace| {
                self.windows(workspace)
                    .into_iter()
                    .map(move |window| (workspace, window))
            })
            .collect()
    }
}

fn sorted<T: Ord>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut items: Vec<_> = items.into_iter().collect();
    items.sort();
    items
}

fn trimmed<T: Clone>(items: &[T; WorkspaceId::COUNT], keep: impl Fn(&T) -> bool) -> Vec<T> {
    let length = items.iter().rposition(keep).map_or(0, |index| index + 1);
    items[..length].to_vec()
}

#[cfg(test)]
mod tests;
