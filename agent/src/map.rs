//! Rules as a workspace map: a tile per workspace showing the apps that open there, and the
//! apps that float below. Dragging an app to another tile, or to the float box, rewrites its
//! rule. Everything sits at computed positions, so a drag knows where the pointer is from the
//! dragged icon's own spot.

use rune::{Cursor, El, PointerPress, Tx, el, row, text};
use rune_kit::{Menu as KitMenu, MenuItem, Placement};
use spaceman::WorkspaceId;

use crate::{
    Menu, Pane, Ui,
    apps::{self, App},
    controls::{self, Paint},
    playground::{Scene, reset_fixture},
    rules::{Doc, Place, Rule},
    view::scroll_body,
};

const COLS: usize = 5;
const TILE_W: f32 = 108.0;
const TILE_H: f32 = 100.0;
const GAP: f32 = 10.0;
pub const MAP_W: f32 = COLS as f32 * TILE_W + (COLS - 1) as f32 * GAP;
const HEAD_H: f32 = 20.0;
const HEAD_GAP: f32 = 10.0;
const GRID_TOP: f32 = HEAD_H + HEAD_GAP;
const SECTION_GAP: f32 = 24.0;
/// Inside a tile: where its icons start, their size and spacing, and how many fit.
const TILE_PAD: f32 = 12.0;
const ICONS_TOP: f32 = 38.0;
const ICON: f32 = 24.0;
const ICON_GAP: f32 = 6.0;
const ICONS_PER_ROW: usize = 3;
const ICONS_SHOWN: usize = 6;
/// The float box and its chips.
const BOX_PAD: f32 = 10.0;
const CHIP_W: f32 = 128.0;
const CHIP_H: f32 = 32.0;
const CHIP_GAP: f32 = 8.0;
const CHIP_ICON: f32 = 20.0;
const CHIPS_PER_ROW: usize = 4;
/// Pointer travel before a press on an app becomes a drag.
const SLOP: f32 = 3.0;

pub(crate) const SCENES: &[Scene] = &[
    ("rules", |ui| reset_fixture(ui, Pane::Rules)),
    ("rules-drag", |ui| {
        reset_fixture(ui, Pane::Rules);
        ui.drag = Drag::fixture(&ui.doc);
    }),
    ("rules-add", |ui| {
        reset_fixture(ui, Pane::Rules);
        ui.open_add_menu(Place::Float);
    }),
];

/// An app being dragged: its rule, where it started and how far the pointer has gone.
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    pub rule: usize,
    snapshot: Rule,
    /// The dragged item's top-left in the map, and its size.
    origin: (f32, f32),
    size: (f32, f32),
    /// Where in the item the press landed.
    grab: (f32, f32),
    delta: (f32, f32),
    moved: bool,
    target: Option<Place>,
}

impl Drag {
    /// Slack mid-drag over workspace 4, for stills.
    fn fixture(doc: &Doc) -> Option<Drag> {
        let layout = Layout::new(doc);
        let rule = doc
            .rules()
            .into_iter()
            .find(|rule| rule.bundle_id.as_deref() == Some("com.tinyspeck.slackmacgap"))?;
        let (origin, size) = layout.spot(&rule)?;
        let grab = (size.0 / 2.0, size.1 / 2.0);
        let to = layout.grid.tile_origin(3);
        let delta = (
            to.0 + TILE_W / 2.0 - origin.0 - grab.0,
            to.1 + TILE_H / 2.0 - origin.1 - grab.1,
        );
        Some(Drag {
            rule: rule.index,
            snapshot: rule,
            origin,
            size,
            grab,
            delta,
            moved: true,
            target: layout
                .grid
                .target_at(to.0 + TILE_W / 2.0, to.1 + TILE_H / 2.0),
        })
    }
}

/// Where everything in the map goes, from the rules.
struct Layout {
    grid: DropGrid,
    /// Rules by tile, in file order.
    routed: Vec<Vec<Rule>>,
    floating: Vec<Rule>,
}

