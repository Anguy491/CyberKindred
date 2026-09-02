use crate::{
    AUTOSTART_VALUE_NAME, AutostartBackend, CheckStatus, CredentialBackend, DefaultSideEffects,
    PreflightReport, ProbeError, REPORT_SCHEMA_VERSION, ResetReport, autostart_status,
    delete_notification_action_log, expected_autostart_command,
};
use std::{
    env,
    path::PathBuf,
    process::{Command, Output, Stdio},
};
use windows::{
    Security::Credentials::{PasswordCredential, PasswordVault},
    core::HSTRING,
};
use zeroize::Zeroizing;

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const WEBVIEW2_CLIENT_ID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
const HRESULT_NOT_FOUND: u32 = 0x8007_0490;

pub struct WindowsCredentialStore {
    vault: PasswordVault,
}

impl WindowsCredentialStore {
    pub fn new() -> Result<Self, ProbeError> {
        PasswordVault::new()
            .map(|vault| Self { vault })
            .map_err(|_| ProbeError::new("credential", "credential_manager_unavailable"))
    }

    fn retrieve(&self) -> Result<Option<PasswordCredential>, ProbeError> {
        let resource = HSTRING::from(crate::CREDENTIAL_SERVICE);
        let user = HSTRING::from(crate::CREDENTIAL_USER);
        match self.vault.Retrieve(&resource, &user) {
            Ok(credential) => Ok(Some(credential)),
            Err(error) if error.code().0.cast_unsigned() == HRESULT_NOT_FOUND => Ok(None),
            Err(_) => Err(ProbeError::new(
                "credential_read",
                "credential_manager_error",
            )),
        }
    }
}

impl CredentialBackend for WindowsCredentialStore {
    fn set(&mut self, secret: &str) -> Result<(), ProbeError> {
        let resource = HSTRING::from(crate::CREDENTIAL_SERVICE);
        let user = HSTRING::from(crate::CREDENTIAL_USER);
        let password = HSTRING::from(secret);
        let credential = PasswordCredential::CreatePasswordCredential(&resource, &user, &password)
            .map_err(|_| ProbeError::new("credential_write", "credential_manager_error"))?;
        self.vault
            .Add(&credential)
            .map_err(|_| ProbeError::new("credential_write", "credential_manager_error"))
    }

    fn exists(&self) -> Result<bool, ProbeError> {
        self.retrieve().map(|credential| credential.is_some())
    }

    fn matches(&self, secret: &str) -> Result<bool, ProbeError> {
        let Some(credential) = self.retrieve()? else {
            return Ok(false);
        };
        credential
            .RetrievePassword()
            .map_err(|_| ProbeError::new("credential_read", "credential_manager_error"))?;
        let password = credential
            .Password()
            .map_err(|_| ProbeError::new("credential_read", "credential_manager_error"))?;
        let password = Zeroizing::new(password.to_string());
        Ok(password.as_str() == secret)
    }

    fn delete(&mut self) -> Result<(), ProbeError> {
        let Some(credential) = self.retrieve()? else {
            return Ok(());
        };
        self.vault
            .Remove(&credential)
            .map_err(|_| ProbeError::new("credential_delete", "credential_manager_error"))
    }
}

#[derive(Default)]
pub struct WindowsAutostart;

