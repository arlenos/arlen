//! Device enumeration for the FM Devices sidebar: the mounted volumes a user can
//! navigate to (and, when removable, eject). Source is `lsblk --json` (util-linux,
//! ubiquitous); this module is the pure parse + filter, unit-tested against
//! sample output, so the host command is a thin "run lsblk, hand the JSON here".
//!
//! Shown: a block device (disk or partition) that is MOUNTED, has a real
//! filesystem, and is not a system mount (`/`, `/boot*`, swap) - i.e. removable
//! drives and extra data volumes, not the root/boot the sidebar's Places already
//! cover. Each carries whether it is removable, so the UI offers eject only there.

use serde::{Deserialize, Serialize};

/// `lsblk --json` top level.
#[derive(Deserialize)]
struct LsblkOutput {
    #[serde(default)]
    blockdevices: Vec<LsblkDevice>,
}

/// One `lsblk` block device. Partitions appear as `children` of their disk; the
/// `rm` (removable) flag is reported on each (a partition inherits its disk's).
#[derive(Deserialize)]
struct LsblkDevice {
    name: String,
    label: Option<String>,
    mountpoint: Option<String>,
    #[serde(default)]
    rm: bool,
    fstype: Option<String>,
    #[serde(default)]
    children: Vec<LsblkDevice>,
}

/// A volume for the Devices sidebar: a mounted one to navigate to, or an
/// unmounted removable drive to mount on click.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MountedDevice {
    /// Display label: the filesystem label, else the device name.
    pub label: String,
    /// Where it is mounted (the path the sidebar navigates to); empty when
    /// the device is unmounted (`mounted == false`).
    pub mountpoint: String,
    /// The block device node (`/dev/sdb1`), for mount/unmount/eject.
    pub device: String,
    /// Whether the drive is removable (the UI offers eject only here).
    pub removable: bool,
    /// Whether the volume is currently mounted. An unmounted entry is a
    /// removable drive the sidebar offers to mount (then navigate into).
    pub mounted: bool,
    /// The filesystem type (for the icon / detail).
    pub fstype: String,
}

/// Whether a mountpoint is a system mount the Devices section should not list
/// (Places already covers Home; root/boot/swap are not user "devices").
fn is_system_mount(mountpoint: &str) -> bool {
    mountpoint == "/" || mountpoint.starts_with("/boot") || mountpoint == "[SWAP]"
}

fn collect(dev: &LsblkDevice, out: &mut Vec<MountedDevice>) {
    let real_fs = dev
        .fstype
        .as_deref()
        .is_some_and(|f| !f.is_empty() && f != "swap");
    let label = || {
        dev.label
            .clone()
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| dev.name.clone())
    };
    match dev.mountpoint.as_deref() {
        // A mounted volume the sidebar navigates into (root/boot/swap excluded).
        Some(mp) if real_fs && !is_system_mount(mp) => {
            out.push(MountedDevice {
                label: label(),
                mountpoint: mp.to_string(),
                device: format!("/dev/{}", dev.name),
                removable: dev.rm,
                mounted: true,
                fstype: dev.fstype.clone().unwrap_or_default(),
            });
        }
        // An unmounted REMOVABLE drive with a real filesystem: offer to mount it
        // (the common "plugged in a stick that did not auto-mount" case). Unmounted
        // INTERNAL volumes are not listed - they are an admin concern, not a device
        // a user casually mounts from the file manager.
        None if real_fs && dev.rm => {
            out.push(MountedDevice {
                label: label(),
                mountpoint: String::new(),
                device: format!("/dev/{}", dev.name),
                removable: dev.rm,
                mounted: false,
                fstype: dev.fstype.clone().unwrap_or_default(),
            });
        }
        _ => {}
    }
    for child in &dev.children {
        collect(child, out);
    }
}

