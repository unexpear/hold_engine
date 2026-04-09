//! Core engine — owns the window, ECS world, system schedule, and renderer.
//! The game loop is driven by a caller-supplied closure.

use crate::ecs::{SystemSchedule, World};
use crate::input::InputState;
use crate::platform::Window;
use crate::renderer::Renderer;
use std::time::Instant;


/// Trait for structured game applications.
///
/// Implement this instead of using a raw closure with `Engine::run`.
///
/// ```rust,no_run
/// use hold::prelude::*;
/// use hold::engine::App;
///
/// struct MyGame;
///
/// impl App for MyGame {
///     fn setup(&mut self, world: &mut World) {
///         // Spawn entities, add components, etc.
///     }
///     fn update(&mut self, world: &mut World, renderer: &mut dyn Renderer, input: &InputState, dt: f32) {
///         // Game logic and rendering each frame.
///     }
/// }
/// ```
pub trait App {
    /// Called once before the first frame.
    fn setup(&mut self, _world: &mut World) {}
    /// Called every frame.
    fn update(&mut self, world: &mut World, renderer: &mut dyn Renderer, input: &InputState, dt: f32);
}


pub struct Engine {
    pub world:    World,
    pub schedule: SystemSchedule,
    pub window:   Window,
    input:        InputState,
    fps_timer:    f32,   // seconds since last title update
}

impl Engine {
    pub fn new(title: &str, width: u32, height: u32) -> Self {
        let window = Window::new(title, width, height);
        Self {
            world:    World::new(),
            schedule: SystemSchedule::new(),
            window,
            input:    InputState::new(),
            fps_timer: 0.0,
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

        let mut last = Instant::now();

        loop {
            // ── Timing ────────────────────────────────────────────────────
            let now = Instant::now();
            let dt = now.duration_since(last).as_secs_f32().min(0.1); // cap at 100 ms to avoid spiral of death
            last = now;

            // Update window title with FPS — throttled to once per second.
            self.fps_timer += dt;
            if self.fps_timer >= 1.0 && dt > 0.0 {
                let fps = (1.0 / dt) as u32;
                self.window.set_title(&format!("Hold — {} FPS", fps));
                self.fps_timer = 0.0;
            }

            // ── Events / Input ────────────────────────────────────────────
            if !self.window.poll_events(&mut self.input) {
                break;
            }

            // Clear events from the previous frame so systems and the frame
            // callback start each iteration with a clean slate.
            self.world.events.clear_all();

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

    /// Run a game loop driven by an [`App`] implementor.
    pub fn run_app<R, A>(mut self, renderer: R, mut app: A)
    where
        R: Renderer,
        A: App,
    {
        app.setup(&mut self.world);
        self.run(renderer, move |world, renderer, input, dt| {
            app.update(world, renderer, input, dt);
        });
    }
}