impl Layout {
    fn new(doc: &Doc) -> Layout {
        let rules = doc.rules();
        let tiles = doc.workspaces_with_referenced(WorkspaceId::numbered().take(9));
        let mut routed = vec![Vec::new(); tiles.len()];
        let mut floating = Vec::new();
        for rule in rules {
            match rule.place {
                Place::Workspace(workspace) => {
                    if let Some(index) = tiles.iter().position(|tile| *tile == workspace) {
                        routed[index].push(rule);
                    }
                }
                Place::Float | Place::Ignore => floating.push(rule),
            }
        }
        Layout {
            grid: DropGrid {
                tiles,
                floating_len: floating.len(),
            },
            routed,
            floating,
        }
    }

    /// The `slot`th icon in a tile, from the tile's top-left.
    fn icon_offset(slot: usize) -> (f32, f32) {
        let (col, row) = (slot % ICONS_PER_ROW, slot / ICONS_PER_ROW);
        (
            TILE_PAD + col as f32 * (ICON + ICON_GAP),
            ICONS_TOP + row as f32 * (ICON + ICON_GAP),
        )
    }

    /// A rule's item: its top-left in the map and its size.
    fn spot(&self, rule: &Rule) -> Option<((f32, f32), (f32, f32))> {
        if let Some(slot) = self.floating.iter().position(|r| r.index == rule.index) {
            return Some((self.grid.chip_origin(slot), (CHIP_W, CHIP_H)));
        }
        self.routed.iter().enumerate().find_map(|(tile, rules)| {
            let slot = rules.iter().position(|r| r.index == rule.index)?;
            let (x, y) = self.grid.tile_origin(tile);
            let (dx, dy) = Self::icon_offset(slot);
            Some(((x + dx, y + dy), (ICON, ICON)))
        })
    }
}

/// The map's geometry: the tiles and how many chips the float box holds. Drag handlers keep a
/// copy to hit-test the pointer.
#[derive(Clone, Debug)]
struct DropGrid {
    tiles: Vec<WorkspaceId>,
    floating_len: usize,
}

impl DropGrid {
    fn grid_rows(&self) -> usize {
        self.tiles.len().div_ceil(COLS)
    }

    fn grid_h(&self) -> f32 {
        let rows = self.grid_rows() as f32;
        rows * TILE_H + (rows - 1.0).max(0.0) * GAP
    }

    fn float_head_top(&self) -> f32 {
        GRID_TOP + self.grid_h() + SECTION_GAP
    }

    fn box_top(&self) -> f32 {
        self.float_head_top() + HEAD_H + HEAD_GAP
    }

    /// The float box holds its rules and the Add chip.
    fn box_h(&self) -> f32 {
        let rows = (self.floating_len + 1).div_ceil(CHIPS_PER_ROW) as f32;
        2.0 * BOX_PAD + rows * CHIP_H + (rows - 1.0) * CHIP_GAP
    }

    fn height(&self) -> f32 {
        self.box_top() + self.box_h()
    }

    fn tile_origin(&self, index: usize) -> (f32, f32) {
        let (col, row) = (index % COLS, index / COLS);
        (
            col as f32 * (TILE_W + GAP),
            GRID_TOP + row as f32 * (TILE_H + GAP),
        )
    }

    fn chip_origin(&self, slot: usize) -> (f32, f32) {
        let (col, row) = (slot % CHIPS_PER_ROW, slot / CHIPS_PER_ROW);
        (
            BOX_PAD + col as f32 * (CHIP_W + CHIP_GAP),
            self.box_top() + BOX_PAD + row as f32 * (CHIP_H + CHIP_GAP),
        )
    }

    /// The place under a point in the map.
    fn target_at(&self, x: f32, y: f32) -> Option<Place> {
        if (0.0..=MAP_W).contains(&x)
            && (GRID_TOP - GAP..=GRID_TOP + self.grid_h() + GAP).contains(&y)
        {
            let col = ((x + GAP / 2.0) / (TILE_W + GAP))
                .floor()
                .clamp(0.0, (COLS - 1) as f32);
            let row = ((y - GRID_TOP + GAP / 2.0) / (TILE_H + GAP))
                .floor()
                .clamp(0.0, (self.grid_rows() - 1) as f32);
            let index = row as usize * COLS + col as usize;
            return self.tiles.get(index).map(|tile| Place::Workspace(*tile));
        }
        let top = self.float_head_top();
        ((0.0..=MAP_W).contains(&x) && (top..=self.box_top() + self.box_h() + GAP).contains(&y))
            .then_some(Place::Float)
    }
}

