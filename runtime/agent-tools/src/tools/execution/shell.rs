// ============================================================================
// SHELL TOOL
// Execute shell commands with platform-aware shell selection and security guardrails
// ============================================================================

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::time::timeout;

use agent_primitives::{AgentError, Result, Tool, ToolContext, ToolPermissions};

// ============================================================================
// SECURITY CONFIGURATION
// ============================================================================

/// Commands that are completely blocked - these are too dangerous
const BLOCKED_COMMANDS: &[&str] = &[
    // Disk/partition destruction
    "mkfs",
    "dd if=",
    "dd of=",
    "fdisk",
    "parted",
    "gdisk",
    "diskutil",
    // System destruction
    "rm -rf /",
    "rm -rf /*",
    "rm -rf ~",
    "rm -rf ~/*",
    ":(){ :|:& };:", // Fork bomb
    // Permission/ownership attacks
    "chmod -R 777 /",
    "chown -R",
    // Network attacks
    "iptables -F",
    "ufw disable",
    "netsh advfirewall set",
    // Kernel/boot attacks
    "insmod",
    "rmmod",
    "modprobe",
    // Windows destructive
    "format c:",
    "format d:",
    "del /f /s /q c:\\",
    "rd /s /q c:\\",
    "diskpart",
    // Credential theft
    "mimikatz",
    "hashcat",
    "john",
    // Privilege escalation attempts
    "sudo su",
    "sudo -i",
    "sudo bash",
    "sudo sh",
    "runas /user:administrator",
    // System modification
    "visudo",
    "passwd root",
    "usermod",
    "useradd",
    "userdel",
    "groupadd",
    "groupdel",
    // Registry attacks (Windows)
    "reg delete HKLM",
    "reg delete HKCU",
    // Process injection
    "ptrace",
    "gdb -p",
    "lldb -p",
    // Crypto mining indicators
    "xmrig",
    "minerd",
    "cgminer",
    // Reverse shells
    "nc -e",
    "ncat -e",
    "bash -i >& /dev/tcp",
    "python -c 'import socket",
    "perl -e 'use Socket",
    "ruby -rsocket",
    "php -r '$sock=fsockopen",
];

/// Command prefixes/patterns that are suspicious but context-dependent
const SUSPICIOUS_PATTERNS: &[&str] = &[
    "curl | sh",
    "curl | bash",
    "wget | sh",
    "wget | bash",
    "eval $(",
    "base64 -d",
    "> /dev/sd",
    "> /dev/nvme",
    "echo '' > /etc/",
    "cat > /etc/",
    "tee /etc/",
    "> /etc/passwd",
    "> /etc/shadow",
    "shutdown",
    "reboot",
    "halt",
    "poweroff",
    "init 0",
    "init 6",
    "systemctl stop",
    "systemctl disable",
    "service stop",
    "launchctl unload",
    "Stop-Service",
    "Set-ExecutionPolicy Unrestricted",
];

/// Maximum command output size (1MB)
const MAX_OUTPUT_SIZE: usize = 1024 * 1024;

/// Default timeout in seconds
const DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Maximum allowed timeout in seconds (10 minutes)
const MAX_TIMEOUT_SECS: u64 = 600;

// ============================================================================
// SHELL TOOL
// ============================================================================

/// Tool for executing shell commands with security guardrails
///
/// Platform behavior:
/// - macOS/Linux: Uses zsh if available, falls back to bash, then sh
/// - Windows: Uses PowerShell, with WSL bash as fallback option
///
/// Security features:
/// - Blocks dangerous commands that could harm the system
/// - Warns on suspicious patterns
/// - Disabled when running as root/administrator
/// - Configurable timeout to prevent runaway processes
pub struct ShellTool {
    /// Whether the tool is disabled due to elevated privileges
    disabled: bool,
    /// Reason for being disabled
    disabled_reason: Option<String>,
}

impl ShellTool {
    /// Create a new Shell tool
    ///
    /// Automatically detects if running with elevated privileges and disables
    /// the tool if so for security reasons.
    #[must_use]
    pub fn new() -> Self {
        let (disabled, disabled_reason) = Self::check_elevated_privileges();
        Self {
            disabled,
            disabled_reason,
        }
    }

