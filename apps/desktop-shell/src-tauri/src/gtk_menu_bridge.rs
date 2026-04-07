/// GTK D-Bus Menu Bridge.
///
/// When the focused app changes, queries `org.gtk.Menus` and converts the
/// menu structure into the `MenuGroup` format used by the global menu bar.
/// Only works for GTK apps that expose menus via GApplication's D-Bus API.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Listener};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value as ZValue};

use crate::menu_store::AppMenuStore;

/// Starts the GTK menu bridge.
///
/// Listens for `lunaris://toplevel-changed` and `lunaris://toplevel-added`
/// Tauri events. When the active app changes, spawns a blocking D-Bus
/// query on a background thread and registers the result in AppMenuStore.
pub fn start(app_handle: AppHandle, store: AppMenuStore) {
    let (tx, rx) = mpsc::channel::<String>();
    let last_app_id: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    // Listener: toplevel-changed
    let tx1 = tx.clone();
    app_handle.listen("lunaris://toplevel-changed", move |event| {
        if let Some(app_id) = extract_active_app_id(&event) {
            let _ = tx1.send(app_id);
        }
    });

    // Listener: toplevel-added (first window may be immediately active)
    let tx2 = tx.clone();
    app_handle.listen("lunaris://toplevel-added", move |event| {
        if let Some(app_id) = extract_active_app_id(&event) {
            let _ = tx2.send(app_id);
        }
    });

    // Background thread: processes focus changes and queries D-Bus.
    let app = app_handle.clone();
    std::thread::Builder::new()
        .name("gtk-menu-bridge".into())
        .spawn(move || bridge_thread(rx, app, store, last_app_id))
        .expect("failed to spawn gtk-menu-bridge thread");
}

/// Extracts app_id from a toplevel event payload, if active.
fn extract_active_app_id(event: &tauri::Event) -> Option<String> {
    let payload: Value = serde_json::from_str(event.payload()).ok()?;
    if payload.get("active")?.as_bool()? != true {
        return None;
    }
    Some(payload.get("app_id")?.as_str()?.to_string())
}

/// Background thread that processes focus change requests sequentially.
fn bridge_thread(
    rx: mpsc::Receiver<String>,
    app: AppHandle,
    store: AppMenuStore,
    last_app_id: Arc<Mutex<Option<String>>>,
) {
    // Lazily connect to the session bus.
    let conn = match Connection::session() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("gtk_menu_bridge: session bus unavailable: {e}");
            return;
        }
    };

    while let Ok(app_id) = rx.recv() {
        let mut last = last_app_id.lock().unwrap();
        if last.as_deref() == Some(&app_id) {
            continue;
        }

        // Unregister previous menu.
        if let Some(prev) = last.take() {
            store.lock().unwrap().remove(&prev);
            let _ = app.emit("lunaris://menu-unregistered", json!({ "app_id": prev }));
        }
        *last = Some(app_id.clone());
        drop(last);

        // Only query reverse-domain app_ids.
        if !app_id.contains('.') {
            continue;
        }

        match query_gtk_menus(&conn, &app_id) {
            Ok(menus) if !menus.is_empty() => {
                let items = json!(menus);
                store.lock().unwrap().insert(app_id.clone(), items.clone());
                let _ = app.emit(
                    "lunaris://menu-registered",
                    json!({ "app_id": app_id, "items": items }),
                );
                log::info!("gtk_menu_bridge: registered {} menu groups for {app_id}", menus.len());
            }
            Ok(_) => {
                log::debug!("gtk_menu_bridge: no menu items for {app_id}");
            }
            Err(e) => {
                log::debug!("gtk_menu_bridge: query failed for {app_id}: {e}");
            }
        }
    }
}

