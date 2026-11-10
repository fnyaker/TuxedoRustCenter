// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Statistiques compactes pour barres d'état et widgets : charge CPU,
//! puissance de la batterie. Lecture sysfs/procfs non privilégiée.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

/// Compteurs cumulés de `/proc/stat` (ligne `cpu`) : `(inactif, total)`.
pub fn parse_proc_stat(content: &str) -> Option<(u64, u64)> {
    let line = content.lines().find(|line| line.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|field| field.parse().ok())
        .collect();
    if fields.len() < 4 {
        return None;
    }
    // idle + iowait sont inactifs ; guest est déjà compté dans user/nice.
    let idle = fields[3] + fields.get(4).copied().unwrap_or(0);
    let total: u64 = fields.iter().take(8).sum();
    Some((idle, total))
}

/// Pourcentage d'utilisation entre deux échantillons.
pub fn usage_percent(before: (u64, u64), after: (u64, u64)) -> Option<f64> {
    let total = after.1.checked_sub(before.1)?;
    let idle = after.0.checked_sub(before.0)?;
    if total == 0 {
        return None;
    }
    let busy = total.saturating_sub(idle) as f64;
    Some((busy / total as f64 * 100.0 * 10.0).round() / 10.0)
}

/// Mesure la charge CPU globale sur `window`.
pub fn cpu_usage(window: Duration) -> Option<f64> {
    let before = parse_proc_stat(&std::fs::read_to_string("/proc/stat").ok()?)?;
    std::thread::sleep(window);
    let after = parse_proc_stat(&std::fs::read_to_string("/proc/stat").ok()?)?;
    usage_percent(before, after)
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|content| content.trim().to_owned())
}

fn read_u64(path: &Path) -> Option<u64> {
    read_trimmed(path)?.parse().ok()
}

/// État de la première batterie de `power_supply_root`.
///
/// `powerW` est toujours positif ; `status` indique le sens (Discharging,
/// Charging...). Absent si ni `power_now` ni `current_now × voltage_now` ne
/// sont lisibles.
pub fn battery(power_supply_root: &Path) -> Value {
    let Ok(entries) = std::fs::read_dir(power_supply_root) else {
        return json!({"present": false});
    };
    let mut dirs: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| read_trimmed(&path.join("type")).as_deref() == Some("Battery"))
        .collect();
    dirs.sort();
    let Some(dir) = dirs.first() else {
        return json!({"present": false});
    };
    let power_uw = read_u64(&dir.join("power_now")).or_else(|| {
        let current = read_u64(&dir.join("current_now"))?;
        let voltage = read_u64(&dir.join("voltage_now"))?;
        // µA × µV = 1e-12 W ; on veut des µW.
        Some(current.saturating_mul(voltage) / 1_000_000)
    });
    json!({
        "present": true,
        "name": dir.file_name().and_then(|name| name.to_str()),
        "percent": read_u64(&dir.join("capacity")),
        "status": read_trimmed(&dir.join("status")),
        "powerW": power_uw.map(|uw| (uw as f64 / 1_000_000.0 * 100.0).round() / 100.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_stat() {
        let stat = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 1 2 3 4\n";
        assert_eq!(parse_proc_stat(stat), Some((850, 1000)));
        assert_eq!(parse_proc_stat("garbage"), None);
    }

    #[test]
    fn computes_usage() {
        assert_eq!(usage_percent((850, 1000), (900, 1100)), Some(50.0));
        assert_eq!(usage_percent((1, 1), (1, 1)), None);
        assert_eq!(usage_percent((10, 10), (5, 20)), None);
    }

    fn fake_battery(power_now: Option<&str>) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let bat = root.path().join("BAT0");
        std::fs::create_dir(&bat).unwrap();
        std::fs::write(bat.join("type"), "Battery\n").unwrap();
        std::fs::write(bat.join("capacity"), "81\n").unwrap();
        std::fs::write(bat.join("status"), "Discharging\n").unwrap();
        match power_now {
            Some(value) => std::fs::write(bat.join("power_now"), value).unwrap(),
            None => {
                std::fs::write(bat.join("current_now"), "1500000\n").unwrap();
                std::fs::write(bat.join("voltage_now"), "11000000\n").unwrap();
            }
        }
        let ac = root.path().join("AC");
        std::fs::create_dir(&ac).unwrap();
        std::fs::write(ac.join("type"), "Mains\n").unwrap();
        root
    }

    #[test]
    fn reads_battery_power_now() {
        let root = fake_battery(Some("12340000\n"));
        let value = battery(root.path());
        assert_eq!(value["present"], true);
        assert_eq!(value["percent"], 81);
        assert_eq!(value["status"], "Discharging");
        assert_eq!(value["powerW"], 12.34);
    }

    #[test]
    fn derives_power_from_current_and_voltage() {
        let root = fake_battery(None);
        assert_eq!(battery(root.path())["powerW"], 16.5);
    }

    #[test]
    fn no_battery_is_not_an_error() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(battery(root.path()), json!({"present": false}));
        assert_eq!(
            battery(Path::new("/nonexistent")),
            json!({"present": false})
        );
    }
}
