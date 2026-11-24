//! Settings controls on `rune-kit` with OpenCode's theme, the same as Hex's and Invoke's: the
//! raised card, its rows, switches, permission controls and buttons.

use std::sync::LazyLock;

use rune::{
    Beat, Color, Easing, Edge, El, Icon, LoopSpec, Sequence, Shadow, Spring, SpringPair, TextStyle,
    Transition, Tx, col, el, icon, row, text,
};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, Switch};

use crate::{Ui, apps::App};

/// Row descriptions and secondary copy.
pub const DESCRIPTION: TextStyle = TextStyle::new(12.0, 440.0).lh(16.0);
const ROW_HEIGHT: f32 = 44.0;
const ROW_HEIGHT_DESCRIBED: f32 = 56.0;
const PULSE: LoopSpec = LoopSpec::opacity(2.0, 0.0, 1.0, 0.45);

/// Everything a control needs to paint itself.
#[derive(Clone, Copy)]
pub struct Paint {
    pub k: &'static Kit,
    pub hair: f32,
}

impl Paint {
    pub fn new(dark: bool, scale: f32) -> Self {
        Self {
            k: rune_theme_opencode::kit(dark),
            hair: 1.0 / scale,
        }
    }
    pub fn text(self) -> Color {
        self.k.c.text_base
    }
    pub fn muted(self) -> Color {
        self.k.c.text_muted
    }
    pub fn faint(self) -> Color {
        self.k.c.text_faint
    }
    pub fn success(self) -> Color {
        self.k.c.success.fg
    }
    pub fn warning(self) -> Color {
        self.k.c.warning.fg
    }
    pub fn danger(self) -> Color {
        self.k.c.danger.fg
    }
    pub fn accent(self) -> Color {
        self.k.c.text_accent
    }
}

/// A small heading over a card, with optional copy on the right.
pub fn heading(p: Paint, key: &str, title: &str, trailing: Option<&str>) -> El {
    row()
        .key(key)
        .h(20.0)
        .px(12.0)
        .center_y()
        .justify_between()
        .child(
            text(title)
                .key("title")
                .font(p.k.ty.ui_medium.lh(16.0))
                .color(p.muted()),
        )
        .children(trailing.map(|copy| {
            text(copy)
                .key("trailing")
                .font(p.k.ty.small)
                .color(p.faint())
        }))
}

/// A titled card of rows.
pub fn section(p: Paint, key: &str, title: &str, rows: Vec<El>) -> El {
    col()
        .key(key)
        .gap(8.0)
        .child(heading(p, "header", title, None))
        .child(card(p, "card", rows))
}

/// A raised card of rows with hairline separators between them.
pub fn card(p: Paint, key: &str, rows: Vec<El>) -> El {
    let mut card = surface(p, col().key(key).padding(4.0));
    for (index, row) in rows.into_iter().enumerate() {
        if index > 0 {
            card = card.child(
                el().key(format!("sep{index}"))
                    .h(p.hair)
                    .ml(12.0)
                    .mr(12.0)
                    .fill(p.k.c.border_muted),
            );
        }
        card = card.child(row);
    }
    card
}

/// The card's raised surface on any element.
pub fn surface(p: Paint, element: El) -> El {
    let mut element = element.fill(p.k.c.bg_base).radius(p.k.radius.lg);
    for shadow in &p.k.elevation.raised {
        element = element.box_shadow(*shadow);
    }
    element
}

/// A setting: title and optional description on the left, its control on the right.
pub fn setting_row(
    p: Paint,
    key: &str,
    title: &str,
    description: Option<(String, Color)>,
    control: El,
) -> El {
    leading_row(p, key, None, title, description, control)
}

/// A setting row led by a 16-point glyph on a shared icon spine.
pub fn icon_row(
    p: Paint,
    key: &str,
    glyph: &'static Icon,
    title: &str,
    description: Option<(String, Color)>,
    control: El,
) -> El {
    let lead = icon(glyph, 16.0).key("icon").color(p.k.c.icon_base);
    leading_row(p, key, Some(lead), title, description, control)
}

fn leading_row(
    p: Paint,
    key: &str,
    lead: Option<El>,
    title: &str,
    description: Option<(String, Color)>,
    control: El,
) -> El {
    let height = if description.is_some() {
        ROW_HEIGHT_DESCRIBED
    } else {
        ROW_HEIGHT
    };
    let copy = col()
        .key("copy")
        .grow(1.0)
        .shrink()
        .gap(2.0)
        .child(text(title).key("label").font(p.k.ty.ui).color(p.text()))
        .children(description.map(|(copy, color)| {
            text(copy)
                .key("description")
                .font(DESCRIPTION)
                .color(color)
                .wrap()
        }));
    row()
        .key(key)
        .min_h(height)
        .px(12.0)
        .py(8.0)
        .gap(24.0)
        .center_y()
        .justify_between()
        .radius(p.k.radius.md)
        .label(title)
        .child(match lead {
            None => copy,
            Some(lead) => row()
                .key("lead")
                .grow(1.0)
                .shrink()
                .gap(12.0)
                .center_y()
                .child(lead)
                .child(copy),
        })
        .child(control)
}

