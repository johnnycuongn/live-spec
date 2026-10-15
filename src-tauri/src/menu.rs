//! The app menu. Tauri's default menu binds ⌘W to Close Window, so the whole menu is
//! replaced. Our own items (`livespec_core::menu`) carry the frontend's action ids
//! and go to the focused window as `menu { id }`; that window's `useActions` runs them.
//! Predefined items stay native. The Edit menu keeps Undo, Redo, Cut, Copy, Paste and
//! Select All: without them the draft `<textarea>` loses those shortcuts on macOS.
//! Services, Hide, Hide Others and Show All (macOS), and Minimize and Zoom, are kept from
//! Tauri's default menu, so ⌘H and ⌘M keep working.

use livespec_core::menu::{self as spec, MenuItemSpec};
use tauri::menu::{
    AboutMetadata, Menu, MenuBuilder, MenuEvent, MenuItem, MenuItemBuilder, Submenu, SubmenuBuilder, WINDOW_SUBMENU_ID,
};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::events;

/// The menu bar. `Builder::menu` calls this once at startup.
pub fn build(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let info = app.package_info();
    let about =
        AboutMetadata { name: Some(info.name.clone()), version: Some(info.version.to_string()), ..Default::default() };
    let settings = item(app, &spec::SETTINGS)?;
    #[cfg(target_os = "macos")]
    let app_menu = SubmenuBuilder::new(app, &info.name)
        .about(Some(about))
        .separator()
        .item(&settings)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;
    #[cfg(not(target_os = "macos"))]
    let app_menu =
        SubmenuBuilder::new(app, &info.name).about(Some(about)).separator().item(&settings).separator().quit().build()?;
    let file = groups(app, SubmenuBuilder::new(app, "File"), &spec::FILE)?;
    let edit = SubmenuBuilder::new(app, "Edit").undo().redo().separator().cut().copy().paste().select_all().build()?;
    // The id makes macOS list the open windows here and add Bring All to Front.
    let window = groups(
        app,
        SubmenuBuilder::with_id(app, WINDOW_SUBMENU_ID, "Window").minimize().maximize().separator(),
        &spec::WINDOW,
    )?;
    MenuBuilder::new(app).items(&[&app_menu, &file, &edit, &window]).build()
}

fn item(app: &AppHandle, spec: &MenuItemSpec) -> tauri::Result<MenuItem<Wry>> {
    MenuItemBuilder::with_id(spec.id, spec.label).accelerator(spec.accelerator).build(app)
}

/// Appends `groups` to `menu`, with a separator between groups.
fn groups<'m>(
    app: &'m AppHandle,
    mut menu: SubmenuBuilder<'m, Wry, AppHandle>,
    groups: &[&[MenuItemSpec]],
) -> tauri::Result<Submenu<Wry>> {
    for (k, group) in groups.iter().enumerate() {
        if k > 0 {
            menu = menu.separator();
        }
        for spec in group.iter() {
            menu = menu.item(&item(app, spec)?);
        }
    }
    menu.build()
}

/// Registered with `Builder::on_menu_event`. A custom item goes to the focused window
/// only (`emit_to` a label reaches that window's window-scoped listener, `onMenu`).
/// `get_focused_window` needs Tauri's `unstable` feature, so the focused window is found
/// by asking each one.
pub fn on_event(app: &AppHandle, event: MenuEvent) {
    let id: &str = event.id().as_ref();
    if !spec::is_action(id) {
        return; // a predefined item; the OS already ran it
    }
    let windows = app.webview_windows();
    let focused = windows.iter().find(|(_, w)| w.is_focused().unwrap_or(false)).map(|(label, _)| label.clone());
    // ⌘N still works with every window minimized; the other items need the window they act on.
    let target = focused.or_else(|| if id == "new-window" { windows.into_keys().min() } else { None });
    if let Some(label) = target {
        let _ = app.emit_to(label.as_str(), events::MENU, events::MenuEvent { id: id.to_owned() });
    }
}
