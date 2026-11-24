use spaceman::shortcuts::parse_chord;

use super::*;

const SOURCE: &str = r#"# Spaceman configuration.
version = 1

[[rules]]
bundle_id = "anomaly.hex"
action = "float"

# My keys.
[shortcuts.bindings]
# Arrows move windows.
"alt+left" = "move-direction left"
"#;

fn chord(source: &str) -> Chord {
    parse_chord(source).unwrap()
}

fn bound(doc: &Doc, source: &str) -> Option<Option<String>> {
    doc.shortcuts()
        .binding(chord(source))
        .map(|command| command.map(Command::description))
}

fn saved(doc: &Doc) -> Doc {
    let source = doc.source();
    spaceman::config::Config::parse(&source).expect("Spaceman reads the edit");
    Doc::parse(&source)
}

#[test]
fn rebinding_a_built_in_keeps_comments_and_unbinds_its_old_chord() {
    let mut doc = Doc::parse(SOURCE);
    doc.set_shortcut(
        "focus left",
        Some(chord("alt+h")),
        Some(chord("ctrl+alt+h")),
    );
    let doc = saved(&doc);
    let source = doc.source();
    assert!(source.contains("# My keys."), "{source}");
    assert!(source.contains("# Arrows move windows."), "{source}");
    assert!(
        source.contains(r#""alt+left" = "move-direction left""#),
        "{source}"
    );
    assert!(
        source.contains(r#""ctrl+alt+h" = "focus left""#),
        "{source}"
    );
    assert_eq!(bound(&doc, "alt+h"), Some(None));
    assert_eq!(doc.rules().len(), 1);
    assert!(doc.has_custom_shortcuts());
}

#[test]
fn moving_a_built_in_back_home_removes_both_bindings() {
    let mut doc = Doc::parse("version = 1\n");
    doc.set_shortcut(
        "focus left",
        Some(chord("alt+h")),
        Some(chord("ctrl+alt+h")),
    );
    doc.set_shortcut("focus left", Some(chord("alt+h")), Some(chord("alt+h")));
    assert_eq!(doc.source(), "version = 1\n");
    assert!(!doc.has_custom_shortcuts());
}

#[test]
fn unbinding_leaves_none_on_the_built_in_chord() {
    let mut doc = Doc::parse("version = 1\n");
    doc.set_shortcut("float", Some(chord("alt+shift+space")), None);
    let doc = saved(&doc);
    assert_eq!(bound(&doc, "alt+shift+space"), Some(None));
    assert_eq!(doc.shortcuts().bindings.len(), 1);
    assert!(
        doc.source().contains("[shortcuts.bindings]") && !doc.source().contains("[shortcuts]\n"),
        "{}",
        doc.source()
    );
}

#[test]
fn unbinding_a_custom_command_removes_its_binding() {
    let mut doc = Doc::parse(SOURCE);
    doc.set_shortcut("move-direction left", None, None);
    let doc = saved(&doc);
    assert!(!doc.has_custom_shortcuts(), "{}", doc.source());
    assert!(doc.source().contains("action = \"float\""));
}

#[test]
fn taking_a_chord_that_holds_none_replaces_it_whatever_its_spelling() {
    let mut doc = Doc::parse("version = 1\n[shortcuts.bindings]\n\"Option + H\" = \"none\"\n");
    doc.set_shortcut("balance", None, Some(chord("alt+h")));
    let doc = saved(&doc);
    assert_eq!(bound(&doc, "alt+h"), Some(Some("balance layout".into())));
    assert_eq!(doc.shortcuts().bindings.len(), 1);
}

#[test]
fn a_command_keeps_one_chord() {
    let mut doc = Doc::parse(
        "version = 1\n[shortcuts.bindings]\n\"alt+b\" = \"balance\"\n\"ctrl+alt+b\" = \"balance\"\n",
    );
    doc.set_shortcut("balance", None, Some(chord("cmd+alt+b")));
    let doc = saved(&doc);
    assert_eq!(doc.shortcuts().bindings.len(), 1);
    assert_eq!(
        bound(&doc, "cmd+alt+b"),
        Some(Some("balance layout".into()))
    );
}

#[test]
fn modifier_moves_unbound_built_ins_with_it() {
    let mut doc = Doc::parse(
        "version = 1\n[shortcuts.bindings]\n\"alt+h\" = \"none\"\n\"alt+shift+h\" = \"none\"\n\"alt+b\" = \"balance\"\n",
    );
    doc.set_shortcut_modifier(Modifiers {
        control: true,
        option: true,
        ..Modifiers::default()
    });
    let doc = saved(&doc);
    let shortcuts = doc.shortcuts();
    assert!(shortcuts.modifier.control && shortcuts.modifier.option);
    assert_eq!(bound(&doc, "ctrl+alt+h"), Some(None));
    assert_eq!(bound(&doc, "ctrl+alt+shift+h"), Some(None));
    assert_eq!(bound(&doc, "alt+h"), None);
    // A binding is an exact chord, so it stays where it was.
    assert_eq!(bound(&doc, "alt+b"), Some(Some("balance layout".into())));

    let mut doc = doc;
    doc.set_shortcut_modifier(Modifiers::OPTION);
    let doc = saved(&doc);
    assert!(!doc.source().contains("modifier"), "{}", doc.source());
    assert_eq!(bound(&doc, "alt+h"), Some(None));
}

#[test]
fn modifier_keeps_fixed_scratchpad_unbinds_in_place() {
    let mut doc = Doc::parse(
        "version = 1\n[labs]\nscratchpad = true\n[[workspaces]]\nworkspace = \"A\"\nmonitor = \"main\"\n[shortcuts.bindings]\n'alt+\\' = 'none'\n'alt+shift+\\' = 'none'\n'alt+h' = 'none'\n'alt+a' = 'none'\n",
    );
    let ctrl_alt = Modifiers {
        control: true,
        option: true,
        ..Modifiers::default()
    };
    doc.set_shortcut_modifier(ctrl_alt);
    let doc = saved(&doc);
    // Fixed Scratchpad chords stay on Option+\ and Option+Shift+\.
    assert_eq!(bound(&doc, "alt+\\"), Some(None));
    assert_eq!(bound(&doc, "alt+shift+\\"), Some(None));
    assert_eq!(bound(&doc, "ctrl+alt+\\"), None);
    assert_eq!(bound(&doc, "ctrl+alt+shift+\\"), None);
    // Built-in layout and enabled named workspace unbinds still follow the base modifier.
    assert_eq!(bound(&doc, "ctrl+alt+h"), Some(None));
    assert_eq!(bound(&doc, "ctrl+alt+a"), Some(None));
    assert_eq!(bound(&doc, "alt+h"), None);
    assert_eq!(bound(&doc, "alt+a"), None);
}

#[test]
fn reset_removes_the_table_and_keeps_everything_else() {
    let mut doc = Doc::parse(SOURCE);
    doc.set_shortcut_modifier(Modifiers {
        control: true,
        option: true,
        ..Modifiers::default()
    });
    doc.reset_shortcuts();
    let doc = saved(&doc);
    let source = doc.source();
    assert!(!source.contains("shortcuts"), "{source}");
    assert!(source.contains("# Spaceman configuration."), "{source}");
    assert_eq!(doc.shortcuts(), Shortcuts::default());
}
