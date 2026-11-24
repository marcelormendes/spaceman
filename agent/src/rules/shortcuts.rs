//! `[shortcuts]` as Settings › Shortcuts edits it: one binding at a time, in place, so comments
//! and every other binding survive. A built-in shortcut moved to a new chord leaves `none` on
//! its old one, and moving it back home removes both.

use spaceman::shortcuts::{self, Chord, Command, Modifiers, Shortcuts};
use toml_edit::{Item, Table, TableLike, value};

use super::Doc;

impl Doc {
    /// `[shortcuts]` as Spaceman reads it, or the built-ins while the file has an error.
    pub fn shortcuts(&self) -> Shortcuts {
        spaceman::config::Config::parse(&self.source())
            .map(|config| config.shortcuts().clone())
            .unwrap_or_default()
    }

    /// Whether `[shortcuts]` changes anything from the built-in shortcuts.
    pub fn has_custom_shortcuts(&self) -> bool {
        self.shortcuts_table()
            .is_some_and(|table| !table.is_empty())
    }

    /// Gives `command` (in `spaceman ctl` syntax) the one chord `chord`, or none at all.
    /// `default` is the chord its built-in shortcut uses, if it has one.
    pub fn set_shortcut(&mut self, command: &str, default: Option<Chord>, chord: Option<Chord>) {
        let Ok(Some(target)) = shortcuts::parse_command(command) else {
            return;
        };
        let defaults = self.shortcuts().defaults;
        let default = default.filter(|_| defaults);
        let bindings = self.bindings_mut();
        let ours = bound_keys(bindings, |bound| bound == Some(&target));
        for key in ours {
            bindings.remove(&key);
        }
        if let Some(chord) = chord {
            remove_chord(bindings, chord);
            if default != Some(chord) {
                bindings.insert(&chord.source(), value(command));
            }
        }
        // The built-in shortcut keeps its chord unless something claims it.
        if let Some(default) = default
            && chord != Some(default)
            && find_chord(bindings, default).is_none()
        {
            bindings.insert(&default.source(), value("none"));
        }
        self.tidy_shortcuts();
    }

    /// Sets the modifiers every built-in shortcut holds. A built-in shortcut unbound with
    /// `none` stays unbound on its new chord; fixed chords (such as Scratchpad's `alt+\`)
    /// stay put because they do not follow `modifier`.
    pub fn set_shortcut_modifier(&mut self, modifier: Modifiers) {
        let shortcuts = self.shortcuts();
        let old = shortcuts.modifier;
        if old == modifier {
            return;
        }
        if shortcuts.defaults {
            let named = self.named_workspaces();
            let bindings = self.bindings_mut();
            let unbound = bound_keys(bindings, |bound| bound.is_none());
            for key in unbound {
                let Ok(chord) = shortcuts::parse_chord(&key) else {
                    continue;
                };
                if !shortcuts::is_built_in_chord(chord, old, &named) {
                    continue;
                }
                let modifiers = if chord.modifiers.shift {
                    modifier.with_shift()
                } else {
                    modifier
                };
                bindings.remove(&key);
                let moved = Chord { modifiers, ..chord };
                if find_chord(bindings, moved).is_none() {
                    bindings.insert(&moved.source(), value("none"));
                }
            }
        }
        let table = self.shortcuts_table_mut();
        if modifier == Modifiers::OPTION {
            table.remove("modifier");
        } else {
            table.insert("modifier", value(modifier.source()));
        }
        self.tidy_shortcuts();
    }

    /// Back to the built-in shortcuts: `[shortcuts]` leaves the file.
    pub fn reset_shortcuts(&mut self) {
        self.doc.remove("shortcuts");
    }

    fn shortcuts_table(&self) -> Option<&dyn TableLike> {
        self.doc.get("shortcuts").and_then(Item::as_table_like)
    }

    fn shortcuts_table_mut(&mut self) -> &mut Table {
        table_at(self.doc.entry("shortcuts").or_insert_with(implicit_table))
    }

    fn bindings_mut(&mut self) -> &mut Table {
        table_at(
            self.shortcuts_table_mut()
                .entry("bindings")
                .or_insert(Item::Table(Table::new())),
        )
    }

    /// Drops a `[shortcuts.bindings]` or `[shortcuts]` table an edit emptied.
    fn tidy_shortcuts(&mut self) {
        let Some(table) = self
            .doc
            .get_mut("shortcuts")
            .and_then(Item::as_table_like_mut)
        else {
            return;
        };
        if table
            .get("bindings")
            .and_then(Item::as_table_like)
            .is_some_and(TableLike::is_empty)
        {
            table.remove("bindings");
        }
        if table.is_empty() {
            self.doc.remove("shortcuts");
        }
    }
}

/// `item` as a standard table: an inline one becomes a table section, anything else a new one.
fn table_at(item: &mut Item) -> &mut Table {
    if let Item::Value(toml_edit::Value::InlineTable(inline)) = item {
        *item = Item::Table(std::mem::take(inline).into_table());
    }
    if !item.is_table() {
        *item = Item::Table(Table::new());
    }
    let Item::Table(table) = item else {
        unreachable!("just made a table")
    };
    table
}

fn implicit_table() -> Item {
    let mut table = Table::new();
    table.set_implicit(true);
    Item::Table(table)
}

/// The keys of every binding whose command passes `keep` (`None` for `none`).
fn bound_keys(bindings: &dyn TableLike, keep: impl Fn(Option<&Command>) -> bool) -> Vec<String> {
    bindings
        .iter()
        .filter(|(_, item)| {
            item.as_str()
                .and_then(|source| shortcuts::parse_command(source).ok())
                .is_some_and(|command| keep(command.as_ref()))
        })
        .map(|(key, _)| key.to_owned())
        .collect()
}

/// The key that spells `chord`, however it is written (`alt+h`, `Option + H`).
fn find_chord(bindings: &dyn TableLike, chord: Chord) -> Option<String> {
    bindings
        .iter()
        .map(|(key, _)| key)
        .find(|key| shortcuts::parse_chord(key).is_ok_and(|parsed| parsed == chord))
        .map(str::to_owned)
}

fn remove_chord(bindings: &mut dyn TableLike, chord: Chord) {
    if let Some(key) = find_chord(bindings, chord) {
        bindings.remove(&key);
    }
}

#[cfg(test)]
mod tests;
