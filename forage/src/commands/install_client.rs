/// D-Bus client for org.lunaris.InstallDaemon1.
///
/// Connects to the session bus, calls methods, and subscribes to
/// progress signals for the terminal UI.

use colored::Colorize;
use futures_util::StreamExt;
use zbus::Connection;

const BUS_NAME: &str = "org.lunaris.InstallDaemon1";
const OBJECT_PATH: &str = "/org/lunaris/InstallDaemon1";
const INTERFACE: &str = "org.lunaris.InstallDaemon1";

/// Connect to the install daemon on the session bus.
pub async fn connect() -> Result<Connection, String> {
    Connection::session()
        .await
        .map_err(|e| format!("failed to connect to session bus: {e}"))
}

/// Call a D-Bus method that returns a job_id, then wait for completion.
async fn call_and_wait(
    conn: &Connection,
    method: &str,
    args: &(impl serde::Serialize + zbus::zvariant::Type),
) -> Result<(), String> {
    let iface = zbus::names::InterfaceName::try_from(INTERFACE)
        .map_err(|e| format!("invalid interface: {e}"))?;
    let bus = zbus::names::BusName::try_from(BUS_NAME)
        .map_err(|e| format!("invalid bus name: {e}"))?;
    let path = zbus::zvariant::ObjectPath::try_from(OBJECT_PATH)
        .map_err(|e| format!("invalid path: {e}"))?;

    // Call the method to get a job_id.
    let reply = conn
        .call_method(Some(&bus), &path, Some(&iface), method, args)
        .await
        .map_err(|e| format!("{method} call failed: {e}"))?;

    let job_id: String = reply
        .body()
        .deserialize()
        .map_err(|e| format!("failed to parse job_id: {e}"))?;

    println!("{} {}", "Job".dimmed(), job_id.dimmed());

    // Subscribe to signals for progress.
    wait_for_job(conn, &job_id).await
}

