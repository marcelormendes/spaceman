use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    DisplaySelector, ObservedWindow, WorkspaceId, Workspaces,
    spacing::{Spacing, SpacingKey},
};

#[derive(Clone, Copy, Debug)]
struct AppSettings {
    hud: bool,
    hud_style: HudStyle,
    update_channel: UpdateChannel,
    spacing: Spacing,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            hud: true,
            hud_style: HudStyle::default(),
            update_channel: UpdateChannel::default(),
            spacing: Spacing::default(),
        }
    }
}

/// Which release channel the in-app updater follows: `settings.update_channel`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    /// Public releases.
    #[default]
    Stable,
    /// A build of every merge to `dev`.
    Dev,
}

impl UpdateChannel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Dev => "dev",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "stable" => Some(Self::Stable),
            "dev" => Some(Self::Dev),
            _ => None,
        }
    }
}

/// How the Workspace HUD draws each workspace in its row: `settings.hud_style`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HudStyle {
    /// Each workspace as its layout, with an app icon in every tile; the current one larger.
    Layouts,
    /// Each workspace as its number and the icons of its apps.
    #[default]
    Icons,
}

impl HudStyle {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Layouts => "layouts",
            Self::Icons => "icons",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "layouts" => Some(Self::Layouts),
            "icons" => Some(Self::Icons),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Config {
    settings: AppSettings,
    rules: Vec<WindowRule>,
    workspace_monitors: Vec<(WorkspaceId, DisplaySelector)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowDisposition {
    Managed,
    Floating,
    Route(WorkspaceId),
    /// Never tiled, owned, moved, or parked. For windows whose applications
    /// insist on managing them themselves.
    Ignored,
}

#[derive(Clone, Debug)]
struct WindowRule {
    matcher: WindowMatcher,
    action: WindowDisposition,
}

#[derive(Clone, Debug)]
struct WindowMatcher {
    bundle_id: Option<String>,
    title: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    version: u32,
    #[serde(default)]
    settings: RawSettings,
    #[serde(default)]
    rules: Vec<RawWindowRule>,
    #[serde(default)]
    workspaces: Vec<RawWorkspace>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSettings {
    #[serde(default = "default_hud")]
    hud: bool,
    #[serde(default)]
    hud_style: HudStyle,
    #[serde(default)]
    update_channel: UpdateChannel,
    #[serde(default)]
    single_window_max_width: i64,
    #[serde(default)]
    gaps: RawGaps,
}

impl Default for RawSettings {
    fn default() -> Self {
        Self {
            hud: true,
            hud_style: HudStyle::default(),
            update_channel: UpdateChannel::default(),
            single_window_max_width: 0,
            gaps: RawGaps::default(),
        }
    }
}

/// `[settings.gaps]`, read wide so out-of-range values get a clear message.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGaps {
    #[serde(default)]
    inner: i64,
    #[serde(default)]
    top: i64,
    #[serde(default)]
    bottom: i64,
    #[serde(default)]
    left: i64,
    #[serde(default)]
    right: i64,
}

impl RawSettings {
    fn spacing(&self) -> Result<Spacing, String> {
        let gaps = &self.gaps;
        let mut spacing = Spacing::default();
        for (key, value) in [
            (SpacingKey::Inner, gaps.inner),
            (SpacingKey::Top, gaps.top),
            (SpacingKey::Bottom, gaps.bottom),
            (SpacingKey::Left, gaps.left),
            (SpacingKey::Right, gaps.right),
            (
                SpacingKey::SingleWindowMaxWidth,
                self.single_window_max_width,
            ),
        ] {
            key.set(&mut spacing, key.check(value)?);
        }
        Ok(spacing)
    }
}

const fn default_hud() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWindowRule {
    bundle_id: Option<String>,
    title: Option<String>,
    action: String,
    workspace: Option<WorkspaceId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWorkspace {
    workspace: WorkspaceId,
    monitor: String,
}

impl Config {
    #[must_use]
    #[cfg(test)]
    pub fn has_rules(&self) -> bool {
        !self.rules.is_empty()
    }

    pub fn parse(source: &str) -> Result<Self, String> {
        let raw: RawConfig = toml::from_str(source).map_err(|error| error.to_string())?;
        if raw.version != 1 {
            return Err(format!(
                "unsupported config version {}; expected 1",
                raw.version
            ));
        }
        let mut rules = Vec::with_capacity(raw.rules.len());
        for rule in raw.rules {
            if rule.bundle_id.is_none() && rule.title.is_none() {
                return Err("window rule must contain at least one matcher".into());
            }
            let action = match (rule.action.as_str(), rule.workspace) {
                ("float", None) => WindowDisposition::Floating,
                ("ignore", None) => WindowDisposition::Ignored,
                ("route", Some(workspace)) => WindowDisposition::Route(workspace),
                ("float", Some(_)) => return Err("float rule cannot specify workspace".into()),
                ("ignore", Some(_)) => return Err("ignore rule cannot specify workspace".into()),
                ("route", None) => return Err("route rule requires workspace".into()),
                (action, _) => return Err(format!("unknown window rule action {action:?}")),
            };
            rules.push(WindowRule {
                matcher: WindowMatcher {
                    bundle_id: rule.bundle_id,
                    title: rule.title,
                },
                action,
            });
        }
        let mut workspace_monitors = Vec::with_capacity(raw.workspaces.len());
        for workspace in raw.workspaces {
            let id = workspace.workspace;
            if workspace.monitor.trim().is_empty() {
                return Err(format!("workspace {id} monitor cannot be empty"));
            }
            if workspace_monitors
                .iter()
                .any(|(existing, _)| *existing == id)
            {
                return Err(format!("workspace {} is configured more than once", id));
            }
            workspace_monitors.push((id, DisplaySelector::parse(&workspace.monitor)));
        }
        Ok(Self {
            settings: AppSettings {
                hud: raw.settings.hud,
                hud_style: raw.settings.hud_style,
                update_channel: raw.settings.update_channel,
                spacing: raw.settings.spacing()?,
            },
            rules,
            workspace_monitors,
        })
    }

    #[must_use]
    pub const fn hud(&self) -> bool {
        self.settings.hud
    }

    #[must_use]
    pub const fn hud_style(&self) -> HudStyle {
        self.settings.hud_style
    }

    #[must_use]
    pub const fn update_channel(&self) -> UpdateChannel {
        self.settings.update_channel
    }

    /// Gaps between tiles, outer screen padding and the lone-window width cap.
    #[must_use]
    pub const fn spacing(&self) -> Spacing {
        self.settings.spacing
    }

    pub fn load() -> Result<Self, String> {
        let path = config_path().ok_or("HOME is unavailable")?;
        match std::fs::read_to_string(&path) {
            Ok(source) => parse_path(&path, &source),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("failed to read {}: {error}", path.display())),
        }
    }

    fn matching_rule(&self, bundle_id: &str, window: &ObservedWindow) -> Option<&WindowRule> {
        self.rules
            .iter()
            .find(|rule| rule.matcher.matches(bundle_id, window))
    }

    #[must_use]
    pub fn disposition(&self, bundle_id: &str, window: &ObservedWindow) -> WindowDisposition {
        self.matching_rule(bundle_id, window)
            .map_or(WindowDisposition::Managed, |rule| rule.action)
    }

    #[must_use]
    pub fn route_matches_title(&self, bundle_id: &str, window: &ObservedWindow) -> bool {
        self.matching_rule(bundle_id, window).is_some_and(|rule| {
            matches!(rule.action, WindowDisposition::Route(_)) && rule.matcher.title.is_some()
        })
    }

    /// True when any rule that can match `bundle_id` inspects the window title.
    #[must_use]
    pub fn has_title_rule_for(&self, bundle_id: &str) -> bool {
        self.rules.iter().any(|rule| {
            rule.matcher.title.is_some()
                && rule
                    .matcher
                    .bundle_id
                    .as_deref()
                    .is_none_or(|value| value == bundle_id)
        })
    }

    #[must_use]
    pub fn has_bundle_route(&self, bundle_id: &str) -> bool {
        self.rules
            .iter()
            .find(|rule| {
                rule.matcher.title.is_none()
                    && rule
                        .matcher
                        .bundle_id
                        .as_deref()
                        .is_some_and(|value| value == bundle_id)
            })
            .is_some_and(|rule| matches!(rule.action, WindowDisposition::Route(_)))
    }

    pub fn configure_workspaces(&self, workspaces: &mut Workspaces) {
        for workspace in self.named_workspaces() {
            workspaces.enable(workspace);
        }
        for workspace in WorkspaceId::all() {
            if !workspaces.is_enabled(workspace) {
                continue;
            }
            let target = self
                .workspace_monitors
                .iter()
                .find(|(id, _)| *id == workspace)
                .map_or_else(
                    || Workspaces::default_assignment(workspace),
                    |(_, selector)| selector.clone(),
                );
            workspaces.set_assignment(workspace, target);
        }
    }

    pub fn set_workspace_monitor(&mut self, workspace: WorkspaceId, selector: DisplaySelector) {
        let should_clear = selector == Workspaces::default_assignment(workspace)
            && (workspace.number().is_some() || self.has_workspace_route(workspace));
        if should_clear {
            self.workspace_monitors
                .retain(|(existing, _)| *existing != workspace);
            return;
        }
        if let Some((_, existing)) = self
            .workspace_monitors
            .iter_mut()
            .find(|(existing, _)| *existing == workspace)
        {
            *existing = selector;
        } else {
            self.workspace_monitors.push((workspace, selector));
        }
    }

    fn has_workspace_route(&self, workspace: WorkspaceId) -> bool {
        self.rules
            .iter()
            .any(|rule| rule.action == WindowDisposition::Route(workspace))
    }

    pub fn named_workspaces(&self) -> impl Iterator<Item = WorkspaceId> + '_ {
        self.workspace_monitors
            .iter()
            .map(|(workspace, _)| *workspace)
            .chain(self.rules.iter().filter_map(|rule| match rule.action {
                WindowDisposition::Route(workspace) => Some(workspace),
                _ => None,
            }))
            .filter(|workspace| workspace.letter().is_some())
    }
}

impl WindowMatcher {
    fn matches(&self, bundle_id: &str, window: &ObservedWindow) -> bool {
        self.bundle_id
            .as_deref()
            .is_none_or(|value| value == bundle_id)
            && self
                .title
                .as_deref()
                .is_none_or(|value| window.title.as_deref() == Some(value))
    }
}

/// Wall-clock duration since the Unix epoch.
#[must_use]
#[expect(
    clippy::disallowed_methods,
    reason = "config::now is the central wall-clock adapter"
)]
pub fn now() -> std::time::Duration {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
}

