use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail};

#[derive(Debug, Clone)]
pub enum Kind {
    MacProfile(String),
    Linux(PathBuf),
    Off,
}

#[derive(Debug, Clone)]
pub struct Sandbox {
    kind: Kind,
}

pub fn prepare(readable: &[PathBuf], writable: &Path, off: bool) -> anyhow::Result<Sandbox> {
    if off {
        return Ok(Sandbox { kind: Kind::Off });
    }
    if cfg!(target_os = "macos") {
        if !Path::new("/usr/bin/sandbox-exec").exists() {
            bail!("sandbox-exec is unavailable; rerun with --no-sandbox to proceed unisolated");
        }
        return Ok(Sandbox {
            kind: Kind::MacProfile(macos_profile(readable, writable)),
        });
    }
    if which("bwrap").is_none() {
        bail!("bwrap is unavailable; rerun with --no-sandbox to proceed unisolated");
    }
    Ok(Sandbox {
        kind: Kind::Linux(writable.to_path_buf()),
    })
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

pub fn macos_profile(readable: &[PathBuf], writable: &Path) -> String {
    let mut profile = String::from("(version 1)\n(deny default)\n(allow process-exec)\n");
    profile.push_str(
        "(allow file-read* (subpath \"/usr\") (subpath \"/System\") (subpath \"/Library\")\n",
    );
    profile.push_str(
        "                  (subpath \"/opt/homebrew\") (subpath \"/usr/local\")\n",
    );
    profile.push_str("                  (subpath \"/private/tmp\") (subpath \"/private/var/tmp\")");
    for path in readable {
        profile.push_str(&format!(
            "\n                  (literal \"{}\")",
            escape(&path.to_string_lossy())
        ));
    }
    profile.push_str(")\n");
    profile.push_str(&format!(
        "(allow file-write* (subpath \"{}\"))\n",
        escape(&writable.to_string_lossy())
    ));
    profile.push_str("(allow sysctl-read)\n(deny network*)\n");
    profile
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn linux_args(writable: &Path) -> Vec<String> {
    let dir = writable.to_string_lossy().into_owned();
    vec![
        "--unshare-all".into(),
        "--die-with-parent".into(),
        "--ro-bind".into(),
        "/".into(),
        "/".into(),
        "--bind".into(),
        dir.clone(),
        dir,
        "--dev".into(),
        "/dev".into(),
        "--proc".into(),
        "/proc".into(),
    ]
}

impl Sandbox {
    pub fn wrap(&self, program: &Path) -> Command {
        match &self.kind {
            Kind::Off => Command::new(program),
            Kind::MacProfile(profile) => {
                let mut cmd = Command::new("/usr/bin/sandbox-exec");
                cmd.arg("-p").arg(profile).arg(program);
                cmd
            }
            Kind::Linux(writable) => {
                let mut cmd = Command::new("bwrap");
                cmd.args(linux_args(writable)).arg(program);
                cmd
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_denies_network_and_limits_writes() {
        let p = macos_profile(&[PathBuf::from("/tmp/a.mp4")], Path::new("/tmp/out"));
        assert!(p.contains("(deny default)"));
        assert!(p.contains("(deny network*)"));
        assert!(p.contains("(literal \"/tmp/a.mp4\")"));
        assert!(p.contains("(subpath \"/tmp/out\")"));
        assert!(p.contains("(subpath \"/opt/homebrew\")"));
    }

    #[test]
    fn profile_escapes_quotes_in_paths() {
        let p = macos_profile(&[PathBuf::from("/tmp/a\"b.mp4")], Path::new("/tmp/out"));
        assert!(p.contains("(literal \"/tmp/a\\\"b.mp4\")"));
    }

    #[test]
    fn linux_args_use_bwrap() {
        let a = linux_args(Path::new("/tmp/out"));
        assert_eq!(a[0], "--unshare-all");
        assert!(a.contains(&"--ro-bind".to_string()));
        assert!(a.contains(&"/tmp/out".to_string()));
    }

    #[test]
    fn off_wraps_plainly() {
        let sb = prepare(&[], Path::new("/tmp"), true).unwrap();
        let cmd = sb.wrap(Path::new("/bin/echo"));
        assert_eq!(cmd.get_program(), "/bin/echo");
    }
}
