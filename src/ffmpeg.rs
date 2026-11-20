use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};

use crate::spec::Encoders;

const CACHE_TTL_SECS: u64 = 7 * 86400;
const MINIMUM: (u32, u32) = (6, 0);

#[derive(Debug, Clone)]
pub struct Ffmpeg {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub major: u32,
    pub minor: u32,
    pub libx264: bool,
    pub libx265: bool,
    pub aac: bool,
    pub filters_ok: bool,
}

impl Capabilities {
    pub fn encoders(&self) -> Encoders {
        Encoders {
            libx264: self.libx264,
            libx265: self.libx265,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryId {
    pub path: String,
    pub mtime: u64,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub id: BinaryId,
    pub checked_at: u64,
    pub caps: Capabilities,
}

pub fn parse_version(output: &str) -> Option<(u32, u32)> {
    let rest = output.split("version ").nth(1)?;
    let token = rest.split_whitespace().next()?.trim_start_matches('n');
    let mut parts = token.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor))
}

pub fn cache_is_valid(entry: &CacheEntry, id: &BinaryId, now: SystemTime) -> bool {
    if &entry.id != id {
        return false;
    }
    let now = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    now.saturating_sub(entry.checked_at) <= CACHE_TTL_SECS
}

pub fn check_minimum(caps: &Capabilities) -> anyhow::Result<()> {
    if (caps.major, caps.minor) < MINIMUM {
        bail!(
            "ffmpeg {}.{} is too old; fictions needs {}.{}+ (run `brew upgrade ffmpeg`)",
            caps.major,
            caps.minor,
            MINIMUM.0,
            MINIMUM.1
        );
    }
    Ok(())
}

pub fn discover() -> anyhow::Result<Ffmpeg> {
    let ffmpeg = find_in_path("ffmpeg")
        .ok_or_else(|| anyhow!("ffmpeg not found in PATH; run `brew install ffmpeg`"))?;
    let ffprobe = find_in_path("ffprobe")
        .ok_or_else(|| anyhow!("ffprobe not found in PATH; run `brew install ffmpeg`"))?;
    Ok(Ffmpeg { ffmpeg, ffprobe })
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

impl Ffmpeg {
    pub fn capabilities(&self) -> anyhow::Result<Capabilities> {
        let id = self.binary_id()?;
        let cache_file = cache_path();

        if let Some(path) = &cache_file {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(entry) = serde_json::from_str::<CacheEntry>(&text) {
                    if cache_is_valid(&entry, &id, SystemTime::now()) {
                        return Ok(entry.caps);
                    }
                }
            }
        }

        let caps = self.probe_capabilities()?;
        check_minimum(&caps)?;

        if let Some(path) = &cache_file {
            let entry = CacheEntry {
                id,
                checked_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                caps: caps.clone(),
            };
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(text) = serde_json::to_string(&entry) {
                let _ = std::fs::write(path, text);
            }
        }

        Ok(caps)
    }

    fn binary_id(&self) -> anyhow::Result<BinaryId> {
        let meta = std::fs::metadata(&self.ffmpeg)
            .with_context(|| format!("cannot stat {}", self.ffmpeg.display()))?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Ok(BinaryId {
            path: self.ffmpeg.to_string_lossy().into_owned(),
            mtime,
            size: meta.len(),
        })
    }

    fn probe_capabilities(&self) -> anyhow::Result<Capabilities> {
        let version = run_capture(&self.ffmpeg, &["-hide_banner", "-version"])?;
        let (major, minor) = parse_version(&version)
            .ok_or_else(|| anyhow!("cannot parse ffmpeg version output"))?;
        let encoders = run_capture(&self.ffmpeg, &["-hide_banner", "-encoders"])?;
        let filters = run_capture(&self.ffmpeg, &["-hide_banner", "-filters"])?;
        Ok(Capabilities {
            major,
            minor,
            libx264: has_token(&encoders, "libx264"),
            libx265: has_token(&encoders, "libx265"),
            aac: has_token(&encoders, "aac"),
            filters_ok: ["scale", "setsar", "yadif", "concat"]
                .iter()
                .all(|f| has_token(&filters, f)),
        })
    }
}

fn has_token(haystack: &str, needle: &str) -> bool {
    haystack
        .split_whitespace()
        .any(|token| token == needle)
}

fn run_capture(program: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("cannot run {}", program.display()))?;
    if !output.status.success() {
        bail!("{} failed", program.display());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr))
}

fn cache_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let base = if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Caches")
    } else if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(xdg)
    } else {
        PathBuf::from(home).join(".cache")
    };
    Some(base.join("fictions").join("ffmpeg.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn caps() -> Capabilities {
        Capabilities {
            major: 7,
            minor: 1,
            libx264: true,
            libx265: true,
            aac: true,
            filters_ok: true,
        }
    }

    #[test]
    fn parses_version_strings() {
        assert_eq!(
            parse_version("ffmpeg version 6.1.1 Copyright (c) 2000-2023"),
            Some((6, 1))
        );
        assert_eq!(
            parse_version("ffmpeg version n9.0.2 Copyright (c) 2000-2026"),
            Some((9, 0))
        );
        assert_eq!(parse_version("garbage"), None);
    }

    #[test]
    fn cache_invalid_after_seven_days() {
        let id = BinaryId {
            path: "/x/ffmpeg".into(),
            mtime: 1,
            size: 1,
        };
        let entry = CacheEntry {
            id: id.clone(),
            checked_at: 0,
            caps: caps(),
        };
        let six_days = SystemTime::UNIX_EPOCH + Duration::from_secs(6 * 86400);
        let eight_days = SystemTime::UNIX_EPOCH + Duration::from_secs(8 * 86400);
        assert!(cache_is_valid(&entry, &id, six_days));
        assert!(!cache_is_valid(&entry, &id, eight_days));
    }

    #[test]
    fn cache_invalid_when_binary_changes() {
        let entry = CacheEntry {
            id: BinaryId {
                path: "/x/ffmpeg".into(),
                mtime: 1,
                size: 1,
            },
            checked_at: 0,
            caps: caps(),
        };
        let other = BinaryId {
            path: "/x/ffmpeg".into(),
            mtime: 2,
            size: 1,
        };
        assert!(!cache_is_valid(&entry, &other, SystemTime::UNIX_EPOCH));
    }

    #[test]
    fn too_old_is_an_error() {
        let old = Capabilities {
            major: 5,
            minor: 1,
            ..caps()
        };
        assert!(check_minimum(&old).is_err());
        assert!(check_minimum(&caps()).is_ok());
    }

    #[test]
    fn encoders_map_from_capabilities() {
        let e = caps().encoders();
        assert!(e.libx264 && e.libx265);
    }
}