pub fn pane(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let layout = Layout::new(&ui.doc);
    let view = MapView {
        ui,
        p,
        layout: &layout,
        active: ui
            .agent
            .as_ref()
            .map_or(&[][..], |agent| agent.active_workspaces.as_slice()),
        target: ui
            .drag
            .as_ref()
            .filter(|drag| drag.moved)
            .and_then(|drag| drag.target),
    };
    let hint = if ui.doc.rules().is_empty() {
        "Hover a workspace to add an app"
    } else {
        "Drag an app to move it"
    };
    let mut map = el().key("map").w(MAP_W).h(layout.grid.height()).child(
        controls::heading(p, "where", "Where apps open", Some(hint))
            .w(MAP_W)
            .px(0.0)
            .absolute(0.0, 0.0),
    );
    for (index, workspace) in layout.grid.tiles.iter().copied().enumerate() {
        let (x, y) = layout.grid.tile_origin(index);
        map = map.child(view.tile(index, workspace, layer).absolute(x, y));
    }
    map = map
        .child(
            controls::heading(p, "float-heading", "Floating", None)
                .w(MAP_W)
                .px(0.0)
                .absolute(0.0, layout.grid.float_head_top()),
        )
        .child(view.float_box(layer));
    if let Some(drag) = ui.drag.as_ref().filter(|drag| drag.moved) {
        map = map.children(view.ghost(drag));
    }
    // Drag positions are in map coordinates, so scrolling the body doesn't move drop targets.
    scroll_body("rules-body", map)
}

#[derive(Clone, Copy)]
struct MapView<'a> {
    ui: &'a Ui,
    p: Paint,
    layout: &'a Layout,
    active: &'a [WorkspaceId],
    target: Option<Place>,
}

