use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

use crate::encode::{self, Request};
use crate::ffmpeg;
use crate::group::{self, Group};
use crate::naming;
use crate::probe::{self, MediaInfo};
use crate::sandbox;
use crate::spec::{self, Container, OutputSpec};
use crate::verify;

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub force: bool,
    pub no_sandbox: bool,
    pub no_group: bool,
    pub dry_run: bool,
    pub quiet: bool,
}

#[derive(Debug)]
pub enum Outcome {
    Written(PathBuf),
    Skipped(PathBuf),
    Suspect(PathBuf, String),
    DryRun(PathBuf, String),
}

pub fn warning_line(msg: &str, quiet: bool) -> Option<String> {
    (!quiet).then(|| msg.to_string())
}

pub fn run(input: &Path, opts: &Options) -> anyhow::Result<Outcome> {
    let group = if opts.no_group {
        group::single(input)?
    } else {
        group::detect_group(input)?
    };
    for dropped in &group.dropped {
        if let Some(line) = warning_line(
            &format!("skipping duplicate part: {}", dropped.display()),
            opts.quiet,
        ) {
            eprintln!("{line}");
        }
    }

    let ff = ffmpeg::discover()?;
    let caps = ff.capabilities()?;

    let dir = group::parent_dir(input);
    let temp = tempfile::Builder::new()
        .prefix(".fictions-tmp-")
        .tempdir_in(&dir)
        .with_context(|| format!("cannot create a temporary directory in {}", dir.display()))?;

    let readable: Vec<PathBuf> = group
        .parts
        .iter()
        .map(|p| std::fs::canonicalize(&p.path).unwrap_or_else(|_| p.path.clone()))
        .collect();
    let sandbox = sandbox::prepare(&readable, temp.path(), opts.no_sandbox)?;

    let mut infos: Vec<MediaInfo> = Vec::new();
    for path in &readable {
        infos.push(probe::probe_file(&ff, &sandbox, path)?);
    }

    let decision = spec::decide(&infos, &caps.encoders())?;
    for warning in &decision.warnings {
        if let Some(line) = warning_line(warning, opts.quiet) {
            eprintln!("{line}");
        }
    }
    let spec = decision.spec;

    let ext = match spec.container {
        Container::Mp4 => "mp4",
        Container::M4a => "m4a",
    };
    let output = dir.join(naming::output_filename(&group.base, &group.hash8, ext));

    let expected_duration = total_duration(&infos)?;

    if opts.dry_run {
        let text = describe(&group, &spec, &output, expected_duration);
        return Ok(Outcome::DryRun(output, text));
    }

    if output.exists() && !opts.force {
        return Ok(Outcome::Skipped(output));
    }

    let staged = temp.path().join(format!("staged.{ext}"));
    let graph = if group.parts.len() > 1 {
        Some(crate::filtergraph::build(&infos, &spec)?)
    } else {
        None
    };
    let encode_result = encode::run(
        &ff,
        &sandbox,
        &Request {
            inputs: &readable,
            spec: &spec,
            output: &staged,
            progress: std::io::stderr().is_terminal(),
            interlaced: infos
                .iter()
                .filter_map(|i| i.video.as_ref())
                .any(|v| v.interlaced),
            duration: Some(expected_duration),
            filtergraph: graph.as_deref(),
        },
    );

    let outcome = match encode_result {
        Ok(outcome) => outcome,
        Err(err) => return Err(keep_temp(temp, &staged, err)),
    };

    let suspect = !outcome.stderr.trim().is_empty();

    if let Err(err) = verify::verify_output(&ff, &sandbox, &staged, &spec, expected_duration) {
        return Err(keep_temp(temp, &staged, err));
    }

    if let Err(err) = std::fs::rename(&staged, &output) {
        return Err(keep_temp(temp, &staged, err.into()));
    }
    drop(temp);

    let input_bytes: u64 = group
        .parts
        .iter()
        .filter_map(|p| std::fs::metadata(&p.path).ok())
        .map(|m| m.len())
        .sum();
    let output_bytes = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    if let Some(line) = growth_warning(input_bytes, output_bytes, opts.quiet) {
        eprintln!("{line}");
    }

    if suspect {
        return Ok(Outcome::Suspect(output, outcome.stderr.trim().to_string()));
    }
    Ok(Outcome::Written(output))
}

pub fn growth_warning(input_bytes: u64, output_bytes: u64, quiet: bool) -> Option<String> {
    if quiet || output_bytes <= input_bytes {
        return None;
    }
    Some(format!(
        "warning: the output is larger than the input ({} -> {} bytes)",
        input_bytes, output_bytes
    ))
}

fn keep_temp(temp: tempfile::TempDir, staged: &Path, err: anyhow::Error) -> anyhow::Error {
    let name = staged.file_name().map(|n| n.to_owned()).unwrap_or_default();
    let dir = temp.keep();
    anyhow::anyhow!(
        "{err}\nthe incomplete output was kept at {}",
        dir.join(name).display()
    )
}

fn total_duration(infos: &[MediaInfo]) -> anyhow::Result<f64> {
    let mut total = 0.0;
    for info in infos {
        match info.duration {
            Some(d) => total += d,
            None => bail!("cannot read the duration of an input"),
        }
    }
    Ok(total)
}

fn describe(group: &Group, spec: &OutputSpec, output: &Path, duration: f64) -> String {
    let video = match &spec.video {
        Some(v) => format!("{:?} {}x{} crf{}", v.codec, v.width, v.height, v.crf),
        None => "audio only".to_string(),
    };
    let audio = match &spec.audio {
        Some(a) => format!("{}ch {}k", a.channels, a.bitrate_kbps),
        None => "no audio".to_string(),
    };
    format!(
        "parts: {} ({:.2}s total)\nvideo: {video}\naudio: {audio}\noutput: {}",
        group.parts.len(),
        duration,
        output.display()
    )
}

pub fn output_path(outcome: &Outcome) -> Option<&Path> {
    match outcome {
        Outcome::Written(p)
        | Outcome::Skipped(p)
        | Outcome::Suspect(p, _)
        | Outcome::DryRun(p, _) => Some(p),
    }
}

pub fn exit_code(outcome: &Outcome) -> u8 {
    match outcome {
        Outcome::Suspect(_, _) => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_suppresses_warnings() {
        assert_eq!(warning_line("warn", true), None);
        assert!(warning_line("warn", false).is_some());
    }

    #[test]
    fn warns_when_output_grows() {
        assert!(growth_warning(100, 120, false).is_some());
        assert!(growth_warning(120, 100, false).is_none());
        assert!(growth_warning(100, 120, true).is_none());
    }

    #[test]
    fn failed_output_is_kept_for_inspection() {
        let temp = tempfile::tempdir().unwrap();
        let staged = temp.path().join("staged.mp4");
        std::fs::write(&staged, b"partial").unwrap();

        let err = keep_temp(temp, &staged, anyhow::anyhow!("encode failed"));

        assert!(err.to_string().contains("kept at"));
        let dir = err.to_string();
        let path = dir.split("kept at ").nth(1).unwrap().trim();
        assert!(std::path::Path::new(path).exists());
        std::fs::remove_dir_all(std::path::Path::new(path).parent().unwrap()).ok();
    }
}