    /// Check if running with elevated privileges (root/sudo/administrator)
    fn check_elevated_privileges() -> (bool, Option<String>) {
        #[cfg(unix)]
        {
            use std::env;

            // Check if running as root (UID 0)
            if unsafe { libc::getuid() } == 0 {
                return (
                    true,
                    Some("Shell tool is disabled when running as root".to_string()),
                );
            }

            // Check if running under sudo
            if env::var("SUDO_USER").is_ok() {
                return (
                    true,
                    Some("Shell tool is disabled when running under sudo".to_string()),
                );
            }
        }

        #[cfg(windows)]
        {
            // Check if running as administrator on Windows
            if is_windows_admin() {
                return (
                    true,
                    Some("Shell tool is disabled when running as Administrator".to_string()),
                );
            }
        }

        (false, None)
    }

    /// Commands that bypass security validation entirely.
    /// These are safe commands that frequently trigger false positives due to
    /// substring matching on their content (e.g., python scripts containing "rm",
    /// cat heredocs with backticks).
    const ALLOWED_PREFIXES: &'static [&'static str] =
        &["python ", "python3 ", "python.exe ", "python3.exe "];

    /// Validate a command against security rules
    fn validate_command(command: &str) -> Result<()> {
        let command_lower = command.to_lowercase();
        let command_normalized = command_lower.replace("  ", " ").trim().to_string();

        // Block file-writing shell commands — use write_file / edit_file instead.
        // Checked before the allowlist so that e.g. `cat > file` is caught
        // even though `cat ` would normally bypass validation.
        if is_file_writing_command(&command_normalized) {
            return Err(AgentError::Tool(
                "Use the write_file tool to create files and edit_file for targeted \
                 edits. Do not write files through the shell."
                    .to_string(),
            ));
        }

        // Block privilege escalation. Checked before the allowlist so that
        // wrapper invocations from inside a python interpreter that shell
        // out are still caught. Privilege escalation hangs on an
        // interactive password prompt the agent cannot answer, so we
        // reject loudly with a recovery hint instead of stalling.
        if let Some(token) = finds_privilege_escalation(&command_normalized) {
            return Err(AgentError::Tool(privilege_escalation_message(token)));
        }

        // Allowlist: commands that bypass validation to avoid false positives
        for prefix in Self::ALLOWED_PREFIXES {
            if command_normalized.starts_with(prefix) {
                return Ok(());
            }
        }

        // Check against blocked commands
        for blocked in BLOCKED_COMMANDS {
            if command_normalized.contains(&blocked.to_lowercase()) {
                return Err(AgentError::Tool(format!(
                    "Command blocked for security: contains forbidden pattern '{}'",
                    blocked
                )));
            }
        }

        // Check for suspicious patterns (warn but allow)
        for pattern in SUSPICIOUS_PATTERNS {
            if command_normalized.contains(&pattern.to_lowercase()) {
                tracing::warn!(
                    "Shell command contains suspicious pattern '{}': {}",
                    pattern,
                    command
                );
            }
        }

        // Additional validation
        // Block commands that try to escape quotes or inject
        if command.contains("$(") && command.contains("rm ") {
            return Err(AgentError::Tool(
                "Command blocked: potential command injection with rm".to_string(),
            ));
        }

        // Block backtick command substitution with dangerous commands
        if command.contains('`')
            && (command.contains("rm ") || command.contains("dd if=") || command.contains("dd of="))
        {
            return Err(AgentError::Tool(
                "Command blocked: potential command injection".to_string(),
            ));
        }

        Ok(())
    }

    /// Get the appropriate shell for the current platform
    #[cfg(unix)]
    fn get_shell() -> (String, Vec<String>) {
        use std::path::Path;

        // Try zsh first (more common on macOS)
        if Path::new("/bin/zsh").exists() {
            return ("zsh".to_string(), vec!["-c".to_string()]);
        }

        // Fall back to bash
        if Path::new("/bin/bash").exists() {
            return ("bash".to_string(), vec!["-c".to_string()]);
        }

        // Last resort: sh
        ("sh".to_string(), vec!["-c".to_string()])
    }

    /// Get the appropriate shell for Windows
    #[cfg(windows)]
    fn get_shell() -> (String, Vec<String>) {
        // Use PowerShell on Windows
        ("powershell".to_string(), vec!["-Command".to_string()])
    }

    /// Truncate output if too large
    fn truncate_output(output: String, max_size: usize) -> (String, bool) {
        if output.len() > max_size {
            let truncated = output.chars().take(max_size).collect::<String>();
            (
                format!(
                    "{}\n\n[Output truncated - exceeded {} bytes]",
                    truncated, max_size
                ),
                true,
            )
        } else {
            (output, false)
        }
    }
}

