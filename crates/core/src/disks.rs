//! Block devices on this machine, readable without root: the tree from
//! `lsblk`, UUIDs from /dev/disk/by-uuid, file system type and labels from
//! udev's database (/run/udev/data). Used by the "Mount a drive" wizard.

use crate::process;
use crate::types::{DiskInfo, PartitionInfo};
use crate::Result;
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Duration;

#[derive(serde::Deserialize)]
struct Lsblk {
    blockdevices: Vec<Dev>,
}

#[derive(serde::Deserialize)]
struct Dev {
    name: String,
    path: Option<String>,
    size: Option<u64>,
    #[serde(rename = "type")]
    kind: Option<String>,
    fstype: Option<String>,
    uuid: Option<String>,
    label: Option<String>,
    mountpoint: Option<String>,
    model: Option<String>,
    vendor: Option<String>,
    tran: Option<String>,
    rm: Option<bool>,
    #[serde(default)]
    children: Vec<Dev>,
}

/// /dev/disk/by-uuid: device path → uuid.
fn uuid_map() -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Ok(rd) = std::fs::read_dir("/dev/disk/by-uuid") {
        for e in rd.flatten() {
            if let Ok(target) = std::fs::canonicalize(e.path()) {
                out.insert(target.to_string_lossy().to_string(), e.file_name().to_string_lossy().to_string());
            }
        }
    }
    out
}

/// udev properties for a block device, keyed by property name.
fn udev_props(dev_path: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(meta) = std::fs::metadata(dev_path) else { return out };
    let rdev = meta.rdev();
    let major = ((rdev >> 8) & 0xfff) | ((rdev >> 32) & !0xfff);
    let minor = (rdev & 0xff) | ((rdev >> 12) & !0xff);
    if let Ok(text) = std::fs::read_to_string(format!("/run/udev/data/b{major}:{minor}")) {
        for line in text.lines() {
            if let Some(kv) = line.strip_prefix("E:") {
                if let Some((k, v)) = kv.split_once('=') {
                    out.insert(k.to_string(), v.to_string());
                }
            }
        }
    }
    out
}

fn partition(d: &Dev, uuids: &HashMap<String, String>) -> PartitionInfo {
    let path = d.path.clone().unwrap_or_else(|| format!("/dev/{}", d.name));
    let props = udev_props(&path);
    let canon = std::fs::canonicalize(&path).map(|p| p.to_string_lossy().to_string()).unwrap_or(path.clone());
    let mountpoint = d.mountpoint.clone();
    let system = matches!(mountpoint.as_deref(), Some("/") | Some("/boot") | Some("/boot/efi") | Some("/nix") | Some("/nix/store") | Some("[SWAP]"));
    PartitionInfo {
        name: d.name.clone(),
        path,
        size: d.size.unwrap_or(0),
        fstype: d.fstype.clone().or_else(|| props.get("ID_FS_TYPE").cloned()),
        uuid: d.uuid.clone().or_else(|| props.get("ID_FS_UUID").cloned()).or_else(|| uuids.get(&canon).cloned()),
        label: d.label.clone().or_else(|| props.get("ID_FS_LABEL").cloned()),
        partlabel: props.get("ID_PART_ENTRY_NAME").cloned(),
        mountpoint,
        system,
    }
}

pub async fn list() -> Result<Vec<DiskInfo>> {
    let out = process::run(
        "lsblk",
        &["-J", "-b", "-o", "NAME,PATH,SIZE,TYPE,FSTYPE,UUID,LABEL,MOUNTPOINT,MODEL,VENDOR,TRAN,RM"],
        None,
        Duration::from_secs(15),
    )
    .await?;
    let parsed: Lsblk = serde_json::from_str(&out.stdout).unwrap_or(Lsblk { blockdevices: vec![] });
    let uuids = uuid_map();
    let mut disks = Vec::new();
    for d in &parsed.blockdevices {
        if d.kind.as_deref() == Some("loop") || d.name.starts_with("zram") {
            continue;
        }
        let mut partitions: Vec<PartitionInfo> = Vec::new();
        fn walk(d: &Dev, uuids: &HashMap<String, String>, out: &mut Vec<PartitionInfo>) {
            for c in &d.children {
                if matches!(c.kind.as_deref(), Some("part") | Some("crypt") | Some("lvm") | Some("md") | Some("raid1") | Some("raid0")) {
                    out.push(partition(c, uuids));
                }
                walk(c, uuids, out);
            }
        }
        walk(d, &uuids, &mut partitions);
        // A disk used directly (no partition table) is itself mountable.
        if partitions.is_empty() {
            let p = partition(d, &uuids);
            if p.fstype.is_some() || p.uuid.is_some() {
                partitions.push(p);
            }
        }
        disks.push(DiskInfo {
            name: d.name.clone(),
            path: d.path.clone().unwrap_or_else(|| format!("/dev/{}", d.name)),
            size: d.size.unwrap_or(0),
            model: d.model.as_deref().map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).or_else(|| d.vendor.as_deref().map(|v| v.trim().to_string())),
            transport: d.tran.clone(),
            removable: d.rm.unwrap_or(false),
            partitions,
        });
    }
    Ok(disks)
}

#[allow(dead_code)]
fn _p(_: &Path) {}