/// Queries org.gtk.Menus.Start([0]) on the session D-Bus for the given app_id.
fn query_gtk_menus(conn: &Connection, app_id: &str) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let dbus = zbus::blocking::fdo::DBusProxy::new(conn)?;
    if !dbus.name_has_owner(app_id.try_into()?)? {
        return Err(format!("{app_id} not on session bus").into());
    }

    // org.gnome.Nautilus -> /org/gnome/Nautilus
    let obj_path: zbus::zvariant::ObjectPath =
        format!("/{}", app_id.replace('.', "/")).try_into()?;
    let bus_name: zbus::names::BusName = app_id.try_into()?;
    let iface: zbus::names::InterfaceName = "org.gtk.Menus".try_into()?;

    // Call Start([0]) -> a(uuaa{sv})
    let reply = conn.call_method(
        Some(&bus_name),
        &obj_path,
        Some(&iface),
        "Start",
        &(vec![0u32],),
    )?;

    let body = reply.body();
    let groups: Vec<(u32, u32, Vec<HashMap<String, OwnedValue>>)> = body.deserialize()?;

    // Query actions for enabled/checked state.
    let actions = query_gtk_actions(conn, &bus_name, &obj_path).unwrap_or_default();

    // We may need more subscription groups. Collect all referenced groups
    // and fetch them too.
    let mut all_groups = groups;
    let referenced = collect_submenu_refs(&all_groups);
    let already_have: Vec<u32> = all_groups.iter().map(|(g, _, _)| *g).collect();
    let missing: Vec<u32> = referenced
        .into_iter()
        .filter(|g| !already_have.contains(g))
        .collect();

    if !missing.is_empty() {
        if let Ok(reply) = conn.call_method(
            Some(&bus_name),
            &obj_path,
            Some(&iface),
            "Start",
            &(missing,),
        ) {
            if let Ok(extra) = reply.body().deserialize::<Vec<(u32, u32, Vec<HashMap<String, OwnedValue>>)>>() {
                all_groups.extend(extra);
            }
        }
    }

    Ok(parse_gtk_menu_groups(&all_groups, &actions))
}

/// Queries org.gtk.Actions.DescribeAll for action metadata.
fn query_gtk_actions(
    conn: &Connection,
    bus_name: &zbus::names::BusName<'_>,
    obj_path: &zbus::zvariant::ObjectPath<'_>,
) -> Result<HashMap<String, ActionInfo>, Box<dyn std::error::Error>> {
    let iface: zbus::names::InterfaceName = "org.gtk.Actions".try_into()?;
    let reply = conn.call_method(Some(bus_name), obj_path, Some(&iface), "DescribeAll", &())?;
    let body = reply.body();
    let raw: HashMap<String, (bool, String, Vec<OwnedValue>)> = body.deserialize()?;

    let mut result = HashMap::new();
    for (name, (enabled, _param_type, state)) in raw {
        let checked = state.first().and_then(|v| {
            if let ZValue::Bool(b) = &**v { Some(*b) } else { None }
        });
        result.insert(name, ActionInfo { enabled, checked });
    }
    Ok(result)
}

struct ActionInfo {
    enabled: bool,
    checked: Option<bool>,
}

/// Activate a GTK action on the app via D-Bus.
///
/// `action` is the full action name from the menu (e.g. `app.new-window`
/// or `win.close`). The `app.` or `win.` prefix is stripped before calling
/// D-Bus because `org.gtk.Actions.Activate` expects the bare action name.
///
/// This is called from `dispatch_menu_action` when the app_id looks like
/// a reverse-domain GTK application.
pub fn activate_gtk_action(
    app_id: &str,
    action: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let conn = Connection::session()?;

    let dbus = zbus::blocking::fdo::DBusProxy::new(&conn)?;
    if !dbus.name_has_owner(app_id.try_into()?)? {
        return Err(format!("{app_id} not on session bus").into());
    }

    let obj_path: zbus::zvariant::ObjectPath =
        format!("/{}", app_id.replace('.', "/")).try_into()?;
    let bus_name: zbus::names::BusName = app_id.try_into()?;
    let iface: zbus::names::InterfaceName = "org.gtk.Actions".try_into()?;

    // Strip namespace prefix.
    let bare_action = action
        .strip_prefix("app.")
        .or_else(|| action.strip_prefix("win."))
        .unwrap_or(action);

    // org.gtk.Actions.Activate(action_name: s, parameter: av, platform_data: a{sv})
    let parameter: Vec<OwnedValue> = vec![];
    let platform_data: HashMap<String, OwnedValue> = HashMap::new();

    conn.call_method(
        Some(&bus_name),
        &obj_path,
        Some(&iface),
        "Activate",
        &(bare_action, &parameter, &platform_data),
    )?;

    log::debug!("gtk_menu_bridge: activated {bare_action} on {app_id}");
    Ok(())
}

/// Collects all subscription group IDs referenced by :submenu and :section.
fn collect_submenu_refs(groups: &[(u32, u32, Vec<HashMap<String, OwnedValue>>)]) -> Vec<u32> {
    let mut refs = Vec::new();
    for (_, _, items) in groups {
        for item in items {
            for key in [":submenu", ":section"] {
                if let Some((g, _)) = get_tuple_ref(item, key) {
                    refs.push(g);
                }
            }
        }
    }
    refs.sort();
    refs.dedup();
    refs
}

