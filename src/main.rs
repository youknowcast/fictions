use std::process::ExitCode;

use fictions::cli;
use fictions::pipeline::{self, Options, Outcome};

fn main() -> ExitCode {
    let cli = match cli::parse_from(std::env::args_os()) {
        Ok(cli) => cli,
        Err(err) => err.exit(),
    };

    let options = Options {
        force: cli.force,
        no_sandbox: cli.no_sandbox,
        no_group: cli.no_group,
        dry_run: cli.dry_run,
        quiet: cli.quiet,
    };

    match pipeline::run(&cli.input, &options) {
        Ok(outcome) => {
            if let Outcome::DryRun(text) = &outcome {
                println!("{text}");
            }
            if let Some(path) = pipeline::output_path(&outcome) {
                if let Outcome::Skipped(_) = outcome {
                    if !options.quiet {
                        eprintln!("skipped (output already exists): {}", path.display());
                    }
                }
                println!("{}", path.display());
            }
            if let Outcome::Suspect(_, detail) = &outcome {
                eprintln!("fictions: the input looked damaged; the output may not play correctly");
                if !detail.is_empty() {
                    eprintln!("{detail}");
                }
            }
            ExitCode::from(pipeline::exit_code(&outcome))
        }
        Err(err) => {
            eprintln!("fictions: {err:#}");
            ExitCode::from(1)
        }
    }
}