/// Optional `SPACEMAN_MANAGED_DISPLAY` override read at startup.
#[expect(
    clippy::disallowed_methods,
    reason = "startup environment variables are read in config.rs"
)]
pub fn managed_display_env() -> Result<Option<String>, String> {
    match std::env::var("SPACEMAN_MANAGED_DISPLAY") {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err("SPACEMAN_MANAGED_DISPLAY must be valid UTF-8".into())
        }
    }
}

/// When the config file last changed, or `None` while it doesn't exist.
#[must_use]
pub fn modified() -> Option<std::time::SystemTime> {
    std::fs::metadata(config_path()?).ok()?.modified().ok()
}

pub fn config_path() -> Option<PathBuf> {
    std::env::var_os("SPACEMAN_CONFIG")
        .map(PathBuf::from)
        .or_else(|| application_support_path("config.toml"))
}

pub fn application_support_path(file: &str) -> Option<PathBuf> {
    std::env::var_os("SPACEMAN_APPLICATION_SUPPORT")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join("Library/Application Support/Spaceman"))
        })
        .map(|directory| directory.join(file))
}

pub fn load_path(path: &Path) -> Result<Config, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    parse_path(path, &source)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EditableConfig {
    version: u32,
    #[serde(default, skip_serializing_if = "EditableSettings::is_empty")]
    settings: EditableSettings,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    rules: Vec<EditableRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    workspaces: Vec<EditableWorkspace>,
}

