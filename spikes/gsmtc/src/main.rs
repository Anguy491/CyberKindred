use cyberkindred_gsmtc_probe::{Command, ProbeObservation, parse_command, snapshot, usage};
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
    let mut previous_fingerprint = None;

    loop {
        let report = snapshot()?;
        let fingerprint = serde_json::to_vec(&report.sessions)?;
        if previous_fingerprint.as_ref() != Some(&fingerprint) {
            write_json_line(&ProbeObservation::from(&report))?;
            previous_fingerprint = Some(fingerprint);
        }
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