/// Parse `lsblk --json` output into the mounted, non-system volumes for the
/// Devices sidebar. Malformed JSON yields an empty list (the sidebar simply
/// shows no devices, never an error).
pub fn mounted_volumes(lsblk_json: &str) -> Vec<MountedDevice> {
    let Ok(parsed) = serde_json::from_str::<LsblkOutput>(lsblk_json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dev in &parsed.blockdevices {
        collect(dev, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // A laptop with the root NVMe + boot, plus a mounted removable USB stick.
    const SAMPLE: &str = r#"{
      "blockdevices": [
        { "name": "sda", "label": "USB DISK", "mountpoint": "/run/media/me/USB DISK",
          "rm": true, "fstype": "exfat" },
        { "name": "nvme0n1", "label": null, "mountpoint": null, "rm": false, "fstype": null,
          "children": [
            { "name": "nvme0n1p1", "label": null, "mountpoint": "/boot/efi", "rm": false, "fstype": "vfat" },
            { "name": "nvme0n1p2", "label": null, "mountpoint": "/", "rm": false, "fstype": "ext4" },
            { "name": "nvme0n1p3", "label": "data", "mountpoint": "/mnt/data", "rm": false, "fstype": "ext4" }
          ]
        },
        { "name": "zram0", "label": null, "mountpoint": "[SWAP]", "rm": false, "fstype": "swap" }
      ]
    }"#;

    #[test]
    fn lists_removable_and_extra_volumes_but_not_system_mounts() {
        let v = mounted_volumes(SAMPLE);
        let mps: Vec<&str> = v.iter().map(|d| d.mountpoint.as_str()).collect();
        // The USB and the extra data partition show; root, /boot/efi, swap do not.
        assert!(mps.contains(&"/run/media/me/USB DISK"));
        assert!(mps.contains(&"/mnt/data"));
        assert!(!mps.contains(&"/"));
        assert!(!mps.contains(&"/boot/efi"));
        assert!(!mps.iter().any(|m| *m == "[SWAP]"));
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn flags_removable_and_builds_device_and_label() {
        let v = mounted_volumes(SAMPLE);
        let usb = v.iter().find(|d| d.removable).unwrap();
        assert_eq!(usb.label, "USB DISK");
        assert_eq!(usb.device, "/dev/sda");
        assert_eq!(usb.fstype, "exfat");
        // The non-removable data partition falls back to its label, not removable.
        let data = v.iter().find(|d| d.mountpoint == "/mnt/data").unwrap();
        assert!(!data.removable);
        assert_eq!(data.label, "data");
        assert_eq!(data.device, "/dev/nvme0n1p3");
    }

    #[test]
    fn mounted_volumes_carry_the_mounted_flag() {
        for d in mounted_volumes(SAMPLE) {
            assert!(d.mounted, "{} should be flagged mounted", d.mountpoint);
            assert!(!d.mountpoint.is_empty());
        }
    }

    // A removable stick whose partition is present but NOT mounted, plus an
    // unmounted INTERNAL partition that must stay hidden.
    const UNMOUNTED: &str = r#"{
      "blockdevices": [
        { "name": "sdb", "label": null, "mountpoint": null, "rm": true, "fstype": null,
          "children": [
            { "name": "sdb1", "label": "BACKUP", "mountpoint": null, "rm": true, "fstype": "ext4" }
          ]
        },
        { "name": "sdc", "label": "SPARE", "mountpoint": null, "rm": false, "fstype": "ext4" }
      ]
    }"#;

    #[test]
    fn unmounted_removable_is_offered_for_mounting_internal_is_not() {
        let v = mounted_volumes(UNMOUNTED);
        // The removable partition is listed, unmounted, with no mountpoint and
        // its /dev node for the mount call.
        let backup = v.iter().find(|d| d.label == "BACKUP").unwrap();
        assert!(!backup.mounted);
        assert!(backup.removable);
        assert!(backup.mountpoint.is_empty());
        assert_eq!(backup.device, "/dev/sdb1");
        // The unmounted internal volume is hidden (admin concern, not a casual mount).
        assert!(!v.iter().any(|d| d.label == "SPARE"));
        // The bare removable disk (no filesystem) is not itself listed.
        assert!(!v.iter().any(|d| d.device == "/dev/sdb"));
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn malformed_json_is_empty_not_an_error() {
        assert!(mounted_volumes("not json").is_empty());
        assert!(mounted_volumes("{}").is_empty());
    }
}
