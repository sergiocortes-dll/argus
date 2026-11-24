use std::sync::Arc;

use anyhow::Result;
use render::{FontConfig, Frame, Renderer, Rgba};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
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

#[derive(Default)]
struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
}

impl App {
    fn demo_text(&self) -> String {
        let Some(renderer) = &self.renderer else {
            return String::new();
        };
        let (cols, rows) = renderer.grid_size();
        let cell = renderer.cell_size();
        let ruler: String = (0..cols)
            .map(|i| char::from(b'0' + (i % 10) as u8))
            .collect();
        format!(
            "Argus — grid {cols}x{rows}, cell {:.1}x{:.1}px\n\
             {ruler}\n\
             ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789\n\
             áéíóú ñÑ ¿? ¡! → ← ✓ ✗ λ\n\
             ┌──────┬──────┐\n\
             │ box  │ draw │\n\
             └──────┴──────┘\n\
             ilIl1| WWWWWW mmmmmm ......",
            cell.width, cell.height
        )
    }
}

impl ApplicationHandler for App {
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
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.set_scale(scale_factor as f32);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                let text = self.demo_text();
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
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut App::default())?;
    Ok(())
}