/// A whole-row switch: the row presses, the switch shows the state.
pub fn toggle_row(
    p: Paint,
    key: &str,
    title: &str,
    description: Option<(String, Color)>,
    on: bool,
    flip: impl Fn(&mut Ui) + 'static,
) -> El {
    let switch = Switch::new(on).build(p.k).key("switch").focusable(false);
    setting_row(p, key, title, description, switch)
        .role("toggle")
        .focusable(true)
        .on_press(flip)
}

/// An [`icon_row`] whose whole row toggles a switch.
pub fn icon_toggle_row(
    p: Paint,
    key: &str,
    glyph: &'static Icon,
    title: &str,
    description: Option<(String, Color)>,
    on: bool,
    flip: impl Fn(&mut Ui) + 'static,
) -> El {
    let switch = Switch::new(on).build(p.k).key("switch").focusable(false);
    icon_row(p, key, glyph, title, description, switch)
        .role("toggle")
        .focusable(true)
        .on_press(flip)
}

/// `path` with the home directory shortened to `~`.
pub fn home_relative(path: &std::path::Path) -> String {
    std::env::var_os("HOME")
        .and_then(|home| path.strip_prefix(home).ok())
        .map_or_else(
            || path.display().to_string(),
            |rest| format!("~/{}", rest.display()),
        )
}

/// Reveals `path` in Finder, or opens its parent folder if the file has not been created yet.
pub fn reveal_in_finder(path: &std::path::Path) {
    let target = if path.exists() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    drop(
        std::process::Command::new("open")
            .arg("-R")
            .arg(target)
            .spawn(),
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
}

pub fn button(
    p: Paint,
    key: &str,
    label: &str,
    kind: ButtonKind,
    press: impl Fn(&mut Ui, &mut Tx) + 'static,
) -> El {
    Button::new(label)
        .variant(match kind {
            ButtonKind::Primary => ButtonVariant::Contrast,
            ButtonKind::Secondary => ButtonVariant::Neutral,
        })
        .on_press_tx(press)
        .control_size(ControlSize::Large)
        .build(p.k)
        .px(11.0)
        .radius(p.k.radius.md)
        .key(key)
}

pub fn status_dot(color: Color) -> El {
    el().key("dot").size(8.0, 8.0).radius(4.0).fill(color)
}

pub fn pulsing_dot(color: Color) -> El {
    status_dot(color).looping(PULSE)
}

/// An app's Finder icon at `size`, or its initial in a rounded square when unavailable.
pub fn icon_or_initial(p: Paint, app: Option<&App>, name: &str, size: f32) -> El {
    match app.and_then(|app| app.icon.as_ref()) {
        Some(pixels) => el().size(size, size).pixels(pixels),
        None => el()
            .size(size, size)
            .radius(size * 0.25)
            .fill(p.k.c.layer_03)
            .center()
            .child(
                text(name.chars().next().map(String::from).unwrap_or_default())
                    .font(p.k.ty.small_medium)
                    .color(p.muted()),
            ),
    }
}

/// How a permission's control reads in onboarding and Settings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionControl {
    Allow,
    Waiting,
    /// Granted (`animated` plays the two-beat check draw-on after `Allow` was pressed).
    Allowed {
        animated: bool,
    },
}

const GATHER: Spring = Spring::new(0.16, 0.0);
const GATHER_SECS: f64 = 0.14;
const DIP: f32 = 0.94;
const RELEASE: Spring = Spring::new(0.3, 0.16);
const RELEASE_SECS: f64 = 0.45;
const DRAW: Spring = Spring::new(0.42, 0.0);
const SUFFIX: Spring = Spring::new(0.4, 0.0);
const INK: Spring = Spring::new(0.3, 0.0);
const APPEAR: &[Beat] = &[
    Beat::new(0.0).opacity(0.0),
    Beat::new(0.1).opacity(0.0),
    Beat::new(0.15).opacity(1.0).easing(Easing::EASE_OUT),
];
const PILL_PADDING: f32 = 11.0;
const CHECK_INK: [f32; 4] = [4.6, 4.6, 6.8, 6.8];
const CHECK_START: [f32; 2] = [5.2, 9.15];
const CHECK_X: f32 = 4.0;