/// Converts the raw GTK menu groups into our JSON MenuGroup format.
fn parse_gtk_menu_groups(
    groups: &[(u32, u32, Vec<HashMap<String, OwnedValue>>)],
    actions: &HashMap<String, ActionInfo>,
) -> Vec<Value> {
    let mut menu_map: HashMap<(u32, u32), &Vec<HashMap<String, OwnedValue>>> = HashMap::new();
    for (group_id, menu_id, items) in groups {
        menu_map.insert((*group_id, *menu_id), items);
    }

    let Some(root_items) = menu_map.get(&(0, 0)) else {
        return vec![];
    };

    let mut menu_groups = Vec::new();
    for item in *root_items {
        let label = get_str(item, "label").unwrap_or_default();
        if let Some(submenu_ref) = get_tuple_ref(item, ":submenu") {
            let children = collect_section_items(submenu_ref, &menu_map, actions);
            if !children.is_empty() {
                menu_groups.push(json!({
                    "label": clean_label(&label),
                    "items": children,
                }));
            }
        }
    }

    menu_groups
}

/// Resolves a menu reference, flattening sections into a single item list
/// with separators between sections.
fn collect_section_items(
    menu_ref: (u32, u32),
    menu_map: &HashMap<(u32, u32), &Vec<HashMap<String, OwnedValue>>>,
    actions: &HashMap<String, ActionInfo>,
) -> Vec<Value> {
    let Some(items) = menu_map.get(&menu_ref) else {
        return vec![];
    };

    let mut result = Vec::new();
    for item in *items {
        // Section reference: inline items from another menu with a separator.
        if let Some(section_ref) = get_tuple_ref(item, ":section") {
            if !result.is_empty() {
                result.push(json!({ "type": "separator", "label": "", "action": "" }));
            }
            let section_items = collect_section_items(section_ref, menu_map, actions);
            result.extend(section_items);
            continue;
        }

        // Submenu reference: add as submenu type.
        if let Some(submenu_ref) = get_tuple_ref(item, ":submenu") {
            let label = get_str(item, "label").unwrap_or_default();
            let children = collect_section_items(submenu_ref, menu_map, actions);
            result.push(json!({
                "type": "submenu",
                "label": clean_label(&label),
                "action": "",
                "children": children,
            }));
            continue;
        }

        // Regular item.
        let Some(label) = get_str(item, "label") else { continue };
        let action = get_str(item, "action").unwrap_or_default();
        let action_key = action
            .strip_prefix("app.")
            .or_else(|| action.strip_prefix("win."))
            .unwrap_or(&action);

        let info = actions.get(action_key);
        let disabled = info.map(|a| !a.enabled).unwrap_or(false);
        let checked = info.and_then(|a| a.checked);
        let accel = get_str(item, "accel");

        let mut entry = json!({
            "type": "item",
            "label": clean_label(&label),
            "action": action,
            "disabled": disabled,
        });
        if let Some(s) = accel {
            entry["shortcut"] = json!(format_accel(&s));
        }
        if let Some(c) = checked {
            entry["checked"] = json!(c);
        }
        result.push(entry);
    }

    result
}

fn get_str(item: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    item.get(key).and_then(|v| {
        if let ZValue::Str(s) = &**v {
            Some(s.to_string())
        } else {
            None
        }
    })
}

/// Extracts a (u32, u32) tuple from a GVariant struct value.
fn get_tuple_ref(item: &HashMap<String, OwnedValue>, key: &str) -> Option<(u32, u32)> {
    let val = item.get(key)?;
    if let ZValue::Structure(s) = &**val {
        let fields = s.fields();
        if fields.len() == 2 {
            let a = if let ZValue::U32(v) = &fields[0] { Some(*v) } else { None };
            let b = if let ZValue::U32(v) = &fields[1] { Some(*v) } else { None };
            return a.zip(b);
        }
    }
    None
}

/// Removes mnemonic underscores from GTK labels.
fn clean_label(label: &str) -> String {
    label.replace('_', "")
}

/// Converts a GTK accelerator string to a human-readable shortcut.
fn format_accel(accel: &str) -> String {
    let mut parts = Vec::new();
    let mut rest = accel;

    while let Some(start) = rest.find('<') {
        if let Some(end) = rest[start..].find('>') {
            let modifier = &rest[start + 1..start + end];
            match modifier.to_lowercase().as_str() {
                "control" | "ctrl" | "primary" => parts.push("Ctrl".to_string()),
                "shift" => parts.push("Shift".to_string()),
                "alt" | "mod1" => parts.push("Alt".to_string()),
                "super" | "meta" => parts.push("Super".to_string()),
                _ => {}
            }
            rest = &rest[start + end + 1..];
        } else {
            break;
        }
    }

    if !rest.is_empty() {
        parts.push(rest.to_uppercase());
    }

    parts.join("+")
}
