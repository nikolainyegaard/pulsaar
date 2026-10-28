//! Last-known battery and full name per device serial, so offline devices
//! still show them. One line per serial in `devices.tsv` under the config
//! directory: serial, name, level, status byte, voltage, Unix time seen.
//! An empty field is unknown.

use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CachedDevice {
    pub name: String,
    pub level: Option<u8>,
    pub status: Option<u8>,
    pub voltage: Option<u16>,
    /// Unix seconds of the read; not compared when deciding to save.
    pub seen: u64,
}

pub struct DeviceCache {
    entries: HashMap<String, CachedDevice>,
    path: PathBuf,
}

impl DeviceCache {
    /// The cache file, or an empty cache when there is none.
    pub fn load() -> Self {
        let path = crate::config_dir().join("devices.tsv");
        let entries = std::fs::read_to_string(&path)
            .map(|t| parse(&t))
            .unwrap_or_default();
        Self { entries, path }
    }

    pub fn get(&self, serial: &str) -> Option<&CachedDevice> {
        self.entries.get(serial)
    }

    /// Record a live read and write the file when the name or battery changed.
    /// Write errors are ignored: the cache only fills gaps in the display.
    pub fn remember(&mut self, serial: &str, mut entry: CachedDevice) {
        if serial.is_empty() {
            return;
        }
        let old = self.entries.get(serial);
        // A read without a battery keeps the last battery; names always update.
        if entry.level.is_none() && entry.status.is_none() {
            if let Some(old) = old {
                entry.level = old.level;
                entry.status = old.status;
                entry.voltage = old.voltage;
                entry.seen = old.seen;
            }
        }
        let same = old.is_some_and(|o| {
            CachedDevice {
                seen: o.seen,
                ..entry.clone()
            } == *o
        });
        if same {
            return;
        }
        self.entries.insert(serial.to_owned(), entry);
        let _ = std::fs::create_dir_all(self.path.parent().unwrap_or(&self.path));
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, format(&self.entries)).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }
}

fn parse(text: &str) -> HashMap<String, CachedDevice> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split('\t');
            let serial = f.next().filter(|s| !s.is_empty())?.to_owned();
            let name = f.next().unwrap_or_default().to_owned();
            let mut num = || f.next().and_then(|v| v.parse::<u64>().ok());
            let level = num().and_then(|v| u8::try_from(v).ok());
            let status = num().and_then(|v| u8::try_from(v).ok());
            let voltage = num().and_then(|v| u16::try_from(v).ok());
            let seen = num().unwrap_or(0);
            Some((
                serial,
                CachedDevice {
                    name,
                    level,
                    status,
                    voltage,
                    seen,
                },
            ))
        })
        .collect()
}

fn format(entries: &HashMap<String, CachedDevice>) -> String {
    let opt = |v: Option<u64>| v.map(|v| v.to_string()).unwrap_or_default();
    let mut lines: Vec<String> = entries
        .iter()
        .map(|(serial, e)| {
            format!(
                "{serial}\t{}\t{}\t{}\t{}\t{}\n",
                e.name.replace(['\t', '\n'], " "),
                opt(e.level.map(u64::from)),
                opt(e.status.map(u64::from)),
                opt(e.voltage.map(u64::from)),
                e.seen
            )
        })
        .collect();
    lines.sort();
    lines.concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_and_parse_round_trip_with_unknown_fields() {
        let mut m = HashMap::new();
        m.insert(
            "699D23C8".to_owned(),
            CachedDevice {
                name: "MX Anywhere 3S".into(),
                level: Some(55),
                status: Some(0),
                voltage: None,
                seen: 1_800_000_000,
            },
        );
        m.insert(
            "B1465FB8".to_owned(),
            CachedDevice {
                name: "MX Keys Mini".into(),
                ..Default::default()
            },
        );
        assert_eq!(parse(&format(&m)), m);
    }
}
