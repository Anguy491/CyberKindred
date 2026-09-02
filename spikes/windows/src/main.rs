use cyberkindred_windows_probe::{
    Command, WindowsAutostart, WindowsCredentialStore, autostart_status, credential_cycle,
    disable_autostart, enable_autostart, expected_autostart_command, local_app_data,
    notification_action_record, parse_command, preflight, record_notification_action, reset_probe,
    usage,
};
use serde::Serialize;
use std::{
    env,
    error::Error,
    io::{self, Write},
    process::ExitCode,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Windows probe failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    match parse_command(env::args().skip(1)).map_err(io::Error::other)? {
        Command::Preflight { silent } => {
            let report = preflight()?;
            if !silent {
                write_json(&report)?;
            }
        }
        Command::CredentialCycle => {
            let mut backend = WindowsCredentialStore::new()?;
            write_json(&credential_cycle(&mut backend)?)?;
        }
        Command::AutostartStatus => write_json(&current_autostart_status()?)?,
        Command::AutostartEnable => {
            let mut backend = WindowsAutostart;
            write_json(&enable_autostart(&mut backend, &env::current_exe()?)?)?;
        }
        Command::AutostartDisable => {
            let mut backend = WindowsAutostart;
            write_json(&disable_autostart(&mut backend, &env::current_exe()?)?)?;
        }
        Command::SimulateNotificationAction(action) => {
            let record = notification_action_record(action);
            record_notification_action(&local_app_data()?, &record)?;
            write_json(&record)?;
        }
        Command::Reset => write_json(&reset_probe()?)?,
        Command::Help => println!("{}", usage()),
    }
    Ok(())
}

fn current_autostart_status() -> Result<cyberkindred_windows_probe::AutostartReport, Box<dyn Error>>
{
    let executable = env::current_exe()?;
    let expected = expected_autostart_command(&executable)?;
    Ok(autostart_status(&WindowsAutostart, &expected)?)
}

fn write_json(value: &impl Serialize) -> Result<(), Box<dyn Error>> {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    serde_json::to_writer_pretty(&mut lock, value)?;
    writeln!(lock)?;
    Ok(())
}
