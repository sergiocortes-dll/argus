use std::io::{Read, Write};
use std::thread;

use anyhow::{Context, Result};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::ShellCommand;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtyGrid {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug)]
pub enum PtyEvent {
    Output(Vec<u8>),
    Exited(Option<u32>),
}

pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

impl Pty {
    /// `on_event` runs on background threads: one reading output, one waiting for exit.
    pub fn spawn<F>(command: ShellCommand, grid: PtyGrid, on_event: F) -> Result<Self>
    where
        F: Fn(PtyEvent) + Send + Sync + Clone + 'static,
    {
        let pair = native_pty_system()
            .openpty(pty_size(grid))
            .context("open pty")?;

        let mut child = pair
            .slave
            .spawn_command(to_builder(&command))
            .with_context(|| format!("spawn {}", command.program))?;
        drop(pair.slave);

        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader().context("clone pty reader")?;
        let writer = pair.master.take_writer().context("take pty writer")?;

        let on_output = on_event.clone();
        thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => on_output(PtyEvent::Output(buf[..n].to_vec())),
                        Err(e) => {
                            log::debug!("pty read ended: {e}");
                            break;
                        }
                    }
                }
            })
            .context("spawn pty reader thread")?;

        thread::Builder::new()
            .name("pty-waiter".into())
            .spawn(move || {
                let code = child.wait().ok().map(|status| status.exit_code());
                on_event(PtyEvent::Exited(code));
            })
            .context("spawn pty waiter thread")?;

        Ok(Self {
            master: pair.master,
            writer,
            killer,
        })
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes).context("write to pty")?;
        self.writer.flush().context("flush pty")
    }

    pub fn resize(&self, grid: PtyGrid) -> Result<()> {
        self.master.resize(pty_size(grid)).context("resize pty")
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.killer.kill();
    }
}

fn pty_size(grid: PtyGrid) -> PtySize {
    PtySize {
        rows: grid.rows.max(1),
        cols: grid.cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn to_builder(command: &ShellCommand) -> CommandBuilder {
    let mut builder = CommandBuilder::new(&command.program);
    for arg in &command.args {
        builder.arg(arg);
    }
    if let Some(cwd) = &command.cwd {
        builder.cwd(cwd);
    }
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder.env("TERM_PROGRAM", "argus");
    for (key, value) in &command.env {
        builder.env(key, value);
    }
    builder
}