impl MapView<'_> {
    fn tile(self, index: usize, workspace: WorkspaceId, layer: &mut Vec<El>) -> El {
        let c = &self.p.k.c;
        let rules = &self.layout.routed[index];
        let active = self.active.contains(&workspace);
        let target = self.target == Some(Place::Workspace(workspace));
        let border = if target {
            (1.5, c.border_focus)
        } else if active {
            (1.5, c.border_strong)
        } else {
            (self.p.hair, c.border_weak)
        };
        let mut tile = el()
            .key(format!("tile-{workspace}"))
            .role("group")
            .label(format!("Workspace {workspace}"))
            .size(TILE_W, TILE_H)
            .radius(self.p.k.radius.lg)
            .fill(if target { c.layer_02 } else { c.layer_01 })
            .border(border.0, border.1)
            .hover_group()
            .child(
                text(workspace.to_string())
                    .key("number")
                    .font(self.p.k.ty.ui_medium)
                    .color(if active {
                        self.p.text()
                    } else {
                        self.p.muted()
                    })
                    .absolute(TILE_PAD, 10.0),
            )
            .child(
                self.add_button(Place::Workspace(workspace), layer)
                    .absolute(TILE_W - 30.0, 8.0),
            );
        let overflow = rules.len() > ICONS_SHOWN;
        for (slot, rule) in rules.iter().enumerate() {
            let (x, y) = Layout::icon_offset(slot);
            if overflow && slot == ICONS_SHOWN - 1 {
                tile = tile.child(
                    el().key("more")
                        .size(ICON, ICON)
                        .radius(6.0)
                        .fill(c.layer_03)
                        .center()
                        .child(
                            text(format!("+{}", rules.len() - slot))
                                .font(self.p.k.ty.small_medium)
                                .color(self.p.muted()),
                        )
                        .absolute(x, y),
                );
                break;
            }
            let item = self.draggable(rule, self.app_icon(rule, ICON), layer);
            tile = tile.child(item.absolute(x, y));
        }
        tile
    }

    /// The float box: a chip per floating app, then Add.
    fn float_box(self, layer: &mut Vec<El>) -> El {
        let c = &self.p.k.c;
        let top = self.layout.grid.box_top();
        let target = self.target == Some(Place::Float);
        let mut area = el()
            .key("float-box")
            .w(MAP_W)
            .h(self.layout.grid.box_h())
            .radius(self.p.k.radius.lg)
            .fill(if target { c.layer_02 } else { c.layer_01 })
            .border(
                if target { 1.5 } else { self.p.hair },
                if target {
                    c.border_focus
                } else {
                    c.border_weak
                },
            )
            .absolute(0.0, top);
        for (slot, rule) in self.layout.floating.iter().enumerate() {
            let (x, y) = self.layout.grid.chip_origin(slot);
            let chip = self.chip(rule);
            area = area.child(self.draggable(rule, chip, layer).absolute(x, y - top));
        }
        let (x, y) = self.layout.grid.chip_origin(self.layout.floating.len());
        area.child(self.add_chip(layer).absolute(x, y - top))
    }

    fn chip(self, rule: &Rule) -> El {
        let c = &self.p.k.c;
        let name = self.rule_name(rule);
        row()
            .size(CHIP_W, CHIP_H)
            .px(6.0)
            .gap(8.0)
            .center_y()
            .radius(self.p.k.radius.md)
            .fill(c.layer_02)
            .child(self.app_icon(rule, CHIP_ICON))
            .child(
                text(if rule.place == Place::Ignore {
                    format!("{name} · ignored")
                } else {
                    name
                })
                .key("name")
                .font(self.p.k.ty.ui)
                .color(self.p.text())
                .truncate()
                .shrink(),
            )
    }

    fn add_chip(self, layer: &mut Vec<El>) -> El {
        let c = &self.p.k.c;
        let trigger = row()
            .key("add-float")
            .role("button")
            .label("Add app")
            .size(CHIP_W, CHIP_H)
            .px(10.0)
            .gap(6.0)
            .center_y()
            .radius(self.p.k.radius.md)
            .border(self.p.hair, c.border_base)
            .hover(c.hover)
            .press(c.pressed)
            .focusable(true)
            .focus_ring(2.0, -2.0, c.border_focus)
            .on_press(|ui: &mut Ui| ui.open_add_menu(Place::Float))
            .child(rune::icon(&rune_theme_opencode::icons::PLUS, 14.0).color(c.icon_muted))
            .child(text("Add app").font(self.p.k.ty.ui).color(self.p.muted()));
        self.add_menu("add-float-menu", trigger, Place::Float, layer)
    }

    /// A tile's Add, shown while the pointer is over the tile or its menu is open.
    fn add_button(self, place: Place, layer: &mut Vec<El>) -> El {
        let c = &self.p.k.c;
        let open = self.ui.menu == Some(Menu::Add(place));
        let trigger = el()
            .key("add")
            .role("button")
            .label("Add an app here")
            .size(22.0, 22.0)
            .center()
            .radius(self.p.k.radius.sm)
            .hover(c.hover)
            .press(c.pressed)
            .on_press(move |ui: &mut Ui| ui.open_add_menu(place))
            .child(rune::icon(&rune_theme_opencode::icons::PLUS, 14.0).color(c.icon_muted));
        let trigger = if open || self.ui.drag.is_some() {
            trigger.opacity(if open { 1.0 } else { 0.0 })
        } else {
            trigger.group_opacity(0.0, 1.0)
        };
        let key = match place {
            Place::Workspace(workspace) => format!("add-{workspace}-menu"),
            Place::Float | Place::Ignore => "add-menu".into(),
        };
        self.add_menu(&key, trigger, place, layer)
    }

    fn add_menu(self, key: &str, trigger: El, place: Place, layer: &mut Vec<El>) -> El {
        let open = self.ui.menu == Some(Menu::Add(place));
        let mut menu = KitMenu::new(key)
            .trigger(trigger)
            .placement(Placement::BottomStart)
            .gutter(4.0)
            .min_width(220.0)
            .open(open)
            .on_close(|ui: &mut Ui| ui.menu = None);
        if open {
            menu = menu.group_label("Running apps");
            if self.ui.running.is_empty() {
                menu = menu.item(MenuItem::new("Every running app has a rule").disabled(true));
            }
            for app in &self.ui.running {
                let bundle_id = app.bundle_id.clone();
                let lead = controls::icon_or_initial(
                    self.p,
                    self.ui.apps.cached(&bundle_id),
                    &app.name,
                    16.0,
                );
                menu = menu.item(MenuItem::new(app.name.clone()).lead(lead).on_select(
                    move |ui: &mut Ui| {
                        let bundle_id = bundle_id.clone();
                        ui.menu = None;
                        ui.edit_and_retile(move |doc| doc.add(&bundle_id, place));
                    },
                ));
            }
        }
        menu.build(self.p.k, layer)
    }

    /// `item` for `rule`, dragged to move it and right-clicked for its menu.
    fn draggable(self, rule: &Rule, item: El, layer: &mut Vec<El>) -> El {
        let index = rule.index;
        let Some((origin, size)) = self.layout.spot(rule) else {
            return item;
        };
        let dragging = self
            .ui
            .drag
            .as_ref()
            .is_some_and(|drag| drag.rule == index && drag.moved);
        let grid = self.layout.grid.clone();
        let drag_rule = rule.clone();
        let item = item
            .key(format!("rule-{index}"))
            .role("button")
            .label(self.rule_name(rule))
            .help(self.rule_help(rule))
            .cursor(Cursor::Grab)
            .opacity(if dragging { 0.3 } else { 1.0 })
            .on_drag(move |ui: &mut Ui, d: &mut rune::Drag, _: &mut Tx| {
                drag(ui, d, &drag_rule, origin, size, |x, y| grid.target_at(x, y))
            })
            .on_secondary_press(move |ui: &mut Ui, press: PointerPress, _: &mut Tx| {
                ui.menu = Some(Menu::Rule(index, press));
            });
        let point = match self.ui.menu {
            Some(Menu::Rule(open, point)) if open == index => Some(point),
            _ => None,
        };
        let mut menu = rune_kit::context_menu(format!("rule-{index}-menu"), item, point)
            .on_close(|ui: &mut Ui| ui.menu = None);
        if point.is_some() {
            if rule.place != Place::Float {
                let expected = rule.clone();
                menu = menu.item(MenuItem::new("Float").on_select(move |ui: &mut Ui| {
                    ui.menu = None;
                    ui.try_edit_and_retile(|doc| doc.place_matching(&expected, Place::Float));
                }));
            }
            let expected = rule.clone();
            menu = menu.item(MenuItem::new("Remove rule").on_select(move |ui: &mut Ui| {
                ui.menu = None;
                ui.try_edit_and_retile(|doc| doc.remove_matching(&expected));
            }));
        }
        menu.build(self.p.k, layer)
    }

    /// The dragged item, following the pointer above everything else in the map.
    fn ghost(self, drag: &Drag) -> Option<El> {
        let rule = self
            .layout
            .routed
            .iter()
            .flatten()
            .chain(&self.layout.floating)
            .find(|rule| rule.index == drag.rule)?;
        let item = if drag.size == (CHIP_W, CHIP_H) {
            self.chip(rule)
        } else {
            self.app_icon(rule, ICON)
        };
        let mut ghost = item
            .key("ghost")
            .cursor(Cursor::Grabbing)
            .absolute(drag.origin.0 + drag.delta.0, drag.origin.1 + drag.delta.1);
        for shadow in &self.p.k.elevation.floating {
            ghost = ghost.box_shadow(*shadow);
        }
        Some(ghost)
    }

    fn rule_app<'a>(self, rule: &Rule) -> Option<&'a App>
    where
        Self: 'a,
    {
        self.ui.apps.cached(rule.bundle_id.as_deref()?)
    }

    fn rule_name(self, rule: &Rule) -> String {
        match (self.rule_app(rule), &rule.bundle_id, &rule.title) {
            (Some(app), _, _) => app.name.clone(),
            (None, Some(bundle_id), _) => apps::fallback_name(bundle_id),
            (None, None, Some(title)) => title.clone(),
            (None, None, None) => "Any app".into(),
        }
    }

    fn rule_help(self, rule: &Rule) -> String {
        let name = self.rule_name(rule);
        match &rule.title {
            Some(title) => format!("{name}, windows titled “{title}”"),
            None => name,
        }
    }

    fn app_icon(self, rule: &Rule, size: f32) -> El {
        let icon =
            controls::icon_or_initial(self.p, self.rule_app(rule), &self.rule_name(rule), size);
        if rule.title.is_none() {
            return icon;
        }
        // A rule for some of an app's windows wears a dot.
        let c = &self.p.k.c;
        icon.child(
            el().key("title-dot")
                .size(8.0, 8.0)
                .radius(4.0)
                .fill(c.icon_accent)
                .border(1.5, c.layer_01)
                .absolute(size - 6.0, -2.0),
        )
    }
}