/// Subscribe to JobProgress and JobCompleted signals until the job finishes.
async fn wait_for_job(conn: &Connection, job_id: &str) -> Result<(), String> {
    let proxy = zbus::Proxy::new(
        conn,
        BUS_NAME,
        OBJECT_PATH,
        INTERFACE,
    )
    .await
    .map_err(|e| format!("proxy creation failed: {e}"))?;

    let mut stream = proxy
        .receive_all_signals()
        .await
        .map_err(|e| format!("signal subscription failed: {e}"))?;

    // Timeout after 10 minutes.
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(600);

    loop {
        let result = tokio::time::timeout_at(deadline, stream.next()).await;

        match result {
            Ok(Some(signal)) => {
                let member = signal.header().member().map(|m| m.to_string()).unwrap_or_default();
                let body = signal.body();

                match member.as_str() {
                    "JobProgress" => {
                        if let Ok((sid, percent, status)) =
                            body.deserialize::<(String, u32, String)>()
                        {
                            if sid == job_id {
                                print!("\r{} {}% {}",
                                    "progress:".dimmed(),
                                    percent.to_string().cyan(),
                                    status,
                                );
                                // Flush stdout for carriage return.
                                use std::io::Write;
                                let _ = std::io::stdout().flush();
                            }
                        }
                    }
                    "JobCompleted" => {
                        if let Ok((sid, success, error)) =
                            body.deserialize::<(String, bool, String)>()
                        {
                            if sid == job_id {
                                println!(); // newline after progress
                                if success {
                                    println!("{}", "done.".green().bold());
                                    return Ok(());
                                } else {
                                    return Err(error);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(None) => {
                return Err("signal stream ended unexpectedly".into());
            }
            Err(_) => {
                return Err("timed out waiting for job completion".into());
            }
        }
    }
}

/// Install a .lunpkg package.
pub async fn install_package(conn: &Connection, path: &str) -> Result<(), String> {
    println!("{} {}", "installing".cyan().bold(), path);
    call_and_wait(conn, "InstallPackage", &(path.to_string(),)).await
}

/// Install a Flatpak app.
pub async fn install_flatpak(conn: &Connection, app_id: &str) -> Result<(), String> {
    println!("{} {} (flatpak)", "installing".cyan().bold(), app_id);
    call_and_wait(conn, "InstallFlatpak", &(app_id.to_string(), String::new())).await
}

/// Uninstall an app.
pub async fn uninstall(conn: &Connection, app_id: &str) -> Result<(), String> {
    println!("{} {}", "removing".cyan().bold(), app_id);
    call_and_wait(conn, "Uninstall", &(app_id.to_string(),)).await
}

/// Uninstall a Flatpak app.
pub async fn uninstall_flatpak(conn: &Connection, app_id: &str) -> Result<(), String> {
    println!("{} {} (flatpak)", "removing".cyan().bold(), app_id);
    call_and_wait(conn, "UninstallFlatpak", &(app_id.to_string(),)).await
}

/// List all installed apps.
pub async fn list_installed(conn: &Connection) -> Result<(), String> {
    let iface = zbus::names::InterfaceName::try_from(INTERFACE).unwrap();
    let bus = zbus::names::BusName::try_from(BUS_NAME).unwrap();
    let path = zbus::zvariant::ObjectPath::try_from(OBJECT_PATH).unwrap();

    let reply = conn
        .call_method(Some(&bus), &path, Some(&iface), "ListInstalled", &())
        .await
        .map_err(|e| format!("ListInstalled failed: {e}"))?;

    let apps: Vec<(String, String, String, String)> = reply
        .body()
        .deserialize()
        .map_err(|e| format!("failed to parse response: {e}"))?;

    if apps.is_empty() {
        println!("{}", "no apps installed".dimmed());
        return Ok(());
    }

    // Header.
    println!(
        "{:<40} {:<20} {:<10} {}",
        "ID".bold(),
        "Name".bold(),
        "Version".bold(),
        "Source".bold()
    );
    println!("{}", "-".repeat(80).dimmed());

    for (id, name, version, source) in &apps {
        let source_colored = match source.as_str() {
            "lunpkg" => source.green(),
            "flatpak" => source.blue(),
            _ => source.dimmed(),
        };
        println!("{:<40} {:<20} {:<10} {}", id, name, version, source_colored);
    }

    println!("{}", format!("\n{} app(s)", apps.len()).dimmed());
    Ok(())
}

/// List apps in the 30-day trash.
pub async fn list_trashed(conn: &Connection) -> Result<(), String> {
    let iface = zbus::names::InterfaceName::try_from(INTERFACE).unwrap();
    let bus = zbus::names::BusName::try_from(BUS_NAME).unwrap();
    let path = zbus::zvariant::ObjectPath::try_from(OBJECT_PATH).unwrap();

    let reply = conn
        .call_method(Some(&bus), &path, Some(&iface), "ListTrashed", &())
        .await
        .map_err(|e| format!("ListTrashed failed: {e}"))?;

    let entries: Vec<(String, String, String, String)> = reply
        .body()
        .deserialize()
        .map_err(|e| format!("failed to parse response: {e}"))?;

    if entries.is_empty() {
        println!("{}", "trash is empty".dimmed());
        return Ok(());
    }

    println!(
        "{:<40} {:<20} {:<10} {}",
        "ID".bold(),
        "Name".bold(),
        "Version".bold(),
        "Deleted".bold()
    );
    println!("{}", "-".repeat(80).dimmed());

    for (id, name, version, deleted_at) in &entries {
        println!("{:<40} {:<20} {:<10} {}", id, name, version, deleted_at.dimmed());
    }

    println!("{}", format!("\n{} app(s) in trash", entries.len()).dimmed());
    Ok(())
}

/// Restore an app from trash.
pub async fn restore_app(conn: &Connection, app_id: &str) -> Result<(), String> {
    let iface = zbus::names::InterfaceName::try_from(INTERFACE).unwrap();
    let bus = zbus::names::BusName::try_from(BUS_NAME).unwrap();
    let path = zbus::zvariant::ObjectPath::try_from(OBJECT_PATH).unwrap();

    let reply = conn
        .call_method(
            Some(&bus),
            &path,
            Some(&iface),
            "RestoreApp",
            &(app_id.to_string(),),
        )
        .await
        .map_err(|e| format!("RestoreApp failed: {e}"))?;

    let (success, error): (bool, String) = reply
        .body()
        .deserialize()
        .map_err(|e| format!("failed to parse response: {e}"))?;

    if success {
        println!("{} {}", "restored".green().bold(), app_id);
        Ok(())
    } else {
        Err(error)
    }
}

/// Trigger trash cleanup.
pub async fn cleanup_trash(conn: &Connection) -> Result<(), String> {
    let iface = zbus::names::InterfaceName::try_from(INTERFACE).unwrap();
    let bus = zbus::names::BusName::try_from(BUS_NAME).unwrap();
    let path = zbus::zvariant::ObjectPath::try_from(OBJECT_PATH).unwrap();

    let reply = conn
        .call_method(Some(&bus), &path, Some(&iface), "CleanupTrash", &())
        .await
        .map_err(|e| format!("CleanupTrash failed: {e}"))?;

    let count: u32 = reply
        .body()
        .deserialize()
        .map_err(|e| format!("failed to parse response: {e}"))?;

    if count > 0 {
        println!("{} {count} expired entries", "cleaned".green().bold());
    } else {
        println!("{}", "nothing to clean".dimmed());
    }
    Ok(())
}

/// Show the install location of an app.
pub async fn which_app(app_id: &str) -> Result<(), String> {
    // Check user apps.
    let user_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.local/share"))
        .join(format!("lunaris/apps/{app_id}"));
    if user_dir.exists() {
        println!("{}", user_dir.display());
        return Ok(());
    }

    // Check system apps.
    let sys_dir = std::path::PathBuf::from(format!("/usr/lib/lunaris/apps/{app_id}"));
    if sys_dir.exists() {
        println!("{}", sys_dir.display());
        return Ok(());
    }

    // Check flatpak.
    let output = std::process::Command::new("flatpak")
        .args(["info", "--show-location", "--user", app_id])
        .output();
    if let Ok(o) = output {
        if o.status.success() {
            let path = String::from_utf8_lossy(&o.stdout).trim().to_string();
            println!("{path}");
            return Ok(());
        }
    }

    Err(format!("{app_id} not found"))
}

/// Show info about an installed app (reads manifest from install dir).
pub async fn info_app(app_id: &str) -> Result<(), String> {
    let user_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.local/share"))
        .join(format!("lunaris/apps/{app_id}"));

    let manifest_path = user_dir.join("manifest.toml");
    if manifest_path.exists() {
        let content = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("failed to read manifest: {e}"))?;

        #[derive(serde::Deserialize)]
        struct Manifest {
            package: Package,
        }
        #[derive(serde::Deserialize)]
        struct Package {
            id: String,
            name: String,
            version: String,
            #[serde(default)]
            description: String,
            #[serde(default)]
            author: String,
        }

        let m: Manifest = toml::from_str(&content)
            .map_err(|e| format!("invalid manifest: {e}"))?;

        println!("{:<12} {}", "ID:".bold(), m.package.id);
        println!("{:<12} {}", "Name:".bold(), m.package.name);
        println!("{:<12} {}", "Version:".bold(), m.package.version);
        if !m.package.description.is_empty() {
            println!("{:<12} {}", "About:".bold(), m.package.description);
        }
        if !m.package.author.is_empty() {
            println!("{:<12} {}", "Author:".bold(), m.package.author);
        }
        println!("{:<12} {}", "Source:".bold(), "lunpkg".green());
        println!("{:<12} {}", "Path:".bold(), user_dir.display());
        return Ok(());
    }

    // Try flatpak.
    let output = std::process::Command::new("flatpak")
        .args(["info", "--user", app_id])
        .output();
    if let Ok(o) = output {
        if o.status.success() {
            println!("{:<12} {}", "Source:".bold(), "flatpak".blue());
            print!("{}", String::from_utf8_lossy(&o.stdout));
            return Ok(());
        }
    }

    Err(format!("{app_id} not found"))
}
