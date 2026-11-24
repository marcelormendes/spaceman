//! Hex's keycaps: low-profile scissor keys after Apple's Magic Keyboard, a flat face on a cap
//! whose skirt shows below it, resting in a tray recessed into the card. A held key sinks its
//! face into the skirt; the key's bounds never change.

use rune::{Color, Edge, El, Shadow, Spring, TextStyle, Transition, col, row, text};
use rune_kit::Kit;

use crate::controls::Paint;

/// Height, skirt included.
pub const KEY_HEIGHT: f32 = 22.0;
/// The skirt showing below a key at rest.
const SKIRT: f32 = 1.5;
/// The cap's rim beside and above the face.
const RIM: f32 = 0.5;
/// A press sinks the face most of the way, leaving a hairline of skirt.
const TRAVEL: f32 = SKIRT - RIM;
const FACE_RADIUS: f32 = 4.0;
const FACE_PADDING: f32 = 6.0;
const LEGEND: TextStyle = TextStyle::new(11.0, 400.0).lh(14.0);
/// Between the keycaps of a chord.
pub const KEY_GAP: f32 = 3.0;
/// Between a tray's edge and its keys: a tray is as tall as a button.
pub const TRAY_PADDING: f32 = 3.0;
/// A face sinks fast and comes back up a little slower, both without bounce.
const KEY_DOWN: Spring = Spring::new(0.08, 0.0);
const KEY_UP: Spring = Spring::new(0.18, 0.0);

/// The keys' material, from the theme's tokens: in light a warm off-white face on a cap that
/// shades to warm grey; in dark a space-black face on a cap that falls to the deepest
/// background.
struct Material {
    cap_top: Color,
    cap_bottom: Color,
    face: Color,
    ink: Color,
    /// The tight shadow under the cap and the soft one it casts at rest.
    contact: f32,
    cast: f32,
}

impl Material {
    fn new(k: &Kit) -> Self {
        let c = &k.c;
        if k.dark {
            Self {
                cap_top: c.layer_03.mix(c.layer_04, 0.35),
                cap_bottom: c.bg_deep,
                face: c.bg_base.mix(c.layer_01, 0.6),
                ink: c.text_base.mix(c.text_faint, 0.15),
                contact: 0.8,
                cast: 0.5,
            }
        } else {
            let warm = c.warning.bg;
            Self {
                cap_top: c.white,
                cap_bottom: c.layer_04.mix(c.text_faint, 0.28).mix(c.warning.fg, 0.05),
                face: c.layer_02.mix(warm, 0.35),
                ink: c.text_base.mix(c.text_muted, 0.15),
                contact: 0.2,
                cast: 0.12,
            }
        }
    }
}

/// One key. `down` sinks the face into the skirt, tightens the cap's shadow and shades the
/// face toward its bottom edge.
pub fn keycap(p: Paint, index: usize, label: &str, down: bool) -> El {
    let k = p.k;
    let m = Material::new(k);
    let pressed = if down { 1.0 } else { 0.0 };
    let travel = TRAVEL * pressed;
    let face_height = KEY_HEIGHT - RIM - SKIRT;
    // Shadow offsets never spring, so the resting and pressed casts are two layers that
    // trade strength.
    let cast = |y: f32, blur: f32, strength: f32| Shadow {
        y,
        blur,
        color: k.dark_a(m.cast * strength),
        ..Shadow::default()
    };
    let cap = col()
        .key("cap")
        .px(RIM)
        .pt(RIM)
        .pb(SKIRT - travel)
        .radius(FACE_RADIUS + RIM)
        .continuous_corners()
        .fill(m.cap_bottom)
        .gradient(m.cap_top, m.cap_bottom)
        .shadow(0.5, 0.5, k.dark_a(m.contact))
        .box_shadow(cast(SKIRT, 2.0 * SKIRT, 1.0 - pressed))
        .box_shadow(cast(RIM, 2.0, pressed))
        .child(
            row()
                .key("face")
                .min_w(KEY_HEIGHT - 2.0 * RIM)
                .h(face_height)
                .px(FACE_PADDING)
                .center()
                .radius(FACE_RADIUS)
                .continuous_corners()
                .fill(m.face)
                .gradient(
                    k.light(0.03 - 0.03 * pressed),
                    k.dark_a(0.02 + 0.06 * pressed),
                )
                .child(
                    text(label.to_owned())
                        .key("label")
                        .font(LEGEND)
                        .color(m.ink),
                ),
        );
    col()
        .key(format!("keycap{index}"))
        .h(KEY_HEIGHT)
        .pt(travel)
        .motion(if down { KEY_DOWN } else { KEY_UP })
        .child(cap)
}

/// A chord's keys at rest in their well, lit from above so its top edge falls into shadow.
/// Keys a new chord adds rise into the well and keys it drops leave through its top.
pub fn tray(p: Paint, key: &str, caps: &[String]) -> El {
    let k = p.k;
    let c = &k.c;
    let (top, bottom, rim, well) = if k.dark {
        (
            c.bg_deep,
            c.bg_base.mix(c.layer_01, 0.7),
            k.light(0.07),
            0.6,
        )
    } else {
        (
            c.layer_04.mix(c.layer_03, 0.35),
            c.layer_03.mix(c.layer_02, 0.6),
            k.dark_a(0.07),
            0.14,
        )
    };
    // Concentric with the caps inside it.
    let radius = FACE_RADIUS + RIM + TRAY_PADDING;
    row()
        .key(key)
        .padding(TRAY_PADDING)
        .gap(KEY_GAP)
        .center_y()
        .radius(radius)
        .continuous_corners()
        .clip_rounded(radius)
        .fill(bottom)
        .gradient(top, bottom)
        .border(p.hair.max(0.5), rim)
        .box_shadow(Shadow {
            y: 1.0,
            blur: 2.5,
            color: k.dark_a(well),
            inset: true,
            ..Shadow::default()
        })
        .children(
            caps.iter()
                .enumerate()
                .map(|(index, cap)| keycap(p, index, cap, false).transition(roll())),
        )
}

/// In from the bottom, out through the top, inside the parent's clip.
pub fn roll() -> Transition {
    Transition::push(Edge::Bottom)
}