impl Default for ShellTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Execute shell commands. For creating or editing files, use `write_file` or `edit_file` — not shell heredocs."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "timeout_seconds": {
                    "type": "integer",
                    "description": "Maximum execution time in seconds (default: 60, max: 600)",
                    "minimum": 1,
                    "maximum": 600
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory for the command (optional)"
                },
                "use_wsl": {
                    "type": "boolean",
                    "description": "On Windows, use WSL bash instead of PowerShell (default: false)"
                }
            },
            "required": ["command"]
        }))
    }

    fn permissions(&self) -> ToolPermissions {
        ToolPermissions::dangerous(vec!["shell:execute".into()])
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        // Check for error markers from truncated/malformed tool calls.
        // Return a result (not error) with recovery guidance so the agent can retry.
        if let Some(error_type) = args.get("__error__").and_then(|v| v.as_str()) {
            let original_len = args
                .get("__original_length__")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let guidance = if error_type == "TRUNCATED_ARGUMENTS" {
                format!(
                    "Your shell command was too large ({} bytes) and was truncated. \
                     To fix this:\n\
                     1. SIMPLIFY: Write shorter, simpler code. Avoid verbose formatting.\n\
                     2. For file creation/editing: use `write_file` / `edit_file` one file per call, keep files under 200 lines.\n\
                     3. Do NOT attempt to assemble a large file from many small chunks — this is fragile and error-prone.\n\
                     4. Keep each shell call under 12,000 bytes of arguments.",
                    original_len
                )
            } else {
                let message = args
                    .get("__message__")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                format!("Shell command could not be parsed: {}", message)
            };
            return Ok(json!({
                "success": false,
                "exit_code": -1,
                "stdout": "",
                "stderr": guidance,
                "truncated": false,
                "shell": "none (arguments were truncated before execution)",
            }));
        }

        // Check if tool is disabled due to elevated privileges
        if self.disabled {
            return Err(AgentError::Tool(
                self.disabled_reason
                    .clone()
                    .unwrap_or_else(|| "Shell tool is disabled".to_string()),
            ));
        }

        // Extract parameters
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AgentError::Tool(
                    "Missing 'command' parameter. Expected shape: {\"command\":\"pwd && ls -la\", \"timeout_seconds\":10}".to_string(),
                )
            })?;

        let timeout_seconds = args
            .get("timeout_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .min(MAX_TIMEOUT_SECS);

        let cwd = args.get("cwd").and_then(|v| v.as_str());

        #[cfg(windows)]
        let use_wsl = args
            .get("use_wsl")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Validate command against security rules
        Self::validate_command(command)?;

        tracing::debug!(
            "Shell: executing command ({} chars) with {}s timeout",
            command.len(),
            timeout_seconds
        );

        // Get shell and arguments
        #[cfg(unix)]
        let (shell, shell_args) = Self::get_shell();

        #[cfg(windows)]
        let (shell, shell_args) = if use_wsl {
            (
                "wsl".to_string(),
                vec!["bash".to_string(), "-c".to_string()],
            )
        } else {
            Self::get_shell()
        };

        // Build the command
        let mut cmd = Command::new(&shell);
        cmd.stdin(std::process::Stdio::null());

        // Add shell args
        for arg in &shell_args {
            cmd.arg(arg);
        }
        cmd.arg(command);

        // Set sandboxed Python virtual environment and Node.js modules
        // Use ~/Documents/zbot (matching gateway data_dir resolution)
        if let Some(doc_dir) = dirs::document_dir().or_else(dirs::home_dir) {
            let zbot_dir = doc_dir.join("zbot");
            let wards_dir = zbot_dir.join("wards");

            // === Python Virtual Environment (shared across all wards) ===
            let venv_path = wards_dir.join(".venv");

            // Set VIRTUAL_ENV to activate the venv
            cmd.env("VIRTUAL_ENV", &venv_path);

            // Get the venv bin/Scripts directory
            #[cfg(windows)]
            let venv_bin = venv_path.join("Scripts");
            #[cfg(not(windows))]
            let venv_bin = venv_path.join("bin");

            // === Shared Node.js environment (shared across all wards) ===
            let node_env_dir = wards_dir.join(".node_env");
            let node_modules = node_env_dir.join("node_modules");

            // Build PATH with venv bin and optional node_modules/.bin
            let mut path_parts: Vec<String> = vec![venv_bin.display().to_string()];

            // Always set NODE_PATH so `npm install` targets the shared location
            cmd.env("NODE_PATH", node_modules.display().to_string());

            // Add .bin to PATH for executables if it exists
            let node_bin = node_modules.join(".bin");
            if node_bin.exists() {
                path_parts.push(node_bin.display().to_string());
            }

            tracing::debug!("Shell: NODE_PATH set to {}", node_modules.display());

            // Build PATH: venv/bin + node_modules/.bin + original PATH
            if let Ok(current_path) = std::env::var("PATH") {
                #[cfg(windows)]
                let new_path = format!("{};{}", path_parts.join(";"), current_path);
                #[cfg(not(windows))]
                let new_path = format!("{}:{}", path_parts.join(":"), current_path);
                cmd.env("PATH", new_path);
            }

            // Unset PYTHONHOME to avoid conflicts with system Python
            cmd.env_remove("PYTHONHOME");
        }

        // Set working directory: explicit cwd > ward dir > scratch ward > none
        if let Some(dir) = cwd {
            // Validate cwd doesn't contain path traversal
            if dir.contains("..") {
                return Err(AgentError::Tool(
                    "Working directory cannot contain '..' for security".to_string(),
                ));
            }
            cmd.current_dir(dir);
        } else if let Some(doc_dir) = dirs::document_dir().or_else(dirs::home_dir) {
            let wards_dir = doc_dir.join("zbot").join("wards");

            // Use ward_id if set, otherwise fall back to "scratch"
            let ward_id = ctx
                .get_state("ward_id")
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "scratch".to_string());

            let ward_dir = wards_dir.join(&ward_id);
            if !ward_dir.exists()
                && let Err(e) = std::fs::create_dir_all(&ward_dir)
            {
                tracing::warn!("Failed to create ward dir {}: {}", ward_dir.display(), e);
            }
            if ward_dir.exists() {
                tracing::debug!(
                    "Shell: cwd set to {} (ward: {})",
                    ward_dir.display(),
                    ward_id
                );
                cmd.current_dir(&ward_dir);
            }
        }

        // Execute with timeout
        let result = timeout(Duration::from_secs(timeout_seconds), cmd.output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout_raw = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr_raw = String::from_utf8_lossy(&output.stderr).to_string();

                let (stdout, stdout_truncated) = Self::truncate_output(stdout_raw, MAX_OUTPUT_SIZE);
                let (stderr, stderr_truncated) = Self::truncate_output(stderr_raw, MAX_OUTPUT_SIZE);

                let exit_code = output.status.code().unwrap_or(-1);
                let success = output.status.success();

                tracing::debug!(
                    "Shell: command completed with exit code {} (stdout: {} bytes, stderr: {} bytes)",
                    exit_code,
                    stdout.len(),
                    stderr.len()
                );

                Ok(json!({
                    "success": success,
                    "exit_code": exit_code,
                    "stdout": stdout,
                    "stderr": stderr,
                    "truncated": stdout_truncated || stderr_truncated,
                    "shell": shell,
                }))
            }
            Ok(Err(e)) => Err(AgentError::Tool(format!(
                "Failed to execute command: {}",
                e
            ))),
            Err(_) => Err(AgentError::Tool(format!(
                "Command timed out after {} seconds",
                timeout_seconds
            ))),
        }
    }
}

