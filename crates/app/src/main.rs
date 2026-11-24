use std::sync::Arc;

use anyhow::Result;
use render::{FontConfig, Frame, Renderer, Rgba};
use terminal_infra::{Pty, PtyEvent, PtyGrid, ShellCommand};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

const BACKGROUND: Rgba = Rgba {
    r: 0.04,
    g: 0.05,
    b: 0.07,
    a: 1.0,
};
const FOREGROUND: Rgba = Rgba {
    r: 0.85,
    g: 0.87,
    b: 0.9,
    a: 1.0,
};
const ESC: char = '\u{1b}';
const BEL: char = '\u{7}';
const CURSOR_POSITION_REQUEST: &[u8] = b"\x1b[6n";
const CURSOR_POSITION_REPORT: &[u8] = b"\x1b[1;1R";

#[derive(Debug)]
enum AppEvent {
    Pty(PtyEvent),
}

struct App {
    proxy: EventLoopProxy<AppEvent>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    pty: Option<Pty>,
    output: Vec<u8>,
}

impl App {
    fn new(proxy: EventLoopProxy<AppEvent>) -> Self {
        Self {
            proxy,
            window: None,
            renderer: None,
            pty: None,
            output: Vec::new(),
        }
    }

    fn grid(&self) -> PtyGrid {
        let (cols, rows) = self.renderer.as_ref().map_or((80, 24), Renderer::grid_size);
        PtyGrid {
            cols: u16::try_from(cols).unwrap_or(u16::MAX),
            rows: u16::try_from(rows).unwrap_or(u16::MAX),
        }
    }

    fn spawn_shell(&mut self) {
        let proxy = self.proxy.clone();
        let command = ShellCommand::default_shell();
        log::info!("shell: {}", command.program);
        match Pty::spawn(command, self.grid(), move |event| {
            let _ = proxy.send_event(AppEvent::Pty(event));
        }) {
            Ok(pty) => self.pty = Some(pty),
            Err(e) => log::error!("spawn shell: {e:#}"),
        }
    }

    // Temporary view until the VT emulator lands (F0-5).
    fn screen_text(&self) -> String {
        let rows = self.grid().rows as usize;
        let text = strip_escapes(&String::from_utf8_lossy(&self.output));
        let lines: Vec<&str> = text.lines().collect();
        lines[lines.len().saturating_sub(rows)..].join("\n")
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title("Argus");
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("create window: {e}");
                event_loop.exit();
                return;
            }
        };
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        let init = Renderer::new(
            window.clone(),
            size.width,
            size.height,
            scale,
            FontConfig::default(),
        );
        match pollster::block_on(init) {
            Ok(renderer) => self.renderer = Some(renderer),
            Err(e) => {
                log::error!("init renderer: {e:#}");
                event_loop.exit();
                return;
            }
        }
        window.request_redraw();
        self.window = Some(window);
        self.spawn_shell();
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Pty(PtyEvent::Output(bytes)) => {
                if bytes
                    .windows(CURSOR_POSITION_REQUEST.len())
                    .any(|w| w == CURSOR_POSITION_REQUEST)
                    && let Some(pty) = &mut self.pty
                {
                    let _ = pty.write(CURSOR_POSITION_REPORT);
                }
                self.output.extend(bytes);
                self.request_redraw();
            }
            AppEvent::Pty(PtyEvent::Exited(code)) => {
                log::info!("shell exited: {code:?}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                let grid = self.grid();
                if let Some(pty) = &self.pty
                    && let Err(e) = pty.resize(grid)
                {
                    log::warn!("resize pty: {e:#}");
                }
                self.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.set_scale(scale_factor as f32);
                }
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let text = self.screen_text();
                if let Some(renderer) = &mut self.renderer {
                    let frame = Frame {
                        background: BACKGROUND,
                        foreground: FOREGROUND,
                        text: &text,
                    };
                    if let Err(e) = renderer.render(frame) {
                        log::error!("render: {e:#}");
                        event_loop.exit();
                    }
                }
            }
            _ => {}
        }
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(event_loop.create_proxy());
    event_loop.run_app(&mut app)?;
    Ok(())
}

fn strip_escapes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ESC => match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == BEL || (c == ESC && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\n' => out.push('\n'),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}
