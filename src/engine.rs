//! Core engine — owns the window, ECS world, system schedule, and renderer.
//! The game loop is driven by a caller-supplied closure.

use crate::ecs::{SystemSchedule, World};
use crate::input::InputState;
use crate::platform::Window;
use crate::renderer::Renderer;
use crate::platform::ffi::{QueryPerformanceCounter, QueryPerformanceFrequency};

pub struct Engine {
    pub world:    World,
    pub schedule: SystemSchedule,
    pub window:   Window,
    input:        InputState,
}

impl Engine {
    pub fn new(title: &str, width: u32, height: u32) -> Self {
        let window = Window::new(title, width, height);
        Self {
            world:    World::new(),
            schedule: SystemSchedule::new(),
            window,
            input:    InputState::new(),
        }
    }

    /// Run the game loop.
    ///
    /// The closure receives:
    ///   `world`    — ECS world (spawn/query/components)
    ///   `renderer` — backend (clear, draw, present)
    ///   `input`    — keyboard/mouse snapshot
    ///   `dt`       — seconds since last frame (clamped to 0.1 s)
    pub fn run<R, F>(mut self, mut renderer: R, mut frame_fn: F)
    where
        R: Renderer,
        F: FnMut(&mut World, &mut R, &InputState, f32),
    {
        self.window.show();

        let mut freq = 0i64;
        let mut last = 0i64;
        unsafe {
            QueryPerformanceFrequency(&mut freq);
            QueryPerformanceCounter(&mut last);
        }

        loop {
            // ── Timing ────────────────────────────────────────────────────
            let mut now = 0i64;
            unsafe { QueryPerformanceCounter(&mut now); }
            let dt = ((now - last) as f64 / freq as f64) as f32;
            let dt = dt.min(0.1); // cap at 100 ms to avoid spiral of death
            last = now;

            // Update window title with FPS.
            if dt > 0.0 {
                let fps = (1.0 / dt) as u32;
                self.window.set_title(&format!("Forge — {} FPS", fps));
            }

            // ── Events / Input ────────────────────────────────────────────
            if !self.window.poll_events(&mut self.input) {
                break;
            }

            // ── ECS systems ───────────────────────────────────────────────
            self.schedule.run(&mut self.world, dt);

            // ── Render ────────────────────────────────────────────────────
            renderer.begin_frame();
            frame_fn(&mut self.world, &mut renderer, &self.input, dt);
            renderer.end_frame();

            // Sync renderer dimensions with window (handles resize).
            let (ww, wh) = (self.window.width(), self.window.height());
            if ww != renderer.width() || wh != renderer.height() {
                renderer.resize(ww, wh);
            }
        }
    }
}