// ============================================================================
// FILE-WRITING DETECTION
// ============================================================================

/// Privilege-escalation tokens we refuse at command position. Lowercase
/// to match `command_normalized`. Order doesn't matter — we compare for
/// exact equality after tokenization, so longer tokens never accidentally
/// shadow shorter ones.
const PRIVILEGE_ESCALATION_TOKENS: &[&str] = &["sudo", "su", "pkexec", "doas"];

/// Return the first privilege-escalation token that appears at a shell
/// command position, or `None` if none do.
///
/// Command positions are: the very first token of the line, or the first
/// token after a separator that begins a new command:
/// `;`, `&&`, `||`, `|`, backtick, `$(`, or a bare `(` subshell open.
///
/// Tokenizes by splitting the command on those separators and taking
/// the first whitespace-separated token of each segment. Comparison is
/// **exact equality** — `sudoku.py`, `pseudo`, `subprocess`, and
/// `submitting` all stay clear of the list. The token has any trailing
/// `)` (from `$(sudo)` style invocations) trimmed before comparison.
fn finds_privilege_escalation(command_normalized: &str) -> Option<&'static str> {
    use std::sync::OnceLock;
    // One regex matches every shell separator that begins a new
    // command. Compiled once, reused across calls.
    static SEPARATOR_RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = SEPARATOR_RE
        .get_or_init(|| regex::Regex::new(r"&&|\|\||[;|`(]|\$\(").expect("static regex"));

    for segment in re.split(command_normalized) {
        let Some(raw) = segment.split_whitespace().next() else {
            continue;
        };
        // Trim trailing `)` so `cmd $(sudo)` is caught (token before trim
        // would be `sudo)`).
        let token = raw.trim_end_matches(')');
        for &t in PRIVILEGE_ESCALATION_TOKENS {
            if token == t {
                return Some(t);
            }
        }
    }
    None
}

