use cyberkindred_gsmtc_probe::{
    Command, ProbeObservation, observation_changed, parse_command, snapshot, usage,
};
use std::{
    env,
    error::Error,
    io::{self, Write},
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("GSMTC probe failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let command = parse_command(env::args().skip(1)).map_err(|message| {
        if message == usage() {
            message
        } else {
            format!("{message}\n{}", usage())
        }
    })?;

    match command {
        Command::Snapshot => write_json(&snapshot()?)?,
        Command::Watch {
            seconds,
            interval_ms,
        } => watch(seconds, interval_ms)?,
    }
    Ok(())
}

fn watch(seconds: u64, interval_ms: u64) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let interval = Duration::from_millis(interval_ms);
    let mut previous_report = None;
    let mut previous_sample_at = None;

    loop {
        let sample_at = Instant::now();
        let report = snapshot()?;
        let elapsed_ms = previous_sample_at.map_or(0, |previous: Instant| {
            sample_at.duration_since(previous).as_millis()
        });
        let changed = previous_report
            .as_ref()
            .is_none_or(|previous| observation_changed(previous, &report, elapsed_ms));
        if changed {
            write_json_line(&ProbeObservation::from(&report))?;
        }
        previous_report = Some(report);
        previous_sample_at = Some(sample_at);
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(interval);
    }
    Ok(())
}

fn write_json<T: serde::Serialize>(value: &T) -> Result<(), Box<dyn Error>> {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    serde_json::to_writer_pretty(&mut lock, value)?;
    writeln!(lock)?;
    Ok(())
}

fn write_json_line<T: serde::Serialize>(value: &T) -> Result<(), Box<dyn Error>> {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    serde_json::to_writer(&mut lock, value)?;
    writeln!(lock)?;
    Ok(())
}
