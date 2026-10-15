//! The app menu's own items, as data. Each id is one of the frontend's `ActionId`s
//! (`src/lib/actions.ts`): the shell builds the native menu from these
//! (`src-tauri/src/menu.rs`) and sends a chosen item's id to the focused window as
//! `menu { id }`. Predefined items (About, Quit, Hide, the Edit menu, Minimize) aren't
//! listed; the OS runs them. Accelerators use Tauri's syntax: `CmdOrCtrl` is ⌘ on macOS
//! and Ctrl elsewhere, and `Ctrl` is the Control key (⌃) everywhere.

/// One custom menu item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuItemSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub accelerator: &'static str,
}

/// In the app menu, after About.
pub static SETTINGS: MenuItemSpec = MenuItemSpec { id: "settings", label: "Settings…", accelerator: "CmdOrCtrl+Comma" };

/// The File menu, as groups with a separator between them.
pub static FILE: [&[MenuItemSpec]; 3] = [
    &[
        MenuItemSpec { id: "new-tab", label: "New Tab", accelerator: "CmdOrCtrl+T" },
        MenuItemSpec { id: "new-window", label: "New Window", accelerator: "CmdOrCtrl+N" },
        MenuItemSpec { id: "open-project", label: "Open Project…", accelerator: "CmdOrCtrl+O" },
    ],
    &[MenuItemSpec { id: "reopen-tab", label: "Reopen Closed Tab", accelerator: "CmdOrCtrl+Shift+T" }],
    &[
        MenuItemSpec { id: "close-tab", label: "Close Tab", accelerator: "CmdOrCtrl+W" },
        MenuItemSpec { id: "close-window", label: "Close Window", accelerator: "CmdOrCtrl+Shift+W" },
    ],
];

/// The Window menu's own items; the shell puts the predefined Minimize and Zoom above them.
pub static WINDOW: [&[MenuItemSpec]; 2] = [
    &[
        MenuItemSpec { id: "next-tab", label: "Next Tab", accelerator: "Ctrl+Tab" },
        MenuItemSpec { id: "prev-tab", label: "Previous Tab", accelerator: "Ctrl+Shift+Tab" },
    ],
    &[MenuItemSpec { id: "toggle-sidebar", label: "Toggle Sidebar", accelerator: "CmdOrCtrl+Backslash" }],
];

/// Every custom item, in menu order.
pub fn items() -> impl Iterator<Item = &'static MenuItemSpec> {
    std::iter::once(&SETTINGS).chain(FILE.iter().chain(WINDOW.iter()).flat_map(|group| group.iter()))
}

/// Whether a menu event's id is one of ours, which the frontend handles. Predefined
/// items have ids of their own and are never forwarded.
pub fn is_action(id: &str) -> bool {
    items().any(|item| item.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn the_spec_items_and_shortcuts() {
        let got: Vec<(&str, &str)> = items().map(|i| (i.id, i.accelerator)).collect();
        assert_eq!(
            got,
            [
                ("settings", "CmdOrCtrl+Comma"),
                ("new-tab", "CmdOrCtrl+T"),
                ("new-window", "CmdOrCtrl+N"),
                ("open-project", "CmdOrCtrl+O"),
                ("reopen-tab", "CmdOrCtrl+Shift+T"),
                ("close-tab", "CmdOrCtrl+W"),
                ("close-window", "CmdOrCtrl+Shift+W"),
                ("next-tab", "Ctrl+Tab"),
                ("prev-tab", "Ctrl+Shift+Tab"),
                ("toggle-sidebar", "CmdOrCtrl+Backslash"),
            ]
        );
    }

    #[test]
    fn ids_and_shortcuts_are_unique() {
        let n = items().count();
        assert_eq!(items().map(|i| i.id).collect::<HashSet<_>>().len(), n);
        assert_eq!(items().map(|i| i.accelerator).collect::<HashSet<_>>().len(), n);
    }

    #[test]
    fn predefined_and_unknown_ids_are_not_forwarded() {
        assert!(is_action("close-tab"));
        for id in ["quit", "about", "hide", "copy", "", "Close-Tab"] {
            assert!(!is_action(id), "{id}");
        }
    }

    /// Cross-language contract: the menu's ids are exactly the frontend's `ActionId`s.
    #[test]
    fn every_item_is_a_frontend_action_and_every_action_has_an_item() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/lib/actions.ts");
        let ts = std::fs::read_to_string(path).expect("src/lib/actions.ts (Phase 2)");
        let union = ts.split("export type ActionId =").nth(1).expect("`export type ActionId =` in actions.ts");
        let union = union.split(';').next().unwrap_or_default();
        let mut actions: Vec<&str> = union.split('"').skip(1).step_by(2).collect();
        let mut ids: Vec<&str> = items().map(|i| i.id).collect();
        actions.sort_unstable();
        ids.sort_unstable();
        assert_eq!(ids, actions, "menu.rs ids vs ActionId in src/lib/actions.ts");
    }
}
