use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ShellCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

impl ShellCommand {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// pwsh, then Windows PowerShell on Windows; `$SHELL`, then `/bin/sh` elsewhere.
    pub fn default_shell() -> Self {
        if cfg!(windows) {
            let program = find_in_path("pwsh.exe").map_or_else(
                || "powershell.exe".to_owned(),
                |p| p.to_string_lossy().into_owned(),
            );
            Self::new(program).arg("-NoLogo")
        } else {
            let program = std::env::var("SHELL")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "/bin/sh".to_owned());
            Self::new(program).arg("-l")
        }
    }
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_file(candidate))
}

fn is_file(path: &Path) -> bool {
    path.metadata().is_ok_and(|m| m.is_file())
}