/// Two-beat permission morph: `Allow` -> pulsing dot while macOS asks -> green `Allowed`.
pub fn permission_control(
    p: Paint,
    key: &str,
    control: PermissionControl,
    kind: ButtonKind,
    allow: impl Fn(&mut Ui) + 'static,
) -> El {
    let (mark, gap, pill) = match control {
        PermissionControl::Allow => (
            None,
            12.0,
            button(p, key, "Allow", kind, move |ui: &mut Ui, _| allow(ui)),
        ),
        PermissionControl::Waiting => (
            Some(
                el().key("mark")
                    .size(8.0, 8.0)
                    .child(pulsing_dot(p.accent()).absolute(0.0, 0.0)),
            ),
            12.0,
            button(
                p,
                key,
                "Allow",
                ButtonKind::Secondary,
                move |ui: &mut Ui, _| allow(ui),
            ),
        ),
        PermissionControl::Allowed { animated: true } => {
            (Some(check_mark(p)), 6.0, allowed_pill(p, key, kind))
        }
        PermissionControl::Allowed { animated: false } => {
            return row()
                .key(format!("{key}-control"))
                .gap(6.0)
                .pr(4.0)
                .center_y()
                .child(
                    icon(p.k.glyph.check_small, 16.0)
                        .key("check")
                        .color(p.success()),
                )
                .child(
                    text("Allowed")
                        .key("label")
                        .font(p.k.ty.ui_medium)
                        .color(p.success()),
                );
        }
    };
    let control_row = row()
        .key(format!("{key}-control"))
        .gap(gap)
        .center_y()
        .motion(GATHER)
        .children(mark)
        .child(pill.motion(GATHER));
    if matches!(control, PermissionControl::Allowed { animated: true }) {
        control_row.sequence(Sequence::new(grant_beats()))
    } else {
        control_row
    }
}

fn grant_beats() -> &'static [Beat] {
    static BEATS: LazyLock<[Beat; 3]> = LazyLock::new(|| {
        static CURVE: LazyLock<Vec<[f32; 2]>> = LazyLock::new(|| {
            const STOPS: usize = 48;
            (0..=STOPS)
                .map(|i| {
                    let x = i as f32 / STOPS as f32;
                    let t = x * RELEASE_SECS as f32;
                    let y = if i == STOPS {
                        1.0
                    } else {
                        RELEASE.sample(0.0, 0.0, 1.0, t).0
                    };
                    [x, y]
                })
                .collect()
        });
        [
            Beat::new(0.0),
            Beat::new(GATHER_SECS).scale(DIP).easing(Easing::EASE_IN),
            Beat::new(GATHER_SECS + RELEASE_SECS).easing(Easing::Piecewise(&CURVE)),
        ]
    });
    &*BEATS
}

fn check_mark(p: Paint) -> El {
    let [x, y, w, h] = CHECK_INK;
    el().key("mark")
        .size(9.0, 16.0)
        .child(
            status_dot(p.success())
                .absolute(CHECK_X + CHECK_START[0], CHECK_START[1])
                .size(0.0, 0.0)
                .radius(0.0)
                .opacity(0.0)
                .motion(GATHER),
        )
        .child(
            el().key("ink")
                .absolute(CHECK_X + x, y)
                .size(w, h)
                .transition(wipe(DRAW, 3.0))
                .sequence(Sequence::new(APPEAR))
                .child(
                    icon(p.k.glyph.check_small, 16.0)
                        .key("check")
                        .absolute(-x, -y)
                        .color(p.success()),
                ),
        )
}

fn wipe(spring: Spring, feather: f32) -> Transition {
    let mut wipe = Transition::reveal(Edge::Leading).springs(SpringPair::new(spring, spring));
    for reveal in [&mut wipe.insertion.reveal, &mut wipe.removal.reveal]
        .into_iter()
        .flatten()
    {
        reveal.feather = feather;
    }
    wipe
}

fn allowed_pill(p: Paint, key: &str, kind: ButtonKind) -> El {
    let k = p.k;
    let (fill, shadows) = match kind {
        ButtonKind::Primary => (k.c.bg_contrast, &k.elevation.button_contrast),
        ButtonKind::Secondary => (k.c.bg_button_neutral, &k.elevation.button_neutral),
    };
    let font = k.ty.ui_medium;
    let ink = p.success();
    let mut pill = row()
        .key(key)
        .label("Allowed")
        .h(28.0)
        .pl(PILL_PADDING)
        .center_y()
        .radius(k.radius.md)
        .fill(fill.alpha(0.0));
    for shadow in k.shadow_layers(shadows) {
        pill = pill.box_shadow(Shadow {
            color: shadow.color.alpha(0.0),
            ..shadow
        });
    }
    if kind == ButtonKind::Primary {
        pill = pill.gradient(Color::TRANSPARENT, Color::TRANSPARENT);
    }
    pill.child(text("Allow").key("label").font(font).color(ink).motion(INK))
        .child(
            text("ed")
                .key("suffix")
                .font(font)
                .color(ink)
                .transition(wipe(SUFFIX, 2.0))
                .sequence(Sequence::new(APPEAR)),
        )
}
