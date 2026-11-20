use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};

#[derive(Debug, Clone)]
pub struct Part {
    pub path: PathBuf,
    pub order: Option<u32>,
    pub digest: [u8; 32],
}

#[derive(Debug)]
pub struct Group {
    pub base: String,
    pub parts: Vec<Part>,
    pub hash8: String,
    pub dropped: Vec<PathBuf>,
}

pub fn strip_index(stem: &str) -> Option<(&str, u32)> {
    let open = stem.rfind(" (")?;
    let head = &stem[..open];
    let body = stem[open + 2..].strip_suffix(')')?;
    if head.is_empty() || body.is_empty() || !body.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((head, body.parse().ok()?))
}

pub fn single(input: &Path) -> anyhow::Result<Group> {
    if !input.is_file() {
        return Err(anyhow!("input is not a file: {}", input.display()));
    }
    let bytes = std::fs::read(input)?;
    let digest = *blake3::hash(&bytes).as_bytes();
    let base = input
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("input has no usable file name: {}", input.display()))?
        .to_string();
    Ok(Group {
        base,
        hash8: hash8_of([digest]),
        parts: vec![Part {
            path: input.to_path_buf(),
            order: None,
            digest,
        }],
        dropped: Vec::new(),
    })
}

fn hash8_of(digests: impl IntoIterator<Item = [u8; 32]>) -> String {
    let mut hasher = blake3::Hasher::new();
    for digest in digests {
        hasher.update(digest.as_slice());
    }
    let hex = hasher.finalize().to_hex();
    hex[..8].to_string()
}

pub fn detect_group(input: &Path) -> anyhow::Result<Group> {
    if !input.is_file() {
        return Err(anyhow!("input is not a file: {}", input.display()));
    }
    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| anyhow!("input has no extension: {}", input.display()))?
        .to_string();
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("input has no usable file name: {}", input.display()))?;
    let dir = input.parent().unwrap_or_else(|| Path::new("."));

    let base = match strip_index(stem) {
        Some((candidate, _)) if dir.join(format!("{candidate}.{ext}")).is_file() => {
            candidate.to_string()
        }
        _ => stem.to_string(),
    };

    let mut parts = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let path = entry?.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some(ext.as_str()) {
            continue;
        }
        let Some(other) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let order = if other == base {
            Some(None)
        } else {
            match strip_index(other) {
                Some((head, n)) if head == base => Some(Some(n)),
                _ => None,
            }
        };
        let Some(order) = order else { continue };
        let bytes = std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?;
        parts.push(Part {
            path,
            order,
            digest: *blake3::hash(&bytes).as_bytes(),
        });
    }

    parts.sort_by(|a, b| {
        (a.order.is_some(), a.order.unwrap_or(0), &a.path).cmp(&(
            b.order.is_some(),
            b.order.unwrap_or(0),
            &b.path,
        ))
    });

    let mut seen = HashSet::new();
    let mut dropped = Vec::new();
    parts.retain(|part| {
        if seen.insert(part.digest) {
            true
        } else {
            dropped.push(part.path.clone());
            false
        }
    });

    if parts.is_empty() {
        return Err(anyhow!("no parts found for {}", input.display()));
    }

    let hash8 = hash8_of(parts.iter().map(|p| p.digest));

    Ok(Group {
        base,
        parts,
        hash8,
        dropped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_only_trailing_index() {
        assert_eq!(strip_index("foo (12)"), Some(("foo", 12)));
        assert_eq!(strip_index("foo"), None);
        assert_eq!(strip_index("foo (12) bar"), None);
    }

    #[test]
    fn orders_numerically_with_base_first() {
        let names = ["foo (10)", "foo", "foo (2)"];
        let mut v: Vec<u32> = names
            .iter()
            .map(|n| strip_index(n).map(|(_, i)| i).unwrap_or(0))
            .collect();
        v.sort();
        assert_eq!(v, vec![0, 2, 10]);
    }
}