fn drag(
    ui: &mut Ui,
    d: &rune::Drag,
    rule: &Rule,
    origin: (f32, f32),
    size: (f32, f32),
    target_at: impl Fn(f32, f32) -> Option<Place>,
) {
    drag_step(
        ui,
        d.phase,
        (d.start_x, d.start_y),
        (d.dx, d.dy),
        d.cancelled(),
        rule,
        origin,
        size,
        target_at,
    );
}

#[allow(clippy::too_many_arguments)]
fn drag_step(
    ui: &mut Ui,
    phase: rune::Phase,
    start: (f32, f32),
    delta: (f32, f32),
    cancelled: bool,
    rule: &Rule,
    origin: (f32, f32),
    size: (f32, f32),
    target_at: impl Fn(f32, f32) -> Option<Place>,
) {
    match phase {
        rune::Phase::Down => {
            ui.menu = None;
            ui.drag = Some(Drag {
                rule: rule.index,
                snapshot: rule.clone(),
                origin,
                size,
                grab: start,
                delta: (0.0, 0.0),
                moved: false,
                target: None,
            });
        }
        rune::Phase::Move => {
            let Some(drag) = ui.drag.as_mut().filter(|drag| drag.rule == rule.index) else {
                return;
            };
            drag.delta = delta;
            drag.moved |= delta.0.abs().max(delta.1.abs()) >= SLOP;
            drag.target = target_at(
                origin.0 + drag.grab.0 + delta.0,
                origin.1 + drag.grab.1 + delta.1,
            );
        }
        rune::Phase::Up => {
            let Some(drag) = ui.drag.take().filter(|drag| drag.rule == rule.index) else {
                return;
            };
            if cancelled || !drag.moved {
                ui.flush_pending_reload();
                return;
            }
            let Some(target) = drag.target else {
                ui.flush_pending_reload();
                return;
            };
            let current = drag.snapshot.place;
            let unchanged =
                current == target || (target == Place::Float && current == Place::Ignore);
            if unchanged {
                ui.flush_pending_reload();
            } else {
                ui.try_edit_and_retile(|doc| doc.place_matching(&drag.snapshot, target));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_tests_tiles_and_the_float_box() {
        let doc = Doc::parse(include_str!("../fixtures/config.toml"));
        let layout = Layout::new(&doc);
        let grid = &layout.grid;
        assert_eq!(grid.tiles.len(), 10, "1–9 and M");
        let ws = |name| Some(Place::Workspace(WorkspaceId::parse(name).unwrap()));
        assert_eq!(grid.target_at(5.0, GRID_TOP + 5.0), ws("1"));
        let (x, y) = grid.tile_origin(3);
        assert_eq!(grid.target_at(x + 50.0, y + 50.0), ws("4"));
        let (x, y) = grid.tile_origin(9);
        assert_eq!(grid.target_at(x + 50.0, y + 50.0), ws("M"));
        assert_eq!(
            grid.target_at(20.0, grid.box_top() + 10.0),
            Some(Place::Float)
        );
        assert_eq!(grid.target_at(20.0, grid.height() + 100.0), None);
    }

    #[test]
    fn every_rule_has_a_spot() {
        let doc = Doc::parse(include_str!("../fixtures/config.toml"));
        let layout = Layout::new(&doc);
        for rule in doc.rules() {
            assert!(layout.spot(&rule).is_some(), "{rule:?}");
        }
    }

    #[test]
    fn rule_drag_preserves_external_save_and_flushes_deferred_reload_on_cancel() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "spaceman-map-drag-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let initial = concat!(
            "version = 1\n\n",
            "[settings]\n",
            "hud = true\n\n",
            "[[rules]]\n",
            "bundle_id = \"com.apple.TextEdit\"\n",
            "action = \"route\"\n",
            "workspace = 1\n",
        );
        let external_hud_off = concat!(
            "version = 1\n\n",
            "[settings]\n",
            "hud = false\n\n",
            "[[rules]]\n",
            "bundle_id = \"com.apple.TextEdit\"\n",
            "action = \"route\"\n",
            "workspace = 1\n",
        );
        std::fs::write(&path, initial).unwrap();

        let mut ui = crate::playground::blank_fixture();
        ui.fixture = false;
        ui.doc = Doc::load_from(path.clone());
        let layout = Layout::new(&ui.doc);
        let rule = ui.doc.rules()[0].clone();
        let (origin, size) = layout.spot(&rule).unwrap();
        let ws2 = layout.grid.tile_origin(1);
        let dx = ws2.0 + TILE_W / 2.0 - origin.0 - size.0 / 2.0;
        let dy = ws2.1 + TILE_H / 2.0 - origin.1 - size.1 / 2.0;

        let grab = (size.0 / 2.0, size.1 / 2.0);

        // 1. Hold pointer down on TextEdit, receive an external save mid-drag, then drop on ws 2.
        drag_step(
            &mut ui,
            rune::Phase::Down,
            grab,
            (0.0, 0.0),
            false,
            &rule,
            origin,
            size,
            |x, y| layout.grid.target_at(x, y),
        );
        assert!(ui.drag.is_some());
        std::fs::write(&path, external_hud_off).unwrap();
        assert!(
            !ui.on_config_changed(),
            "reload must defer while a drag is active"
        );
        assert!(ui.doc.reload_pending());
        drag_step(
            &mut ui,
            rune::Phase::Move,
            grab,
            (dx, dy),
            false,
            &rule,
            origin,
            size,
            |x, y| layout.grid.target_at(x, y),
        );
        drag_step(
            &mut ui,
            rune::Phase::Up,
            grab,
            (dx, dy),
            false,
            &rule,
            origin,
            size,
            |x, y| layout.grid.target_at(x, y),
        );
        assert!(ui.drag.is_none());
        assert_eq!(ui.error, None);
        assert!(!ui.doc.reload_pending());
        let saved = std::fs::read_to_string(&path).unwrap();
        let parsed = spaceman::config::Config::parse(&saved).unwrap();
        assert!(!parsed.hud(), "external hud = false must survive:\n{saved}");
        assert_eq!(ui.doc.rules()[0].place, Place::Workspace(WorkspaceId::TWO));
        assert!(saved.contains("workspace = 2"), "{saved}");

        // 2. Cancelled drag flushes a deferred reload immediately instead of leaving Rules stale.
        let rule_ws2 = ui.doc.rules()[0].clone();
        drag_step(
            &mut ui,
            rune::Phase::Down,
            grab,
            (0.0, 0.0),
            false,
            &rule_ws2,
            origin,
            size,
            |x, y| layout.grid.target_at(x, y),
        );
        std::fs::write(&path, initial).unwrap();
        assert!(!ui.on_config_changed());
        drag_step(
            &mut ui,
            rune::Phase::Up,
            grab,
            (0.0, 0.0),
            true,
            &rule_ws2,
            origin,
            size,
            |x, y| layout.grid.target_at(x, y),
        );
        assert!(ui.drag.is_none());
        assert!(!ui.doc.reload_pending());
        assert!(ui.doc.hud(), "deferred reload must apply after drag cancel");
        assert_eq!(ui.doc.rules()[0].place, Place::Workspace(WorkspaceId::ONE));

        // 3. Control: idle config_changed notification reloads immediately.
        std::fs::write(&path, external_hud_off).unwrap();
        assert!(ui.on_config_changed());
        assert!(!ui.doc.hud());

        drop(std::fs::remove_dir_all(dir));
    }
}