impl AutostartBackend for WindowsAutostart {
    fn read(&self) -> Result<Option<String>, ProbeError> {
        let output = reg_output(["query", RUN_KEY, "/v", AUTOSTART_VALUE_NAME])?;
        if !output.status.success() {
            let key_status = reg_output(["query", RUN_KEY])?;
            if key_status.status.success() {
                return Ok(None);
            }
            let parent_status =
                reg_output(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion"])?;
            return if parent_status.status.success() {
                Ok(None)
            } else {
                Err(ProbeError::new("autostart_read", "registry_error"))
            };
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.lines().find_map(|line| {
            let (_, value) = line.split_once("REG_SZ")?;
            line.contains(AUTOSTART_VALUE_NAME)
                .then(|| value.trim().to_owned())
        }))
    }

    fn write(&mut self, command: &str) -> Result<(), ProbeError> {
        let output = reg_output([
            "add",
            RUN_KEY,
            "/v",
            AUTOSTART_VALUE_NAME,
            "/t",
            "REG_SZ",
            "/d",
            command,
            "/f",
        ])?;
        if output.status.success() {
            Ok(())
        } else {
            Err(ProbeError::new("autostart_write", "registry_error"))
        }
    }

    fn delete(&mut self) -> Result<(), ProbeError> {
        if self.read()?.is_none() {
            return Ok(());
        }
        let output = reg_output(["delete", RUN_KEY, "/v", AUTOSTART_VALUE_NAME, "/f"])?;
        if output.status.success() {
            Ok(())
        } else {
            Err(ProbeError::new("autostart_delete", "registry_error"))
        }
    }
}

pub fn local_app_data() -> Result<PathBuf, ProbeError> {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| ProbeError::new("artifact_scope", "local_app_data_unavailable"))
}

pub fn preflight() -> Result<PreflightReport, ProbeError> {
    let executable = env::current_exe()
        .map_err(|_| ProbeError::new("preflight", "current_executable_unavailable"))?;
    let expected = expected_autostart_command(&executable)?;
    let autostart = autostart_status(&WindowsAutostart, &expected)?;
    let credential_manager = detected_status(PasswordVault::new().is_ok());
    Ok(PreflightReport {
        schema_version: REPORT_SCHEMA_VERSION,
        platform: "windows",
        architecture: std::env::consts::ARCH,
        default_side_effects: DefaultSideEffects::default(),
        credential_manager,
        autostart,
        webview2_runtime: detected_status(webview2_detected()),
        rust_msvc_toolchain: detected_status(cfg!(target_env = "msvc") && tool_available("rustc")),
        node_tool: detected_status(tool_available("node")),
        pnpm_tool: detected_status(tool_available("pnpm")),
        nsis_makensis_tool: detected_status(tool_available("makensis")),
        tauri_shell_build: CheckStatus::NotRunManual,
        tray_real_action: CheckStatus::NotRunManual,
        notification_real_action: CheckStatus::NotRunManual,
        clean_standard_user_vm: CheckStatus::NotRunManual,
    })
}

pub fn reset_probe() -> Result<ResetReport, ProbeError> {
    let mut credential = WindowsCredentialStore::new()?;
    credential.delete()?;
    let credential_absent = !credential.exists()?;

    let executable = env::current_exe()
        .map_err(|_| ProbeError::new("reset", "current_executable_unavailable"))?;
    let expected = expected_autostart_command(&executable)?;
    let mut autostart = WindowsAutostart;
    autostart.delete()?;
    let autostart_absent = matches!(
        autostart_status(&autostart, &expected)?.state,
        crate::AutostartState::Disabled
    );
    let (notification_action_log_absent, artifact_directory_removed_if_empty) =
        delete_notification_action_log(&local_app_data()?)?;
    Ok(ResetReport {
        schema_version: REPORT_SCHEMA_VERSION,
        credential_absent,
        autostart_absent,
        notification_action_log_absent,
        artifact_directory_removed_if_empty,
        recursive_delete_used: false,
        user_files_touched: false,
    })
}

fn reg_output<const N: usize>(arguments: [&str; N]) -> Result<Output, ProbeError> {
    Command::new("reg.exe")
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .map_err(|_| ProbeError::new("registry", "reg_executable_unavailable"))
}

fn detected_status(detected: bool) -> CheckStatus {
    if detected {
        CheckStatus::Passed
    } else {
        CheckStatus::NotDetected
    }
}

fn tool_available(tool: &str) -> bool {
    Command::new("where.exe")
        .arg(tool)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn webview2_detected() -> bool {
    [
        format!(r"HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_CLIENT_ID}"),
        format!(r"HKLM\SOFTWARE\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_CLIENT_ID}"),
        format!(r"HKCU\SOFTWARE\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_CLIENT_ID}"),
    ]
    .iter()
    .any(|key| {
        reg_output(["query", key.as_str(), "/v", "pv"]).is_ok_and(|output| output.status.success())
    })
}
