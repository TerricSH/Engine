#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use engine_asset::cook::{cook_orchestrate_checked, DependencyGraph};

#[derive(Debug)]
struct Options {
    source: PathBuf,
    output: PathBuf,
    report: PathBuf,
}

fn parse_options() -> Result<Options, String> {
    let mut source = PathBuf::from("assets/source");
    let mut output = PathBuf::from("assets/cooked");
    let mut report = PathBuf::from("artifacts/asset-cook-report.json");
    let mut arguments = std::env::args().skip(1);

    while let Some(argument) = arguments.next() {
        let value = match argument.as_str() {
            "--source" | "--output" | "--report" => arguments
                .next()
                .ok_or_else(|| format!("{argument} requires a path"))?,
            "--help" | "-h" => {
                return Err(
                    "usage: asset-cook [--source PATH] [--output PATH] [--report PATH]".into(),
                );
            }
            _ => return Err(format!("unknown argument '{argument}'")),
        };

        match argument.as_str() {
            "--source" => source = value.into(),
            "--output" => output = value.into(),
            "--report" => report = value.into(),
            _ => unreachable!(),
        }
    }

    Ok(Options {
        source,
        output,
        report,
    })
}

fn run(options: Options) -> Result<bool, String> {
    let mut graph = DependencyGraph::new();
    let report = cook_orchestrate_checked(&options.source, &options.output, &mut graph);
    let report_json = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("could not serialize asset cook report: {error}"))?;
    if let Some(parent) = options.report.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create report directory: {error}"))?;
    }
    std::fs::write(&options.report, format!("{report_json}\n"))
        .map_err(|error| format!("could not write asset cook report: {error}"))?;

    println!(
        "asset cook: {} succeeded, {} failed, {} manifest failures; report: {}",
        report.succeeded_asset_count,
        report.failed_asset_count,
        report.failed_manifest_count,
        options.report.display()
    );
    for diagnostic in &report.diagnostics {
        eprintln!("{}: {}", diagnostic.code, diagnostic.message);
    }
    Ok(report.is_success())
}

fn main() -> ExitCode {
    let options = match parse_options() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("asset-cook: {error}");
            return ExitCode::from(2);
        }
    };
    match run(options) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("asset-cook: {error}");
            ExitCode::FAILURE
        }
    }
}
