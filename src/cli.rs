use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "fictions",
    version,
    about = "Normalize downloaded media for local playback"
)]
pub struct Cli {
    pub input: PathBuf,
    #[arg(long)]
    pub force: bool,
    #[arg(long = "no-sandbox")]
    pub no_sandbox: bool,
    #[arg(long = "no-group")]
    pub no_group: bool,
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    #[arg(long)]
    pub quiet: bool,
}

pub fn parse_from<I, T>(args: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    Cli::try_parse_from(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags() {
        let c = parse_from(["fictions", "--no-group", "--force", "a.mp4"]).unwrap();
        assert_eq!(c.input, PathBuf::from("a.mp4"));
        assert!(c.no_group && c.force);
    }

    #[test]
    fn requires_input() {
        assert!(parse_from(["fictions"]).is_err());
    }
}