impl Default for EditableConfig {
    fn default() -> Self {
        Self {
            version: 1,
            settings: EditableSettings::default(),
            rules: Vec::new(),
            workspaces: Vec::new(),
        }
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EditableSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hud: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hud_style: Option<HudStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    update_channel: Option<UpdateChannel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    single_window_max_width: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gaps: Option<toml::Table>,
}

impl EditableSettings {
    const fn is_empty(&self) -> bool {
        self.hud.is_none()
            && self.hud_style.is_none()
            && self.update_channel.is_none()
            && self.single_window_max_width.is_none()
            && self.gaps.is_none()
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EditableRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace: Option<WorkspaceId>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EditableWorkspace {
    workspace: WorkspaceId,
    monitor: String,
}

/// Atomically updates `workspace`'s display assignment in `path`, clearing the
/// `[[workspaces]]` override when it returns to the built-in default (while
/// keeping named workspaces that have no route rule declared).
pub fn save_workspace_monitor_at(
    path: &Path,
    workspace: WorkspaceId,
    selector: &DisplaySelector,
) -> Result<Config, String> {
    let mut doc = match std::fs::read_to_string(path) {
        Ok(source) => {
            parse_path(path, &source)?;
            toml::from_str::<EditableConfig>(&source)
                .map_err(|error| format!("{}: {error}", path.display()))?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => EditableConfig::default(),
        Err(error) => return Err(format!("failed to read {}: {error}", path.display())),
    };
    let referenced_by_rule = doc
        .rules
        .iter()
        .any(|rule| rule.workspace == Some(workspace));
    let clear = *selector == Workspaces::default_assignment(workspace)
        && (workspace.number().is_some() || referenced_by_rule);
    if clear {
        doc.workspaces.retain(|entry| entry.workspace != workspace);
    } else {
        let monitor = selector.as_str().to_owned();
        if let Some(existing) = doc
            .workspaces
            .iter_mut()
            .find(|entry| entry.workspace == workspace)
        {
            existing.monitor = monitor;
        } else {
            doc.workspaces
                .push(EditableWorkspace { workspace, monitor });
        }
    }
    let rendered = toml::to_string_pretty(&doc)
        .map_err(|error| format!("failed to format config.toml: {error}"))?;
    let parsed = Config::parse(&rendered)
        .map_err(|error| format!("refusing to write invalid config.toml: {error}"))?;
    crate::state_file::write_atomic(path, rendered.as_bytes())
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    Ok(parsed)
}

fn parse_path(path: &Path, source: &str) -> Result<Config, String> {
    Config::parse(source).map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use crate::{AppInstanceId, PhysicalWindowId, Rect};

    use super::*;

    fn window(title: Option<&str>) -> ObservedWindow {
        ObservedWindow {
            id: PhysicalWindowId {
                app: AppInstanceId {
                    pid: 1,
                    launched_at: 1,
                },
                window_id: 1,
            },
            frame: Rect::default(),
            resizable: true,
            title: title.map(std::sync::Arc::from),
        }
    }

    #[test]
    fn ordered_rules_match_all_supplied_fields() {
        let config = Config::parse(
            r#"
version = 1

[[rules]]
bundle_id = "com.example.App"
title = "Settings"
action = "float"

[[rules]]
bundle_id = "com.example.App"
action = "route"
workspace = 7
"#,
        )
        .unwrap();

        assert_eq!(
            config.disposition("com.example.App", &window(Some("Settings"))),
            WindowDisposition::Floating
        );
        assert_eq!(
            config.disposition("com.example.App", &window(Some("Document"))),
            WindowDisposition::Route(WorkspaceId::new(7).unwrap())
        );
        assert!(config.has_title_rule_for("com.example.App"));
        assert!(!config.has_title_rule_for("com.example.Other"));
    }

    #[test]
    fn rejects_invalid_rules_and_workspaces() {
        assert!(Config::parse("version = 2").is_err());
        assert!(Config::parse("version = 1\n[[rules]]\naction = 'float'").is_err());
        assert!(
            Config::parse(
                "version = 1\n[[rules]]\nbundle_id = 'x'\naction = 'route'\nworkspace = 11"
            )
            .is_err()
        );
        assert!(
            Config::parse(
                "version = 1\n[[rules]]\nbundle_id = 'x'\naction = 'route'\nworkspace = 'AA'"
            )
            .is_err()
        );
    }

    #[test]
    fn accepts_named_workspace_routes_and_assignments() {
        let config = Config::parse(
            r#"
version = 1
[[rules]]
bundle_id = "ai.opencode.desktop"
action = "route"
workspace = "D"
[[workspaces]]
workspace = "D"
monitor = "main"
"#,
        )
        .unwrap();
        assert_eq!(
            config.disposition("ai.opencode.desktop", &window(None)),
            WindowDisposition::Route(WorkspaceId::named('D').unwrap())
        );
        assert_eq!(
            config.named_workspaces().collect::<Vec<_>>(),
            vec![
                WorkspaceId::named('D').unwrap(),
                WorkspaceId::named('D').unwrap()
            ]
        );
    }

    #[test]
    fn rejects_obsolete_keyboard_mapping_override() {
        assert!(Config::parse("version = 1\nkey_mapping = 'dvorak'").is_err());
    }

    #[test]
    fn parses_hud_setting_with_enabled_default() {
        assert!(Config::default().hud());
        assert!(Config::parse("version = 1").unwrap().hud());
        assert!(Config::parse("version = 1\n[settings]\n").unwrap().hud());
        assert!(
            !Config::parse("version = 1\n[settings]\nhud = false\n")
                .unwrap()
                .hud()
        );
        assert!(Config::parse("version = 1\n[settings]\nunknown = true\n").is_err());
    }

    #[test]
    fn parses_hud_style_with_icons_default() {
        assert_eq!(Config::default().hud_style(), HudStyle::Icons);
        assert_eq!(
            Config::parse("version = 1").unwrap().hud_style(),
            HudStyle::Icons
        );
        assert_eq!(
            Config::parse("version = 1\n[settings]\nhud = false\n")
                .unwrap()
                .hud_style(),
            HudStyle::Icons
        );
        for style in [HudStyle::Layouts, HudStyle::Icons] {
            let source = format!(
                "version = 1\n[settings]\nhud_style = {:?}\n",
                style.as_str()
            );
            assert_eq!(Config::parse(&source).unwrap().hud_style(), style);
            assert_eq!(HudStyle::parse(style.as_str()), Some(style));
        }
        assert!(Config::parse("version = 1\n[settings]\nhud_style = \"grid\"\n").is_err());
        assert!(Config::parse("version = 1\n[settings]\nhud_style = true\n").is_err());
        assert_eq!(HudStyle::parse("Icons"), None);
    }

    #[test]
    fn parses_update_channel_with_stable_default() {
        assert_eq!(Config::default().update_channel(), UpdateChannel::Stable);
        assert_eq!(
            Config::parse("version = 1").unwrap().update_channel(),
            UpdateChannel::Stable
        );
        for channel in [UpdateChannel::Stable, UpdateChannel::Dev] {
            let source = format!(
                "version = 1\n[settings]\nupdate_channel = {:?}\n",
                channel.as_str()
            );
            assert_eq!(Config::parse(&source).unwrap().update_channel(), channel);
            assert_eq!(UpdateChannel::parse(channel.as_str()), Some(channel));
        }
        assert!(Config::parse("version = 1\n[settings]\nupdate_channel = \"beta\"\n").is_err());
    }

    #[test]
    fn save_workspace_monitor_persists_overrides_and_clears_built_in_defaults() {
        let dir = std::env::temp_dir().join(format!(
            "spaceman-config-save-{}-{}",
            std::process::id(),
            now().as_nanos()
        ));
        let path = dir.join("config.toml");

        let saved = save_workspace_monitor_at(&path, WorkspaceId::ONE, &DisplaySelector::Secondary)
            .unwrap();
        let mut workspaces = Workspaces::default();
        saved.configure_workspaces(&mut workspaces);
        assert_eq!(
            workspaces.assignment(WorkspaceId::ONE),
            &DisplaySelector::Secondary
        );
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("monitor = \"secondary\""), "{written}");

        let reset =
            save_workspace_monitor_at(&path, WorkspaceId::ONE, &DisplaySelector::Main).unwrap();
        reset.configure_workspaces(&mut workspaces);
        assert_eq!(
            workspaces.assignment(WorkspaceId::ONE),
            &DisplaySelector::Main
        );
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(!written.contains("[[workspaces]]"), "{written}");

        let named = WorkspaceId::named('D').unwrap();
        let kept = save_workspace_monitor_at(&path, named, &DisplaySelector::Main).unwrap();
        assert_eq!(kept.named_workspaces().collect::<Vec<_>>(), vec![named]);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("workspace = \"D\""), "{written}");

        drop(std::fs::remove_dir_all(dir));
    }

    #[test]
    fn spacing_defaults_to_none_and_parses_every_key() {
        assert!(Config::default().spacing().is_none());
        assert!(Config::parse("version = 1").unwrap().spacing().is_none());
        assert!(
            Config::parse("version = 1\n[settings]\nhud = false\n")
                .unwrap()
                .spacing()
                .is_none()
        );
        let config = Config::parse(
            r#"
version = 1
[settings]
single_window_max_width = 1600
[settings.gaps]
inner = 8
top = 32
bottom = 4
left = 0
right = 128
"#,
        )
        .unwrap();
        assert_eq!(
            config.spacing(),
            Spacing {
                inner: 8,
                top: 32,
                bottom: 4,
                left: 0,
                right: 128,
                single_window_max_width: 1600,
            }
        );
        let partial = Config::parse("version = 1\n[settings.gaps]\ninner = 12\n").unwrap();
        assert_eq!(partial.spacing().inner, 12);
        assert_eq!(partial.spacing().top, 0);
    }

    #[test]
    fn spacing_rejects_out_of_range_and_unknown_keys_with_clear_messages() {
        for (source, message) in [
            (
                "[settings.gaps]\ninner = -1",
                "settings.gaps.inner must be between 0 and 48 px, got -1",
            ),
            (
                "[settings.gaps]\ninner = 49",
                "settings.gaps.inner must be between 0 and 48 px, got 49",
            ),
            (
                "[settings.gaps]\ntop = 129",
                "settings.gaps.top must be between 0 and 128 px, got 129",
            ),
            (
                "[settings.gaps]\nright = -4",
                "settings.gaps.right must be between 0 and 128 px, got -4",
            ),
            (
                "[settings]\nsingle_window_max_width = 100",
                "settings.single_window_max_width must be 0 (off) or between 320 and 16384 px, got 100",
            ),
            (
                "[settings]\nsingle_window_max_width = 99999",
                "settings.single_window_max_width must be 0 (off) or between 320 and 16384 px, got 99999",
            ),
        ] {
            let error = Config::parse(&format!("version = 1\n{source}\n")).unwrap_err();
            assert_eq!(error, message);
        }
        assert!(Config::parse("version = 1\n[settings.gaps]\nouter = 4\n").is_err());
        assert!(Config::parse("version = 1\n[settings.gaps]\ninner = 1.5\n").is_err());
    }

    #[test]
    fn saving_a_workspace_monitor_keeps_spacing() {
        let dir = std::env::temp_dir().join(format!(
            "spaceman-config-spacing-{}-{}",
            std::process::id(),
            now().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "version = 1\n[settings]\nsingle_window_max_width = 1440\n[settings.gaps]\ninner = 6\nleft = 20\n",
        )
        .unwrap();
        let saved = save_workspace_monitor_at(&path, WorkspaceId::TWO, &DisplaySelector::Secondary)
            .unwrap();
        assert_eq!(saved.spacing().inner, 6);
        assert_eq!(saved.spacing().left, 20);
        assert_eq!(saved.spacing().single_window_max_width, 1440);
        drop(std::fs::remove_dir_all(dir));
    }
}
