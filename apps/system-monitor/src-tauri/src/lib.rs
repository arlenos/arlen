//! The Arlen system monitor Tauri shell (the task manager). The landing is the
//! live process list - what is running, the hog on top, real CPU/memory/disk per
//! row - replacing the frontend fixture with a real `/proc` feed.
//!
//! `system_tick` does the same for the whole machine (the Performance tab's CPU,
//! memory, disk and network), replacing a frontend random walk that looked like
//! measurement and was not.
//!
//! `list_processes` samples `/proc` and computes CPU% + disk-rate deltas against
//! the previous sample (held in the [`procmon::Monitor`] managed state), so the
//! rates settle after the first poll while memory + names are real immediately.
//! The Stop/Freeze/Limit actions are the next increment; per-process network needs
//! eBPF/cgroup attribution and is reported as 0 until then.

use tauri::Manager;

use arlen_system_monitor_core::actions;
use arlen_system_monitor_core::procdetail::{held_resources, proc_stats, HeldResources, ProcStats};
use arlen_system_monitor_core::procmon::{group_processes, Monitor, Process};
use arlen_system_monitor_core::sysmon::{SystemMonitor, SystemTick};

/// A structured log line from the frontend into the app's stdout (the shell has no
/// devtools console the operator can open).
#[tauri::command]
fn frontend_log(level: String, message: String) {
    match level.as_str() {
        "error" => log::error!("[frontend] {message}"),
        "warn" => log::warn!("[frontend] {message}"),
        _ => log::info!("[frontend] {message}"),
    }
}

/// The live process list: real `/proc` processes mapped to the frontend `Process`
/// shape, the hog on top. CPU% and disk KB/s are 0 on the first call (no previous
/// sample to delta against) and real from the second poll on.
#[tauri::command]
fn list_processes(monitor: tauri::State<'_, Monitor>) -> Vec<Process> {
    monitor.sample()
}

/// The same sample, folded into the app-grouped rows the landing view opens on.
///
/// A second command rather than a flag on the first, because the flat list is
/// the power-user toggle and both are wanted: the frontend holds one sample and
/// asks for whichever shape it is showing. Same `Process` type either way - the
/// grouped rows carry `children`, which the frontend model already has - so the
/// table learns no second shape. The fold is pure and unit-tested in `procmon`,
/// so this is the wiring and nothing else.
#[tauri::command]
fn list_app_rows(monitor: tauri::State<'_, Monitor>) -> Vec<Process> {
    group_processes(&monitor.sample())
}

/// What one process is holding open: its files, its sockets, and whether a camera
/// or microphone is among them.
///
/// Read on demand rather than per poll: walking an fd table for every process
/// every second is work done on the machine the user opened this to relieve, and
/// only the selected row's detail is on screen. Every field is optional, and a
/// process belonging to another user comes back unmeasured with the reason
/// attached - never as an empty list, which would read as "holds nothing open".
#[tauri::command]
fn process_held_resources(pid: u32) -> HeldResources {
    held_resources(std::path::Path::new("/proc"), pid)
}

/// The per-process Statistics and Memory figures, read from `/proc/<pid>`.
///
/// Paired with `process_held_resources` on the same selection rather than folded
/// into it: the fd walk is the expensive half and the statistics are three small
/// file reads, so a pane that only shows numbers does not pay for a file table
/// it is not displaying.
#[tauri::command]
fn process_stats(pid: u32) -> ProcStats {
    proc_stats(std::path::Path::new("/proc"), pid)
}

/// One tick of system-wide device counters for the Performance tab: CPU, memory,
/// disk and network, read from `/proc` and `/sys`. The rate fields are zero on the
/// first call with `ratesReady` false, and real from the second on.
#[tauri::command]
fn system_tick(system: tauri::State<'_, SystemMonitor>) -> SystemTick {
    system.sample()
}

/// Gracefully stop a process (SIGTERM). The kernel refuses a process the user does
/// not own, so the error is surfaced to the row.
#[tauri::command]
fn stop_process(id: u32) -> Result<(), String> {
    actions::stop(id)
}

/// Freeze (`paused=true`) or thaw (`paused=false`) a process - the non-destructive
/// pause (SIGSTOP/SIGCONT).
#[tauri::command]
fn freeze_process(id: u32, paused: bool) -> Result<(), String> {
    actions::freeze(id, paused)
}

/// Soft-leash (`limited=true`) or release a process's CPU via its cgroup `cpu.max`.
/// Best-effort: without cgroup delegation the write fails and the error is
/// surfaced, so the UI never falsely shows a limit.
#[tauri::command]
fn limit_process(id: u32, limited: bool) -> Result<(), String> {
    actions::limit(id, limited)
}

/// Set a process's scheduling priority - the Advanced affordance
/// (system-monitor-plan.md (c)).
///
/// Refusals reach the UI verbatim, which matters here more than elsewhere:
/// raising a nice value needs no privilege but LOWERING one needs `CAP_SYS_NICE`,
/// so "make this faster" is refused for an ordinary user while "make this
/// slower" works. A control that quietly did nothing in one direction would be
/// worse than no control.
#[tauri::command]
fn renice_process(id: u32, nice: i32) -> Result<(), String> {
    actions::set_nice(id, nice)
}

/// The priority a process is at now, so the menu can tick the real one. `None`
/// when it cannot be read, never a confident Normal.
#[tauri::command]
fn process_nice(id: u32) -> Option<i32> {
    actions::get_nice(id)
}

/// The priority levels the Advanced menu offers, as (label, nice) pairs.
///
/// Served rather than duplicated in the frontend: `set_nice` validates against
/// this same table, so a hand-copied list in TypeScript would drift into
/// offering a level the backend refuses. One source, like the `critical` flag on
/// a row.
#[tauri::command]
fn nice_levels() -> Vec<(String, i32)> {
    actions::NICE_LEVELS.iter().map(|(l, n)| ((*l).to_string(), *n)).collect()
}

/// Build + run the app.
pub fn run() {
    // Dependencies at warn, this app at info. A blanket `info` also turns on
    // zbus, which logs D-Bus handshake frames WITH their message bytes - and a
    // message body is user content: file paths, query strings, notification
    // text. At info that lands in the journal, readable by anything with
    // journal access and covered by no capability grant, which undoes in a log
    // line what the graph's scoping is for. A byte trace stays available as
    // `RUST_LOG=zbus=trace`, deliberately, rather than by default.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,arlen_system_monitor_lib=info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .setup(|app| {
            app.manage(Monitor::new());
            app.manage(SystemMonitor::new());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            frontend_log,
            list_processes,
            process_held_resources,
            process_stats,
            list_app_rows,
            system_tick,
            stop_process,
            freeze_process,
            limit_process,
            renice_process,
            process_nice,
            nice_levels
        ])
        .run(tauri::generate_context!())
        .expect("error while running arlen-system-monitor");
}
