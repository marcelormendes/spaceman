//! General pane: Accessibility (only while missing), core preferences, updates, and
//! configuration.

use rune::{El, Tx, col};
use rune_kit::SegmentedControl;
use spaceman::{
    app::UiLaunchAtLogin,
    config::{HudStyle, MouseFollowsFocus},
};

use crate::{
    Ui,
    controls::{self, ButtonKind, Paint},
    view::scroll_body,
};

const HUD_MODES: [(&str, Option<HudStyle>); 3] = [
    ("Off", None),
    ("Icons", Some(HudStyle::Icons)),
    ("Layouts", Some(HudStyle::Layouts)),
];

fn hud_mode_index(enabled: bool, style: HudStyle) -> usize {
    let current = enabled.then_some(style);
    HUD_MODES
        .iter()
        .position(|(_, mode)| *mode == current)
        .unwrap_or(1)
}

const MOUSE_FOLLOW_MODES: [(&str, MouseFollowsFocus); 3] = [
    ("Off", MouseFollowsFocus::Off),
    ("Display", MouseFollowsFocus::Display),
    ("Window", MouseFollowsFocus::Window),
];

fn mouse_follow_index(mode: MouseFollowsFocus) -> usize {
    MOUSE_FOLLOW_MODES
        .iter()
        .position(|(_, candidate)| *candidate == mode)
        .unwrap_or(0)
}

pub fn pane(ui: &Ui, p: Paint) -> El {
    let c = &p.k.c;
    let mut sections = Vec::new();
    if let Some(agent) = &ui.agent {
        if !agent.accessibility {
            let access = controls::setting_row(
                p,
                "accessibility",
                spaceman::permission_guide::list_name(),
                Some(("Required to move and resize windows.".into(), c.warning.fg)),
                controls::button(
                    p,
                    "grant",
                    "Open System Settings",
                    ButtonKind::Primary,
                    |ui, _| ui.open_accessibility(),
                ),
            );
            sections.push(controls::card(p, "accessibility-card", vec![access]));
        }
        let focus = ui.highlight_feature.as_deref();
        let highlight = |el: El, key: &str| {
            if focus == Some(key) {
                el.fill(c.layer_02).border(p.hair, c.border_focus)
            } else {
                el
            }
        };
        let tiling = highlight(
            if agent.accessibility {
                controls::toggle_row(
                    p,
                    "tiling",
                    "Automatic tiling",
                    (!agent.automatic)
                        .then(|| ("Windows stay where you place them.".into(), p.muted())),
                    agent.automatic,
                    |ui| ui.toggle_automatic(),
                )
            } else {
                controls::setting_row(
                    p,
                    "tiling",
                    "Automatic tiling",
                    Some(("Requires Accessibility permission.".into(), p.muted())),
                    controls::status_dot(c.border_strong),
                )
            },
            "tiling",
        );
        let hud = highlight(
            controls::setting_row(
                p,
                "hud",
                "Workspace HUD",
                Some(("Shown when switching workspaces.".into(), p.muted())),
                SegmentedControl::new(
                    &HUD_MODES.map(|(label, _)| label.to_owned()),
                    Some(hud_mode_index(ui.doc.hud(), ui.doc.hud_style())),
                )
                .fit()
                .on_pick(|ui: &mut Ui, index, _: &mut Tx| {
                    if let Some((_, mode)) = HUD_MODES.get(index) {
                        ui.set_hud(*mode);
                    }
                })
                .build(p.k)
                .key("hud-mode-picker"),
            ),
            "hud",
        );
        let (on, description) = match &agent.launch_at_login {
            UiLaunchAtLogin::On => (true, None),
            UiLaunchAtLogin::Off => (false, None),
            UiLaunchAtLogin::NeedsApproval => (
                true,
                Some((
                    "Waiting for approval in System Settings › Login Items.".to_owned(),
                    c.warning.fg,
                )),
            ),
            UiLaunchAtLogin::Failed(error) => (false, Some((error.clone(), c.danger.fg))),
        };
        let login = highlight(
            controls::toggle_row(p, "login", "Launch at Login", description, on, |ui| {
                ui.toggle_launch_at_login();
            }),
            "login",
        );
        let mouse_follows_focus = highlight(
            controls::setting_row(
                p,
                "mouse-follows-focus",
                "Mouse follows focus",
                Some((
                    "Move the pointer on keyboard focus changes.".into(),
                    p.muted(),
                )),
                SegmentedControl::new(
                    &MOUSE_FOLLOW_MODES.map(|(label, _)| label.to_owned()),
                    Some(mouse_follow_index(ui.doc.mouse_follows_focus())),
                )
                .fit()
                .on_pick(|ui: &mut Ui, index, _: &mut Tx| {
                    if let Some((_, mode)) = MOUSE_FOLLOW_MODES.get(index) {
                        ui.edit(|doc| doc.set_mouse_follows_focus(*mode));
                    }
                })
                .build(p.k)
                .key("mouse-follows-focus-picker"),
            ),
            "mouse-follows-focus",
        );
        sections.push(controls::card(
            p,
            "agent",
            vec![tiling, hud, mouse_follows_focus, login],
        ));
    }
    sections.push(crate::updates::section(ui, p));
    let focus = ui.highlight_feature.as_deref();
    let highlight = |el: El, key: &str| {
        if focus == Some(key) {
            el.fill(c.layer_02).border(p.hair, c.border_focus)
        } else {
            el
        }
    };
    let config_file = highlight(
        controls::setting_row(
            p,
            "file",
            "config.toml",
            Some((ui.doc.display_path(), p.muted())),
            controls::button(
                p,
                "reveal",
                "Show in Finder",
                ButtonKind::Secondary,
                |_: &mut Ui, _: &mut Tx| {
                    if let Some(path) = spaceman::config::config_path() {
                        controls::reveal_in_finder(&path);
                    }
                },
            ),
        ),
        "config",
    );
    let setup_tour = highlight(
        controls::setting_row(
            p,
            "setup-tour",
            "Setup tour",
            None,
            controls::button(
                p,
                "run-tour",
                "Run Setup Tour…",
                ButtonKind::Secondary,
                |ui, _| ui.start_tour(),
            ),
        ),
        "setup-tour",
    );
    sections.push(controls::section(
        p,
        "config",
        "Configuration",
        vec![config_file, setup_tour],
    ));
    scroll_body(
        "general-body",
        col().key("sections").gap(24.0).children(sections),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_mode_picker_maps_off_icons_and_layouts() {
        assert_eq!(hud_mode_index(false, HudStyle::Icons), 0);
        assert_eq!(hud_mode_index(false, HudStyle::Layouts), 0);
        assert_eq!(hud_mode_index(true, HudStyle::Icons), 1);
        assert_eq!(hud_mode_index(true, HudStyle::Layouts), 2);
    }

    #[test]
    fn mouse_follow_picker_maps_off_display_and_window() {
        assert_eq!(mouse_follow_index(MouseFollowsFocus::Off), 0);
        assert_eq!(mouse_follow_index(MouseFollowsFocus::Display), 1);
        assert_eq!(mouse_follow_index(MouseFollowsFocus::Window), 2);
    }
}
