#![forbid(unsafe_code)]

use serde::Serialize;
use std::{
    fmt,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

pub const REPORT_SCHEMA_VERSION: &str = "1.0.0";
pub const CREDENTIAL_SERVICE: &str = "CyberKindred/probe/task-003";
pub const CREDENTIAL_USER: &str = "credential-canary";
pub const AUTOSTART_VALUE_NAME: &str = "CyberKindred.Task003Probe";
pub const AUTOSTART_SILENT_ARGUMENTS: &str = "preflight --silent";
pub const CONFIRM_CREDENTIAL_WRITE: &str = "--confirm-probe-write";
pub const CONFIRM_AUTOSTART_CHANGE: &str = "--confirm-user-scope";
pub const CONFIRM_NOTIFICATION_SIMULATION: &str = "--confirm-local-only";
pub const CONFIRM_RESET: &str = "--confirm-probe-only";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Preflight { silent: bool },
    CredentialCycle,
    AutostartStatus,
    AutostartEnable,
    AutostartDisable,
    SimulateNotificationAction(NotificationAction),
    Reset,
    Help,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationAction {
    Start,
    Snooze,
    Ignore,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationActionRecord {
    pub schema_version: &'static str,
    pub action: NotificationAction,
    pub explicit_action_recorded: bool,
    pub program_started: bool,
    pub audio_started: bool,
    pub paid_provider_calls: u8,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialCycleReport {
    pub schema_version: &'static str,
    pub target_scope: &'static str,
    pub write_succeeded: bool,
    pub read_exists: bool,
    pub read_matches: bool,
    pub delete_succeeded: bool,
    pub absent_after_delete: bool,
    pub secret_output_fields: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutostartState {
    Disabled,
    EnabledExpectedSilentCommand,
    UnexpectedValueAtProbeName,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutostartReport {
    pub schema_version: &'static str,
    pub registry_scope: &'static str,
    pub value_name: &'static str,
    pub state: AutostartState,
    pub startup_is_silent: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetReport {
    pub schema_version: &'static str,
    pub credential_absent: bool,
    pub autostart_absent: bool,
    pub notification_action_log_absent: bool,
    pub artifact_directory_removed_if_empty: bool,
    pub recursive_delete_used: bool,
    pub user_files_touched: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    NotDetected,
    NotRunManual,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightReport {
    pub schema_version: &'static str,
    pub platform: &'static str,
    pub architecture: &'static str,
    pub default_side_effects: DefaultSideEffects,
    pub credential_manager: CheckStatus,
    pub autostart: AutostartReport,
    pub webview2_runtime: CheckStatus,
    pub rust_msvc_toolchain: CheckStatus,
    pub node_tool: CheckStatus,
    pub pnpm_tool: CheckStatus,
    pub nsis_makensis_tool: CheckStatus,
    pub tauri_shell_build: CheckStatus,
    pub tray_real_action: CheckStatus,
    pub notification_real_action: CheckStatus,
    pub clean_standard_user_vm: CheckStatus,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultSideEffects {
    pub emits_notification: bool,
    pub enables_autostart: bool,
    pub starts_audio_or_program: bool,
    pub calls_paid_provider: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeError {
    stage: &'static str,
    kind: &'static str,
}

impl ProbeError {
    #[must_use]
    pub const fn new(stage: &'static str, kind: &'static str) -> Self {
        Self { stage, kind }
    }
}

impl fmt::Display for ProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.stage, self.kind)
    }
}

impl std::error::Error for ProbeError {}

pub trait CredentialBackend {
    fn set(&mut self, secret: &str) -> Result<(), ProbeError>;
    fn exists(&self) -> Result<bool, ProbeError>;
    fn matches(&self, secret: &str) -> Result<bool, ProbeError>;
    fn delete(&mut self) -> Result<(), ProbeError>;
}

pub trait AutostartBackend {
    fn read(&self) -> Result<Option<String>, ProbeError>;
    fn write(&mut self, command: &str) -> Result<(), ProbeError>;
    fn delete(&mut self) -> Result<(), ProbeError>;
}

pub fn parse_command<I, S>(arguments: I) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = arguments.into_iter().map(Into::into).collect::<Vec<_>>();
    match args.as_slice() {
        [] => Ok(Command::Preflight { silent: false }),
        [single] if single == "preflight" => Ok(Command::Preflight { silent: false }),
        [command, option] if command == "preflight" && option == "--silent" => {
            Ok(Command::Preflight { silent: true })
        }
        [command, confirmation]
            if command == "credential-cycle" && confirmation == CONFIRM_CREDENTIAL_WRITE =>
        {
            Ok(Command::CredentialCycle)
        }
        [left, right] if left == "autostart" && right == "status" => Ok(Command::AutostartStatus),
        [left, right, confirmation]
            if left == "autostart"
                && right == "enable"
                && confirmation == CONFIRM_AUTOSTART_CHANGE =>
        {
            Ok(Command::AutostartEnable)
        }
        [left, right, confirmation]
            if left == "autostart"
                && right == "disable"
                && confirmation == CONFIRM_AUTOSTART_CHANGE =>
        {
            Ok(Command::AutostartDisable)
        }
        [command, action, confirmation]
            if command == "simulate-notification-action"
                && confirmation == CONFIRM_NOTIFICATION_SIMULATION =>
        {
            parse_notification_action(action).map(Command::SimulateNotificationAction)
        }
        [command, confirmation] if command == "reset" && confirmation == CONFIRM_RESET => {
            Ok(Command::Reset)
        }
        [command] if matches!(command.as_str(), "help" | "--help" | "-h") => Ok(Command::Help),
        _ => Err(format!("invalid or unconfirmed command\n{}", usage())),
    }
}

fn parse_notification_action(value: &str) -> Result<NotificationAction, String> {
    match value {
        "start" => Ok(NotificationAction::Start),
        "snooze" => Ok(NotificationAction::Snooze),
        "ignore" => Ok(NotificationAction::Ignore),
        _ => Err("notification action must be start, snooze, or ignore".to_owned()),
    }
}

#[must_use]
pub fn usage() -> &'static str {
    "Usage:\n  cyberkindred-windows-probe preflight [--silent]\n  cyberkindred-windows-probe credential-cycle --confirm-probe-write\n  cyberkindred-windows-probe autostart status\n  cyberkindred-windows-probe autostart enable|disable --confirm-user-scope\n  cyberkindred-windows-probe simulate-notification-action start|snooze|ignore --confirm-local-only\n  cyberkindred-windows-probe reset --confirm-probe-only"
}

pub fn credential_cycle(
    backend: &mut impl CredentialBackend,
) -> Result<CredentialCycleReport, ProbeError> {
    backend.delete()?;
    let secret = Zeroizing::new(format!(
        "ck-task003-canary-{}-{}",
        std::process::id(),
        now_unix_nanos()
    ));
    if let Err(error) = backend.set(secret.as_str()) {
        let _ = backend.delete();
        return Err(error);
    }
    let read_exists = match backend.exists() {
        Ok(value) => value,
        Err(error) => {
            let _ = backend.delete();
            return Err(error);
        }
    };
    let read_matches = match backend.matches(secret.as_str()) {
        Ok(value) => value,
        Err(error) => {
            let _ = backend.delete();
            return Err(error);
        }
    };
    backend.delete()?;
    let absent_after_delete = !backend.exists()?;
    Ok(CredentialCycleReport {
        schema_version: REPORT_SCHEMA_VERSION,
        target_scope: CREDENTIAL_SERVICE,
        write_succeeded: true,
        read_exists,
        read_matches,
        delete_succeeded: true,
        absent_after_delete,
        secret_output_fields: 0,
    })
}

pub fn autostart_status(
    backend: &impl AutostartBackend,
    expected_command: &str,
) -> Result<AutostartReport, ProbeError> {
    let value = backend.read()?;
    let state = match value.as_deref() {
        None => AutostartState::Disabled,
        Some(actual) if actual == expected_command => AutostartState::EnabledExpectedSilentCommand,
        Some(_) => AutostartState::UnexpectedValueAtProbeName,
    };
    Ok(AutostartReport {
        schema_version: REPORT_SCHEMA_VERSION,
        registry_scope: "HKCU/Software/Microsoft/Windows/CurrentVersion/Run",
        value_name: AUTOSTART_VALUE_NAME,
        state,
        startup_is_silent: state == AutostartState::EnabledExpectedSilentCommand,
    })
}

pub fn enable_autostart(
    backend: &mut impl AutostartBackend,
    executable: &Path,
) -> Result<AutostartReport, ProbeError> {
    let command = expected_autostart_command(executable)?;
    backend.write(&command)?;
    autostart_status(backend, &command)
}

pub fn disable_autostart(
    backend: &mut impl AutostartBackend,
    executable: &Path,
) -> Result<AutostartReport, ProbeError> {
    backend.delete()?;
    let command = expected_autostart_command(executable)?;
    autostart_status(backend, &command)
}

pub fn expected_autostart_command(executable: &Path) -> Result<String, ProbeError> {
    if !executable.is_absolute() {
        return Err(ProbeError::new("autostart", "executable_not_absolute"));
    }
    let value = executable.to_string_lossy();
    if value.contains(['"', '\r', '\n']) {
        return Err(ProbeError::new(
            "autostart",
            "executable_contains_unsafe_character",
        ));
    }
    Ok(format!("\"{value}\" {AUTOSTART_SILENT_ARGUMENTS}"))
}

#[must_use]
pub fn notification_action_record(action: NotificationAction) -> NotificationActionRecord {
    NotificationActionRecord {
        schema_version: REPORT_SCHEMA_VERSION,
        action,
        explicit_action_recorded: true,
        program_started: false,
        audio_started: false,
        paid_provider_calls: 0,
    }
}

pub fn probe_artifact_dir(local_app_data: &Path) -> Result<PathBuf, ProbeError> {
    if !local_app_data.is_absolute() {
        return Err(ProbeError::new(
            "artifact_scope",
            "local_app_data_not_absolute",
        ));
    }
    Ok(local_app_data
        .join("CyberKindred")
        .join("probe")
        .join("task-003"))
}

pub fn record_notification_action(
    local_app_data: &Path,
    record: &NotificationActionRecord,
) -> Result<(), ProbeError> {
    let directory = probe_artifact_dir(local_app_data)?;
    fs::create_dir_all(&directory)
        .map_err(|_| ProbeError::new("notification_action", "create_owned_dir_failed"))?;
    let log_path = directory.join("notification-actions.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|_| ProbeError::new("notification_action", "open_owned_log_failed"))?;
    serde_json::to_writer(&mut file, record)
        .map_err(|_| ProbeError::new("notification_action", "serialize_failed"))?;
    writeln!(file).map_err(|_| ProbeError::new("notification_action", "write_failed"))
}

pub fn delete_notification_action_log(local_app_data: &Path) -> Result<(bool, bool), ProbeError> {
    let directory = probe_artifact_dir(local_app_data)?;
    let log_path = directory.join("notification-actions.jsonl");
    match fs::remove_file(&log_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(ProbeError::new("reset", "remove_owned_action_log_failed")),
    }
    let log_absent = !log_path.exists();
    let directory_removed = match fs::remove_dir(&directory) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => false,
        Err(_) => false,
    };
    Ok((log_absent, directory_removed))
}

fn now_unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}

#[cfg(windows)]
mod platform;

#[cfg(windows)]
pub use platform::{
    WindowsAutostart, WindowsCredentialStore, local_app_data, preflight, reset_probe,
};

#[cfg(not(windows))]
pub fn preflight() -> Result<PreflightReport, ProbeError> {
    Err(ProbeError::new("preflight", "windows_only"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeCredential {
        secret: Option<String>,
    }

    impl CredentialBackend for FakeCredential {
        fn set(&mut self, secret: &str) -> Result<(), ProbeError> {
            self.secret = Some(secret.to_owned());
            Ok(())
        }

        fn exists(&self) -> Result<bool, ProbeError> {
            Ok(self.secret.is_some())
        }

        fn matches(&self, secret: &str) -> Result<bool, ProbeError> {
            Ok(self.secret.as_deref() == Some(secret))
        }

        fn delete(&mut self) -> Result<(), ProbeError> {
            self.secret = None;
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeAutostart(Option<String>);

    impl AutostartBackend for FakeAutostart {
        fn read(&self) -> Result<Option<String>, ProbeError> {
            Ok(self.0.clone())
        }

        fn write(&mut self, command: &str) -> Result<(), ProbeError> {
            self.0 = Some(command.to_owned());
            Ok(())
        }

        fn delete(&mut self) -> Result<(), ProbeError> {
            self.0 = None;
            Ok(())
        }
    }

    #[derive(Default)]
    struct FailingCredential {
        secret: Option<String>,
        delete_count: u8,
    }

    impl CredentialBackend for FailingCredential {
        fn set(&mut self, secret: &str) -> Result<(), ProbeError> {
            self.secret = Some(secret.to_owned());
            Ok(())
        }

        fn exists(&self) -> Result<bool, ProbeError> {
            Ok(self.secret.is_some())
        }

        fn matches(&self, _secret: &str) -> Result<bool, ProbeError> {
            Err(ProbeError::new("credential_read", "injected_failure"))
        }

        fn delete(&mut self) -> Result<(), ProbeError> {
            self.secret = None;
            self.delete_count = self.delete_count.saturating_add(1);
            Ok(())
        }
    }

    #[test]
    fn default_command_is_read_only_preflight() {
        assert_eq!(
            parse_command(Vec::<String>::new()),
            Ok(Command::Preflight { silent: false })
        );
        assert!(parse_command(["credential-cycle"]).is_err());
        assert!(parse_command(["autostart", "enable"]).is_err());
        assert!(parse_command(["reset"]).is_err());
    }

    #[test]
    fn credential_cycle_reports_presence_but_never_secret_and_cleans_up() {
        let mut backend = FakeCredential::default();
        let report = credential_cycle(&mut backend).expect("fake cycle should succeed");
        let json = serde_json::to_string(&report).expect("report should serialize");
        assert!(report.read_exists);
        assert!(report.read_matches);
        assert!(report.absent_after_delete);
        assert_eq!(report.secret_output_fields, 0);
        assert!(backend.secret.is_none());
        assert!(!json.contains("ck-task003-canary"));
    }

    #[test]
    fn credential_cycle_cleans_up_after_a_read_failure() {
        let mut backend = FailingCredential::default();
        let error = credential_cycle(&mut backend).expect_err("injected read failure");
        assert_eq!(error.kind, "injected_failure");
        assert!(backend.secret.is_none());
        assert_eq!(backend.delete_count, 2);
    }

    #[test]
    fn notification_action_only_records_intent() {
        for action in [
            NotificationAction::Start,
            NotificationAction::Snooze,
            NotificationAction::Ignore,
        ] {
            let result = notification_action_record(action);
            assert!(result.explicit_action_recorded);
            assert!(!result.program_started);
            assert!(!result.audio_started);
            assert_eq!(result.paid_provider_calls, 0);
        }
    }

    #[test]
    fn autostart_is_disabled_until_explicitly_enabled_and_always_silent() {
        let mut backend = FakeAutostart::default();
        let executable = Path::new(r"C:\Probe\cyberkindred-windows-probe.exe");
        let expected = expected_autostart_command(executable).expect("absolute test path");
        assert_eq!(
            autostart_status(&backend, &expected).expect("status").state,
            AutostartState::Disabled
        );
        let enabled = enable_autostart(&mut backend, executable).expect("enable");
        assert_eq!(enabled.state, AutostartState::EnabledExpectedSilentCommand);
        assert!(enabled.startup_is_silent);
        let disabled = disable_autostart(&mut backend, executable).expect("disable");
        assert_eq!(disabled.state, AutostartState::Disabled);

        backend.0 = Some("unexpected command".to_owned());
        assert_eq!(
            autostart_status(&backend, &expected)
                .expect("unexpected status")
                .state,
            AutostartState::UnexpectedValueAtProbeName
        );
    }

    #[test]
    fn reset_deletes_only_allowlisted_probe_action_log() {
        let base = std::env::temp_dir().join(format!("ck-task003-test-{}", now_unix_nanos()));
        let owned = probe_artifact_dir(&base).expect("absolute temp path");
        fs::create_dir_all(&owned).expect("create fixture");
        fs::write(owned.join("notification-actions.jsonl"), b"safe\n").expect("action fixture");
        let sibling = base.join("user-music-canary.wav");
        fs::write(&sibling, b"do-not-delete").expect("sibling fixture");

        let (log_absent, directory_removed) =
            delete_notification_action_log(&base).expect("scoped reset");
        assert!(log_absent);
        assert!(directory_removed);
        assert_eq!(
            fs::read(&sibling).expect("sibling remains"),
            b"do-not-delete"
        );

        fs::remove_file(sibling).expect("cleanup sibling");
        fs::remove_dir_all(base).expect("cleanup test base");
    }

    #[test]
    fn reset_refuses_relative_scope() {
        assert!(probe_artifact_dir(Path::new("relative")).is_err());
    }

    #[test]
    fn reset_preserves_unknown_files_even_inside_the_probe_directory() {
        let base = std::env::temp_dir().join(format!("ck-task003-extra-{}", now_unix_nanos()));
        let owned = probe_artifact_dir(&base).expect("absolute temp path");
        fs::create_dir_all(&owned).expect("create fixture");
        fs::write(owned.join("notification-actions.jsonl"), b"safe\n").expect("action fixture");
        let unknown = owned.join("unknown-user-file.txt");
        fs::write(&unknown, b"preserve").expect("unknown fixture");

        let (log_absent, directory_removed) =
            delete_notification_action_log(&base).expect("scoped reset");
        assert!(log_absent);
        assert!(!directory_removed);
        assert_eq!(fs::read(&unknown).expect("unknown remains"), b"preserve");

        fs::remove_dir_all(base).expect("cleanup fixture root");
    }
}
