mod cli;
mod config;
mod encode;
mod error;
mod logs;
mod metrics;
mod model;
mod runner;
mod semconv;
mod sink;
mod traces;

#[cfg(test)]
mod tests;

use clap::{CommandFactory, Parser};
use clap_complete::generate;
use cli::{Cli, Command};
use config::{Config, Preset};
use error::Error;
use serde_json::json;
use std::io::{self, Write};

#[tokio::main]
async fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            error.exit()
        }
        Err(error) => {
            write_stderr_json(&json!({
                "schema_version": 1,
                "status": "error",
                "error": Error::new("invalid_arguments", error.to_string())
            }));
            std::process::exit(2);
        }
    };
    match execute(cli).await {
        Ok(Output::Json(result)) => {
            if let Err(error) = write_stdout_json(&result)
                && error.kind() != io::ErrorKind::BrokenPipe
            {
                // A downstream command closing a pipeline is a normal Unix exit
                // condition. Other stdout failures are reported on stderr.
                write_stderr_json(&json!({
                    "schema_version": 1,
                    "status": "error",
                    "error": Error::new("output_write_failed", "Cannot write JSON result to stdout")
                }));
                std::process::exit(1);
            }
        }
        Ok(Output::Raw(bytes)) => {
            if let Err(error) = write_stdout_bytes(&bytes)
                && error.kind() != io::ErrorKind::BrokenPipe
            {
                write_stderr_json(&json!({
                    "schema_version": 1,
                    "status": "error",
                    "error": Error::new("output_write_failed", "Cannot write completion script to stdout")
                }));
                std::process::exit(1);
            }
        }
        Err(error) => {
            write_stderr_json(&json!({ "schema_version": 1, "status": "error", "error": error }));
            std::process::exit(if error.code == "interrupted" { 130 } else { 1 });
        }
    }
}

fn write_stdout_json(value: &serde_json::Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    write_stdout_bytes(&bytes)
}

fn write_stdout_bytes(bytes: &[u8]) -> io::Result<()> {
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    stdout.write_all(bytes)?;
    stdout.flush()
}

fn write_stderr_json(value: &serde_json::Value) {
    let mut stderr = io::BufWriter::new(io::stderr().lock());
    // There is no useful recovery path when stderr is closed. Keep the process
    // exit code deterministic and avoid a panic from eprintln!/write_all.
    if serde_json::to_writer(&mut stderr, value).is_ok() {
        let _ = stderr.write_all(b"\n");
        let _ = stderr.flush();
    }
}

enum Output {
    Json(serde_json::Value),
    Raw(Vec<u8>),
}

async fn execute(cli: Cli) -> Result<Output, Error> {
    match cli.command {
        Command::Schema => Ok(Output::Json(json!({
            "schema_version": 1,
            "config_schema": schemars::schema_for!(Config),
            "defaults": Config::default()
        }))),
        Command::Presets => {
            let presets = [Preset::Agent, Preset::Rag, Preset::Web]
                .into_iter()
                .map(|preset| json!({ "name": preset, "spans_per_record": preset.spans_per_record(), "logs_per_record": 1, "metric_points_per_record": preset.points_per_record() }))
                .collect::<Vec<_>>();
            Ok(Output::Json(json!({ "schema_version": 1, "presets": presets })))
        }
        Command::Completions { shell } => {
            let mut command = Cli::command();
            let mut output = Vec::new();
            generate(shell, &mut command, env!("CARGO_PKG_NAME"), &mut output);
            Ok(Output::Raw(output))
        }
        Command::Generate(args) => {
            let (config, dry_run) = args.resolve()?;
            if dry_run {
                Ok(Output::Json(runner::plan(&config)))
            } else {
                Ok(Output::Json(
                    serde_json::to_value(runner::run(config).await?)
                        .map_err(|_| Error::new("encode_failed", "Cannot encode summary"))?,
                ))
            }
        }
    }
}
