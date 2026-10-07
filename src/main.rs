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

use clap::Parser;
use cli::{Cli, Command};
use config::{Config, Preset};
use error::Error;
use serde_json::json;

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
            eprintln!(
                "{}",
                json!({ "schema_version": 1, "status": "error", "error": Error::new("invalid_arguments", error.to_string()) })
            );
            std::process::exit(2);
        }
    };
    match execute(cli).await {
        Ok(result) => println!("{result}"),
        Err(error) => {
            eprintln!("{}", json!({ "schema_version": 1, "status": "error", "error": error }));
            std::process::exit(if error.code == "interrupted" { 130 } else { 1 });
        }
    }
}

async fn execute(cli: Cli) -> Result<serde_json::Value, Error> {
    match cli.command {
        Command::Schema => Ok(
            json!({ "schema_version": 1, "config_schema": schemars::schema_for!(Config), "defaults": Config::default() }),
        ),
        Command::Presets => {
            let presets = [Preset::Agent, Preset::Rag, Preset::Web]
                .into_iter()
                .map(|preset| json!({ "name": preset, "spans_per_record": preset.spans_per_record(), "logs_per_record": 1, "metric_points_per_record": preset.points_per_record() }))
                .collect::<Vec<_>>();
            Ok(json!({ "schema_version": 1, "presets": presets }))
        }
        Command::Generate(args) => {
            let (config, dry_run) = args.resolve()?;
            if dry_run {
                Ok(runner::plan(&config))
            } else {
                serde_json::to_value(runner::run(config).await?)
                    .map_err(|_| Error::new("encode_failed", "Cannot encode summary"))
            }
        }
    }
}
