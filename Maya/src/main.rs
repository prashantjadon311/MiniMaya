use maya_mini::app::App;
use maya_mini::config::{self, CliArgs};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args = match CliArgs::parse_from(std::env::args()) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{}: {}", error.code(), error.message());
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .enable_all()
        .build()
    {
        Ok(value) => value,
        Err(error) => {
            eprintln!("INTERNAL: failed to initialize runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(async move {
        let config = config::load(args)?;
        App::build(config).await?.run().await
    }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}: {}", error.code(), error.message());
            ExitCode::FAILURE
        }
    }
}
