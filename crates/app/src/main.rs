use std::sync::Arc;

use anyhow::Result;
use render::{FontConfig, GridCell, GridFrame, Renderer};
use terminal_infra::{
    Emulator, Key, Mods, NamedKey, Pty, PtyEvent, PtyGrid, ScreenCell, ShellCommand, encode_key,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key as WinitKey, NamedKey as WinitNamedKey};
use winit::window::{Window, WindowId};

#[derive(Debug)]
enum AppEvent {
    Pty(PtyEvent),
}

struct App {
    proxy: EventLoopProxy<AppEvent>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    pty: Option<Pty>,
    emulator: Emulator,
    cells: Vec<GridCell>,
    mods: Mods,
}

impl App {
    fn new(proxy: EventLoopProxy<AppEvent>) -> Self {
        Self {
            proxy,
            window: None,
            renderer: None,
            pty: None,
            emulator: Emulator::new(80, 24),
            cells: Vec::new(),
            mods: Mods::default(),
        }
    }

    fn on_key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let key = match &event.logical_key {
            WinitKey::Named(named) => match map_named(*named) {
                Some(named) => Key::Named(named),
                None => return,
            },
            WinitKey::Character(base) => Key::Char {
                base: base.as_str(),
                text: event.text.as_deref(),
            },
            _ => return,
        };
        let Some(bytes) = encode_key(&key, self.mods, self.emulator.input_modes()) else {
            return;
        };
        if let Some(pty) = &mut self.pty
            && let Err(e) = pty.write(&bytes)
        {
            log::warn!("write key to pty: {e:#}");
        }
    }

    fn grid(&self) -> (usize, usize) {
        self.renderer.as_ref().map_or((80, 24), Renderer::grid_size)
    }

    fn spawn_shell(&mut self) {
        let proxy = self.proxy.clone();
        let command = ShellCommand::default_shell();
        log::info!("shell: {}", command.program);
        match Pty::spawn(command, pty_grid(self.grid()), move |event| {
            let _ = proxy.send_event(AppEvent::Pty(event));
        }) {
            Ok(pty) => self.pty = Some(pty),
            Err(e) => log::error!("spawn shell: {e:#}"),
        }
    }

    fn resize_grid(&mut self) {
        let (cols, rows) = self.grid();
        if self.emulator.size() == (cols, rows) {
            return;
        }
        self.emulator.resize(cols, rows);
        if let Some(pty) = &self.pty
            && let Err(e) = pty.resize(pty_grid((cols, rows)))
        {
            log::warn!("resize pty: {e:#}");
        }
    }

    fn on_output(&mut self, bytes: &[u8]) {
        self.emulator.feed(bytes);
        let replies = self.emulator.take_replies();
        if !replies.is_empty()
            && let Some(pty) = &mut self.pty
            && let Err(e) = pty.write(&replies)
        {
            log::warn!("reply to pty: {e:#}");
        }
        if let Some(title) = self.emulator.take_title()
            && let Some(window) = &self.window
        {
            window.set_title(&format!("{title} — Argus"));
        }
        self.request_redraw();
    }

    fn draw(&mut self) -> Result<()> {
        let screen = self.emulator.screen();
        self.cells.clear();
        self.cells.extend(screen.cells.iter().map(to_grid_cell));
        let Some(renderer) = &mut self.renderer else {
            return Ok(());
        };
        renderer.render(GridFrame {
            cols: screen.cols,
            background: screen.background,
            cells: &self.cells,
        })
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
        self.window = Some(window);
        self.resize_grid();
        self.spawn_shell();
        self.request_redraw();
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Pty(PtyEvent::Output(bytes)) => self.on_output(&bytes),
            AppEvent::Pty(PtyEvent::Exited(code)) => {
                log::info!("shell exited: {code:?}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::ModifiersChanged(modifiers) => {
                let state = modifiers.state();
                self.mods = Mods {
                    shift: state.shift_key(),
                    ctrl: state.control_key(),
                    alt: state.alt_key(),
                };
            }
            WindowEvent::KeyboardInput { event, .. } => self.on_key(&event),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                self.resize_grid();
                self.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.set_scale(scale_factor as f32);
                }
                self.resize_grid();
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.draw() {
                    log::error!("render: {e:#}");
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }
}

fn pty_grid((cols, rows): (usize, usize)) -> PtyGrid {
    PtyGrid {
        cols: u16::try_from(cols).unwrap_or(u16::MAX),
        rows: u16::try_from(rows).unwrap_or(u16::MAX),
    }
}

fn map_named(key: WinitNamedKey) -> Option<NamedKey> {
    Some(match key {
        WinitNamedKey::Enter => NamedKey::Enter,
        WinitNamedKey::Backspace => NamedKey::Backspace,
        WinitNamedKey::Tab => NamedKey::Tab,
        WinitNamedKey::Escape => NamedKey::Escape,
        WinitNamedKey::Space => NamedKey::Space,
        WinitNamedKey::ArrowUp => NamedKey::Up,
        WinitNamedKey::ArrowDown => NamedKey::Down,
        WinitNamedKey::ArrowLeft => NamedKey::Left,
        WinitNamedKey::ArrowRight => NamedKey::Right,
        WinitNamedKey::Home => NamedKey::Home,
        WinitNamedKey::End => NamedKey::End,
        WinitNamedKey::Insert => NamedKey::Insert,
        WinitNamedKey::Delete => NamedKey::Delete,
        WinitNamedKey::PageUp => NamedKey::PageUp,
        WinitNamedKey::PageDown => NamedKey::PageDown,
        WinitNamedKey::F1 => NamedKey::F(1),
        WinitNamedKey::F2 => NamedKey::F(2),
        WinitNamedKey::F3 => NamedKey::F(3),
        WinitNamedKey::F4 => NamedKey::F(4),
        WinitNamedKey::F5 => NamedKey::F(5),
        WinitNamedKey::F6 => NamedKey::F(6),
        WinitNamedKey::F7 => NamedKey::F(7),
        WinitNamedKey::F8 => NamedKey::F(8),
        WinitNamedKey::F9 => NamedKey::F(9),
        WinitNamedKey::F10 => NamedKey::F(10),
        WinitNamedKey::F11 => NamedKey::F(11),
        WinitNamedKey::F12 => NamedKey::F(12),
        _ => return None,
    })
}

fn to_grid_cell(c: &ScreenCell) -> GridCell {
    GridCell {
        ch: c.ch,
        fg: c.fg,
        bg: c.bg,
        bold: c.bold,
        italic: c.italic,
        underline: c.underline,
        spacer: c.spacer,
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