/// Build the agent-readable error returned when a privilege-escalation
/// token is detected. Names concrete recovery paths so the agent
/// self-corrects without user intervention.
fn privilege_escalation_message(token: &str) -> String {
    format!(
        "This shell tool refuses `{token}`. Privilege escalation isn't \
         available in this environment — the call would hang on a \
         password prompt. Pick a non-privileged path: install into a \
         user-writable location (`pip install --user`, `npm install \
         --prefix`, `~/.local`, a venv), use a user-level service \
         (`systemctl --user`), or ask the user to perform the \
         privileged step themselves."
    )
}

/// Detect shell commands that write files — these should use `write_file` / `edit_file` instead.
fn is_file_writing_command(command: &str) -> bool {
    let cmd = command.to_lowercase();

    // PowerShell file-writing cmdlets
    if cmd.contains("set-content") || cmd.contains("out-file") || cmd.contains("add-content") {
        return true;
    }

    // PowerShell here-string to file: @" ... "@ or @' ... '@
    // These create multi-line content and pipe to file.
    if cmd.contains("@\"") || cmd.contains("@'") {
        return true;
    }

    // Unix file-writing: cat > file, echo > file (but NOT cat file or echo text)
    // Direct redirects (cat > file, echo > file)
    if cmd.contains("cat >") || cmd.contains("echo >") || cmd.contains("printf >") {
        return true;
    }

    // echo/printf with redirect anywhere: echo 'text' > file.txt
    if (cmd.starts_with("echo ") || cmd.starts_with("printf ")) && cmd.contains(" > ") {
        return true;
    }

    // Heredoc: << 'EOF' or << EOF — block unless it's python stdin (not file writing).
    if (cmd.contains("<< '") || cmd.contains("<<'") || cmd.contains("<< \""))
        && !cmd.starts_with("python")
    {
        return true;
    }

    // Python file writing via -c: python -c "open('file', 'w').write(...)"
    // Too broad to detect reliably; rely on the write_file / edit_file guidance
    // in the shell tool description.

    false
}

// ============================================================================
// WINDOWS ADMIN CHECK
// ============================================================================

