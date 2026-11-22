use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use anyhow::{bail, Context};

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

pub fn prepare(
    readable: &[PathBuf],
    writable: &Path,
    binaries: &[PathBuf],
    off: bool,
) -> anyhow::Result<Sandbox> {
    if off {
        return Ok(Sandbox { kind: Kind::Off });
    }
    if cfg!(target_os = "macos") {
        if !Path::new("/usr/bin/sandbox-exec").exists() {
            bail!("sandbox-exec is unavailable; rerun with --no-sandbox to proceed unisolated");
        }
        let binaries: Vec<PathBuf> = binaries.iter().map(|b| resolve(b)).collect();
        return Ok(Sandbox {
            kind: Kind::MacProfile(macos_profile(readable, writable, &binaries)),
        });
    }
    if which("bwrap").is_none() {
        bail!("bwrap is unavailable; rerun with --no-sandbox to proceed unisolated");
    }
    Ok(Sandbox {
        kind: Kind::Linux(writable.to_path_buf()),
    })
}

fn resolve(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

pub fn macos_profile(readable: &[PathBuf], writable: &Path, binaries: &[PathBuf]) -> String {
    let mut profile = String::from("(version 1)\n(deny default)\n(allow process-exec)\n");
    profile.push_str("(allow file-read-metadata)\n");
    profile.push_str("(allow file-read* (literal \"/\")\n");
    profile.push_str(
        "                  (subpath \"/usr\") (subpath \"/System\") (subpath \"/Library\")\n",
    );
    profile.push_str("                  (subpath \"/opt/homebrew\") (subpath \"/usr/local\")\n");
    profile.push_str("                  (subpath \"/private/tmp\") (subpath \"/private/var/tmp\")");
    for path in readable {
        profile.push_str(&format!(
            "\n                  (literal \"{}\")",
            escape(&path.to_string_lossy())
        ));
    }
    profile.push_str(")\n");
    if !binaries.is_empty() {
        profile.push_str("(allow file-read*");
        for binary in binaries {
            profile.push_str(&format!(
                "\n                  (literal \"{}\")",
                escape(&binary.to_string_lossy())
            ));
            if let Some(parent) = binary.parent().filter(|p| !p.as_os_str().is_empty()) {
                profile.push_str(&format!(
                    "\n                  (subpath \"{}\")",
                    escape(&parent.to_string_lossy())
                ));
            }
        }
        profile.push_str(")\n");
    }
    profile.push_str(&format!(
        "(allow file-write* (subpath \"{}\"))\n",
        escape(&writable.to_string_lossy())
    ));
    profile.push_str(&format!(
        "(allow file-read* (subpath \"{}\"))\n",
        escape(&writable.to_string_lossy())
    ));
    profile.push_str("(allow sysctl-read)\n(deny network*)\n");
    profile
}

pub fn describe_status(status: &ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("exit code {code}"),
        (None, Some(signal)) => format!("killed by signal {signal}"),
        (None, None) => format!("{status:?}"),
    }
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
    pub fn check_ffprobe(&self, ffprobe: &Path) -> anyhow::Result<()> {
        if matches!(&self.kind, Kind::Off) {
            return Ok(());
        }
        let output = self
            .wrap(ffprobe)
            .arg("-version")
            .output()
            .with_context(|| format!("cannot run {}", ffprobe.display()))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        let detail = if stderr.is_empty() {
            "no stderr"
        } else {
            stderr
        };
        bail!(
            "the sandbox cannot launch ffprobe ({}): {detail}\n\
             rerun with --no-sandbox to proceed unisolated",
            describe_status(&output.status)
        );
    }

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
        let p = macos_profile(&[PathBuf::from("/tmp/a.mp4")], Path::new("/tmp/out"), &[]);
        assert!(p.contains("(deny default)"));
        assert!(p.contains("(deny network*)"));
        assert!(p.contains("(literal \"/tmp/a.mp4\")"));
        assert!(p.contains("(subpath \"/tmp/out\")"));
        assert!(p.contains("(subpath \"/opt/homebrew\")"));
    }

    #[test]
    fn profile_allows_metadata_lookups_and_the_root() {
        let p = macos_profile(&[PathBuf::from("/tmp/a.mp4")], Path::new("/tmp/out"), &[]);
        assert!(p.contains("(allow file-read-metadata)"));
        assert!(p.contains("(allow file-read* (literal \"/\")"));
    }

    #[test]
    fn profile_allows_resolved_binaries_and_their_parent() {
        let ffprobe = PathBuf::from("/Users/youknow/bin/ffprobe");
        let p = macos_profile(&[], Path::new("/tmp/out"), std::slice::from_ref(&ffprobe));
        assert!(p.contains("(literal \"/Users/youknow/bin/ffprobe\")"));
        assert!(p.contains("(subpath \"/Users/youknow/bin\")"));
    }

    #[test]
    fn profile_skips_an_empty_binary_parent() {
        let p = macos_profile(&[], Path::new("/tmp/out"), &[PathBuf::from("ffprobe")]);
        assert!(p.contains("(literal \"ffprobe\")"));
        assert!(!p.contains("(subpath \"\")"));
    }

    #[test]
    fn profile_is_balanced() {
        let p = macos_profile(
            &[PathBuf::from("/tmp/a (1).mp4")],
            Path::new("/tmp/out"),
            &[PathBuf::from("/opt/homebrew/Cellar/ffmpeg/9.0/bin/ffprobe")],
        );
        assert_eq!(p.matches('(').count(), p.matches(')').count(), "{p}");
    }

    #[test]
    fn profile_lets_verification_read_the_staged_output() {
        let p = macos_profile(&[PathBuf::from("/tmp/a.mp4")], Path::new("/tmp/out"), &[]);
        assert!(p.contains("(allow file-read* (subpath \"/tmp/out\"))"));
    }

    #[test]
    fn profile_escapes_quotes_in_paths() {
        let p = macos_profile(
            &[PathBuf::from("/tmp/a\"b.mp4")],
            Path::new("/tmp/out"),
            &[],
        );
        assert!(p.contains("(literal \"/tmp/a\\\"b.mp4\")"));
    }

    #[test]
    fn status_describes_exit_code() {
        let status = Command::new("/bin/sh")
            .args(["-c", "exit 3"])
            .status()
            .unwrap();
        assert_eq!(describe_status(&status), "exit code 3");
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
        let sb = prepare(&[], Path::new("/tmp"), &[], true).unwrap();
        let cmd = sb.wrap(Path::new("/bin/echo"));
        assert_eq!(cmd.get_program(), "/bin/echo");
    }

    #[test]
    fn off_skips_the_ffprobe_self_check() {
        let sb = prepare(&[], Path::new("/tmp"), &[], true).unwrap();
        assert!(sb.check_ffprobe(Path::new("/nonexistent/ffprobe")).is_ok());
    }
}