#[cfg(windows)]
fn is_windows_admin() -> bool {
    use std::mem;
    use std::ptr;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token: HANDLE = ptr::null_mut() as HANDLE;

        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }

        let mut elevation: TOKEN_ELEVATION = mem::zeroed();
        let mut size: u32 = mem::size_of::<TOKEN_ELEVATION>() as u32;

        let result = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            size,
            &mut size,
        );

        CloseHandle(token);

        result != 0 && elevation.TokenIsElevated != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blocked_commands() {
        assert!(ShellTool::validate_command("rm -rf /").is_err());
        assert!(ShellTool::validate_command("mkfs.ext4 /dev/sda1").is_err());
        assert!(ShellTool::validate_command("dd if=/dev/zero of=/dev/sda").is_err());
        assert!(ShellTool::validate_command("dd of=/dev/sda").is_err());
        assert!(ShellTool::validate_command("format c:").is_err());
        assert!(ShellTool::validate_command("sudo su").is_err());
    }

    // -----------------------------------------------------------------
    // Privilege escalation guard
    // Token-position match — `sudoku.py` must pass, `sudo apt` must fail.
    // -----------------------------------------------------------------

    #[test]
    fn priv_sudo_at_command_start_rejected() {
        let err = ShellTool::validate_command("sudo apt install foo")
            .expect_err("sudo at start must be rejected");
        let msg = format!("{err}");
        assert!(msg.contains("sudo"), "got: {msg}");
    }

    #[test]
    fn priv_su_dash_rejected() {
        assert!(ShellTool::validate_command("su -").is_err());
        assert!(ShellTool::validate_command("su -c 'whoami'").is_err());
        assert!(ShellTool::validate_command("su username").is_err());
    }

    #[test]
    fn priv_sudo_after_pipe_rejected() {
        assert!(ShellTool::validate_command("echo y | sudo apt install foo").is_err());
    }

    #[test]
    fn priv_sudo_after_and_rejected() {
        assert!(ShellTool::validate_command("cd /tmp && sudo systemctl restart bar").is_err());
    }

    #[test]
    fn priv_sudo_after_or_rejected() {
        assert!(ShellTool::validate_command("test -d /etc || sudo mkdir /etc/foo").is_err());
    }

    #[test]
    fn priv_sudo_after_semicolon_rejected() {
        assert!(ShellTool::validate_command("date; sudo whoami").is_err());
    }

    #[test]
    fn priv_sudo_in_command_substitution_rejected() {
        assert!(ShellTool::validate_command("echo $(sudo whoami)").is_err());
        assert!(ShellTool::validate_command("echo `sudo whoami`").is_err());
    }

    #[test]
    fn priv_sudo_in_subshell_rejected() {
        assert!(ShellTool::validate_command("(sudo apt update)").is_err());
    }

    #[test]
    fn priv_pkexec_rejected() {
        assert!(ShellTool::validate_command("pkexec systemctl restart foo").is_err());
    }

    #[test]
    fn priv_doas_rejected() {
        assert!(ShellTool::validate_command("doas pkg_add ripgrep").is_err());
    }

    #[test]
    fn priv_sudoku_filename_allowed() {
        // Substring would have caught this; token-position must not.
        assert!(ShellTool::validate_command("python sudoku.py").is_ok());
    }

    #[test]
    fn priv_pseudo_token_allowed() {
        // `pseudo` is not `sudo`; `subprocess` / `submit` / `subset` are not `su`.
        assert!(ShellTool::validate_command("pseudo --help").is_ok());
        assert!(ShellTool::validate_command("subprocess foo").is_ok());
        assert!(ShellTool::validate_command("submit job").is_ok());
        assert!(ShellTool::validate_command("subset data.csv").is_ok());
    }

    #[test]
    fn priv_sudo_inside_string_arg_allowed() {
        // The token after `echo` is `"use`, not `sudo` — passes.
        assert!(ShellTool::validate_command("echo \"use sudo for X\"").is_ok());
    }

    #[test]
    fn priv_error_message_lists_recovery_paths() {
        let err = ShellTool::validate_command("sudo apt install foo").expect_err("must error");
        let msg = format!("{err}");
        // Agent must see at least one concrete alternative so it
        // self-corrects without nudging.
        assert!(
            msg.contains("--user")
                || msg.contains("--prefix")
                || msg.contains("~/.local")
                || msg.contains("venv"),
            "error must name at least one recovery path; got: {msg}"
        );
    }

    #[test]
    fn priv_finds_returns_token_for_diagnostics() {
        // White-box test on the helper so the matched token is what we
        // claim it is — used by the error message.
        assert_eq!(finds_privilege_escalation("sudo whoami"), Some("sudo"));
        assert_eq!(finds_privilege_escalation("su -"), Some("su"));
        assert_eq!(finds_privilege_escalation("pkexec foo"), Some("pkexec"));
        assert_eq!(finds_privilege_escalation("doas foo"), Some("doas"));
        assert_eq!(finds_privilege_escalation("ls -la"), None);
        assert_eq!(finds_privilege_escalation("python sudoku.py"), None);
    }

    #[test]
    fn test_allowed_commands() {
        assert!(ShellTool::validate_command("ls -la").is_ok());
        assert!(ShellTool::validate_command("echo hello").is_ok());
        assert!(ShellTool::validate_command("pwd").is_ok());
        assert!(ShellTool::validate_command("cat file.txt").is_ok());
        assert!(ShellTool::validate_command("git status").is_ok());
        // "dd" as substring should not be blocked
        assert!(ShellTool::validate_command("git add .").is_ok());
        assert!(ShellTool::validate_command("npm add express").is_ok());
    }

    #[test]
    fn missing_command_error_includes_expected_shape() {
        let msg = "Missing 'command' parameter. Expected shape: {\"command\":\"pwd && ls -la\", \"timeout_seconds\":10}";
        assert!(msg.contains("\"command\""));
        assert!(msg.contains("timeout_seconds"));
    }

    #[test]
    fn test_allowlisted_commands_bypass_validation() {
        // python/python3 with content that would normally trigger backtick+rm injection check
        assert!(ShellTool::validate_command("python script.py").is_ok());
        assert!(ShellTool::validate_command("python3 -c 'import os; os.remove(\"file\")'").is_ok());
        assert!(ShellTool::validate_command("python -c 'x = `cmd`; rm something'").is_ok());
        assert!(ShellTool::validate_command("python3 run.py --flag").is_ok());

        // cat reading is fine (no longer in ALLOWED_PREFIXES, but doesn't trigger any rules)
        assert!(ShellTool::validate_command("cat file.txt").is_ok());

        // cat writing is blocked — use write_file / edit_file instead
        assert!(ShellTool::validate_command("cat > file.py << 'EOF'\nrm -rf /\nEOF").is_err());
    }

    #[test]
    fn test_output_truncation() {
        let short = "hello".to_string();
        let (result, truncated) = ShellTool::truncate_output(short, 100);
        assert!(!truncated);
        assert_eq!(result, "hello");

        let long = "a".repeat(200);
        let (result, truncated) = ShellTool::truncate_output(long, 100);
        assert!(truncated);
        assert!(result.contains("[Output truncated"));
    }

    #[test]
    fn test_file_writing_commands_blocked() {
        // PowerShell
        assert!(ShellTool::validate_command("Set-Content -Path 'file.py' -Value 'code'").is_err());
        assert!(ShellTool::validate_command("'hello' | Out-File test.txt").is_err());
        assert!(ShellTool::validate_command("Add-Content -Path log.txt -Value 'line'").is_err());

        // PowerShell here-strings
        assert!(ShellTool::validate_command("@\"\ncode\n\"@ | Set-Content file.py").is_err());

        // Unix redirects
        assert!(ShellTool::validate_command("cat > file.py << 'EOF'").is_err());
        assert!(ShellTool::validate_command("echo 'hello' > output.txt").is_err());

        // Python heredocs are allowed (stdin, not file writing).
        assert!(ShellTool::validate_command("python << 'EOF'\nprint('hi')\nEOF").is_ok());
    }

    #[test]
    fn test_reading_commands_not_blocked() {
        // Reading commands should NOT be blocked
        assert!(ShellTool::validate_command("Get-Content file.py").is_ok());
        assert!(ShellTool::validate_command("cat file.py").is_ok());
        assert!(ShellTool::validate_command("python script.py").is_ok());
        assert!(ShellTool::validate_command("python -c \"print('hello')\"").is_ok());
    }
}
