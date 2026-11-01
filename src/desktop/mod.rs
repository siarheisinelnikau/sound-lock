//! Reactive egui host: CPU rasterization and GDI presentation, without a GPU API.
mod gdi;
use crate::{
    app::{App, Application},
    popup, update,
};
use anyhow::{Context, Result};
use egui::{ViewportCommand, ViewportId};
use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::platform::windows::{WindowAttributesExtWindows, WindowExtWindows};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

#[allow(dead_code)] // Native-window access is used by the opt-in tray runner.
pub struct CreationContext<'a> {
    pub egui_ctx: egui::Context,
    window: &'a Arc<Window>,
}
impl CreationContext<'_> {
    #[cfg(feature = "tray-smoke-test")]
    pub fn winit_window(&self) -> Option<&Arc<Window>> {
        Some(self.window)
    }
}
struct Repaint {
    id: ViewportId,
    when: Instant,
    pass: u64,
}
enum Event {
    Repaint(Repaint),
    Accessibility(accesskit_winit::Event),
}
impl From<accesskit_winit::Event> for Event {
    fn from(event: accesskit_winit::Event) -> Self {
        Self::Accessibility(event)
    }
}
struct Viewport {
    window: Arc<Window>,
    input: egui_winit::State,
    info: egui::ViewportInfo,
    callback: Option<Arc<egui::DeferredViewportUiCallback>>,
    builder: egui::ViewportBuilder,
    next: Option<Instant>,
    force_ui: bool,
    visible: bool,
    focus_after_paint: bool,
    close: bool,
    bitmap: Option<gdi::Bitmap>,
    has_frame: bool,
    scene: Option<(f32, Vec<egui::epaint::ClippedShape>)>,
    last_ui: Option<Instant>,
    frame_interval: Duration,
}
impl Viewport {
    fn request(&mut self, when: Instant) {
        let when = self
            .last_ui
            .map_or(when, |last| when.max(last + self.frame_interval));
        self.next = Some(self.next.map_or(when, |pending| pending.min(when)));
    }
}
struct Host {
    proxy: winit::event_loop::EventLoopProxy<Event>,
    context: egui::Context,
    application: Option<Application>,
    startup: Option<update::Startup>,
    viewports: BTreeMap<ViewportId, Viewport>,
    renderer: EguiSoftwareRender,
    pending: egui::TexturesDelta,
    error: Option<anyhow::Error>,
    next_logic: Option<Instant>,
    started: Instant,
    stop_at: Option<Instant>,
    trace: Trace,
}
impl Host {
    fn create(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: ViewportId,
        builder: egui::ViewportBuilder,
        callback: Option<Arc<egui::DeferredViewportUiCallback>>,
    ) -> Result<()> {
        let visible = builder.visible.unwrap_or(true);
        let attributes = egui_winit::create_winit_window_attributes(&self.context, builder.clone())
            .with_visible(false)
            .with_transparent(false)
            .with_undecorated_shadow(false);
        let window = Arc::new(event_loop.create_window(attributes)?);
        if id == ViewportId::ROOT {
            window.set_taskbar_icon(Some(winit::window::Icon::from_rgba(
                crate::tray::icon_rgba(64),
                64,
                64,
            )?));
        }
        if id == popup::id() {
            gdi::transparent(&window)?;
        }
        let mut input = egui_winit::State::new(
            self.context.clone(),
            id,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            window.theme(),
            None,
        );
        input.init_accesskit(event_loop, &window, self.proxy.clone());
        let mut info = egui::ViewportInfo::default();
        egui_winit::update_viewport_info(&mut info, &self.context, &window, true);
        if id != ViewportId::ROOT {
            info.parent = Some(ViewportId::ROOT);
        }
        let frame_interval = display_interval(&window);
        self.viewports.insert(
            id,
            Viewport {
                window,
                input,
                info,
                callback,
                builder,
                next: Some(Instant::now()),
                force_ui: true,
                visible,
                focus_after_paint: true,
                close: false,
                bitmap: None,
                has_frame: false,
                scene: None,
                last_ui: None,
                frame_interval,
            },
        );
        Ok(())
    }
    fn commands(
        &mut self,
        id: ViewportId,
        commands: impl IntoIterator<Item = ViewportCommand>,
        event_loop: &ActiveEventLoop,
    ) -> Result<()> {
        let Some(state) = self.viewports.get_mut(&id) else {
            return Ok(());
        };
        let mut native = Vec::new();
        for command in commands {
            match command {
                ViewportCommand::Close => {
                    state.close = true;
                    state.info.events.push(egui::ViewportEvent::Close);
                    state.request(Instant::now());
                }
                ViewportCommand::CancelClose => state.close = false,
                ViewportCommand::Visible(visible) => {
                    state.visible = visible;
                    state.builder.visible = Some(visible);
                    if !visible {
                        state.window.set_visible(false);
                    } else {
                        state.request(Instant::now());
                        if state.has_frame {
                            state.window.set_visible(true);
                            if let Some(bitmap) = &state.bitmap {
                                bitmap.present(&state.window, id == popup::id())?;
                            }
                        }
                    }
                }
                ViewportCommand::Focus => {
                    state.focus_after_paint = true;
                    if state.window.is_visible() == Some(true) {
                        state.window.focus_window();
                        state.focus_after_paint = false;
                    }
                }
                ViewportCommand::RequestPaintWhileHidden => {
                    state.force_ui = true;
                    state.request(Instant::now());
                }
                command => native.push(command),
            }
        }
        let mut actions = Vec::new();
        egui_winit::process_viewport_commands(
            &self.context,
            &mut state.info,
            native,
            &state.window,
            &mut actions,
        );
        for action in actions {
            match action {
                egui_winit::ActionRequested::Screenshot(callback) => {
                    // Preserve the screenshot request until this viewport has new pixels.
                    self.context
                        .send_viewport_cmd_to(id, ViewportCommand::Screenshot(callback));
                    state.force_ui = true;
                }
                egui_winit::ActionRequested::PaintWhileHidden => state.force_ui = true,
                egui_winit::ActionRequested::Cut => {
                    state.input.egui_input_mut().events.push(egui::Event::Cut)
                }
                egui_winit::ActionRequested::Copy => {
                    state.input.egui_input_mut().events.push(egui::Event::Copy)
                }
                egui_winit::ActionRequested::Paste => {
                    if let Some(text) = state.input.clipboard_text() {
                        state
                            .input
                            .egui_input_mut()
                            .events
                            .push(egui::Event::Paste(text));
                    }
                }
            }
        }
        if id == popup::id() {
            state.window.set_undecorated_shadow(false);
        }
        if state.close
            && self
                .application
                .as_ref()
                .is_some_and(Application::is_quitting)
        {
            if id == ViewportId::ROOT {
                event_loop.exit();
            } else {
                state.window.set_visible(false);
                state.visible = false;
                state.close = false;
            }
        }
        Ok(())
    }
    fn draw(
        &mut self,
        id: ViewportId,
        event_loop: &ActiveEventLoop,
        logic_only: bool,
    ) -> Result<()> {
        let began = Instant::now();
        let Some(state) = self.viewports.get_mut(&id) else {
            return Ok(());
        };
        if !logic_only {
            state.next = None;
        }
        egui_winit::update_viewport_info(&mut state.info, &self.context, &state.window, false);
        // A background logic tick must not consume clicks/keys waiting for the
        // next UI pass, even when a timer and input arrive together.
        let direct_ui = !logic_only
            && ((state.visible && state.window.is_minimized() != Some(true)) || state.force_ui);
        let mut input = if logic_only {
            state.input.egui_input_mut().clone()
        } else {
            state.input.take_egui_input(&state.window)
        };
        input.time = Some(self.started.elapsed().as_secs_f64());
        for (key, viewport) in &self.viewports {
            input.viewports.insert(*key, viewport.info.clone());
        }
        let mut ui_changed = false;
        let ran_logic = id == ViewportId::ROOT && !direct_ui;
        if ran_logic {
            let output = self.context.run_logic(&input, |ctx| {
                ui_changed = self.application.as_mut().unwrap().logic(ctx)
            });
            self.next_logic = self
                .application
                .as_ref()
                .unwrap()
                .next_poll()
                .and_then(|delay| Instant::now().checked_add(delay));
            self.trace.logic += 1;
            if self.started.elapsed() >= Duration::from_secs(2) {
                self.trace.logic_after_warmup += 1;
            }
            {
                let state = self.viewports.get_mut(&id).unwrap();
                state.input.handle_platform_output_with_event_loop(
                    &state.window,
                    event_loop,
                    output.platform_output,
                );
            }
            for (target, commands) in output.viewport_commands {
                self.commands(target, commands, event_loop)?;
            }
        }
        let state = self.viewports.get_mut(&id).unwrap();
        let force = std::mem::take(&mut state.force_ui);
        let ui_needed =
            !logic_only || ui_changed || self.application.as_ref().unwrap().progress_visible();
        let paint =
            (state.visible && state.window.is_minimized() != Some(true) && ui_needed) || force;
        if !paint {
            return Ok(());
        }
        if logic_only {
            input = state.input.take_egui_input(&state.window);
            input.time = Some(self.started.elapsed().as_secs_f64());
            for (key, viewport) in &self.viewports {
                input.viewports.insert(*key, viewport.info.clone());
            }
        }
        let state = self.viewports.get_mut(&id).unwrap();
        state.last_ui = Some(Instant::now());
        let callback = state.callback.clone();
        for (key, viewport) in &self.viewports {
            input.viewports.insert(*key, viewport.info.clone());
        }
        let observed_close = input
            .viewports
            .get(&id)
            .is_some_and(|info| info.close_requested());
        if self.trace.path.is_some() {
            for event in &input.events {
                let kind = match event {
                    egui::Event::PointerMoved(_) => "pointer_move",
                    egui::Event::MouseMoved(_) => "mouse_move",
                    egui::Event::PointerButton { .. } => "pointer_button",
                    egui::Event::WindowFocused(_) => "focus",
                    egui::Event::Key { .. } => "key",
                    _ => "other",
                };
                *self.trace.input_events.entry(kind).or_default() += 1;
            }
        }
        let output = self.context.run_ui(input, |ui| {
            if let Some(callback) = &callback {
                callback(ui);
            } else {
                if !ran_logic {
                    self.application.as_mut().unwrap().logic(ui.ctx());
                    self.trace.logic += 1;
                    if self.started.elapsed() >= Duration::from_secs(2) {
                        self.trace.logic_after_warmup += 1;
                    }
                }
                self.application.as_mut().unwrap().ui(ui);
            }
        });
        if id == ViewportId::ROOT {
            self.next_logic = self
                .application
                .as_ref()
                .unwrap()
                .next_poll()
                .and_then(|delay| Instant::now().checked_add(delay));
        }
        self.viewports.get_mut(&id).unwrap().info.events.clear();
        let egui::FullOutput {
            platform_output,
            textures_delta,
            shapes,
            pixels_per_point,
            viewport_output,
        } = output;
        self.pending.append(textures_delta);
        {
            let state = self.viewports.get_mut(&id).unwrap();
            state.input.handle_platform_output_with_event_loop(
                &state.window,
                event_loop,
                platform_output,
            );
        }
        let mut screenshots = Vec::new();
        for (target, mut viewport) in viewport_output {
            if !self.viewports.contains_key(&target) && target != ViewportId::ROOT {
                self.create(
                    event_loop,
                    target,
                    viewport.builder.clone(),
                    viewport.viewport_ui_cb.clone(),
                )?;
            }
            if target != ViewportId::ROOT
                && let Some(state) = self.viewports.get_mut(&target)
            {
                let (commands, _) = state.builder.patch(viewport.builder);
                state.callback = viewport.viewport_ui_cb;
                viewport.commands.extend(commands);
            }
            let mut commands = Vec::new();
            for command in viewport.commands {
                match command {
                    ViewportCommand::Screenshot(callback) if target == id => {
                        screenshots.push(callback)
                    }
                    command => commands.push(command),
                }
            }
            self.commands(target, commands, event_loop)?;
            // The repaint callback owns deadlines. Output includes other viewports'
            // stale delays, which must not restart already consumed redraws.
        }
        let state = self.viewports.get_mut(&id).unwrap();
        let size = state.window.inner_size();
        if observed_close && state.close {
            if id == ViewportId::ROOT {
                event_loop.exit();
            } else {
                state.window.set_visible(false);
                state.visible = false;
            }
        }
        if size.width == 0 || size.height == 0 || state.window.is_minimized() == Some(true) {
            return Ok(());
        }
        if !state.visible && screenshots.is_empty() {
            return Ok(());
        }
        if state
            .bitmap
            .as_ref()
            .is_none_or(|b| b.width != size.width || b.height != size.height)
        {
            state.bitmap = Some(gdi::Bitmap::new(size.width, size.height)?);
            state.has_frame = false;
        }
        if state.has_frame
            && self.pending.is_empty()
            && screenshots.is_empty()
            && state.scene.as_ref().is_some_and(|(density, previous)| {
                *density == pixels_per_point && *previous == shapes
            })
        {
            self.trace.unchanged += 1;
            return Ok(());
        }
        state.scene = Some((pixels_per_point, shapes.clone()));
        let bitmap = state.bitmap.as_mut().unwrap();
        bitmap.pixels().fill([0; 4]);
        let primitives = self.context.tessellate(shapes, pixels_per_point);
        self.renderer.render(
            &mut BufferMutRef::new(bitmap.pixels(), size.width as usize, size.height as usize),
            &primitives,
            &self.pending,
            pixels_per_point,
        );
        self.pending.clear();
        if !screenshots.is_empty() {
            let pixels = bitmap
                .pixels()
                .iter()
                .map(|p| egui::Color32::from_rgba_premultiplied(p[2], p[1], p[0], p[3]))
                .collect();
            let image = Arc::new(egui::ColorImage::new(
                [size.width as usize, size.height as usize],
                pixels,
            ));
            for callback in screenshots {
                callback.complete(image.clone());
            }
        }
        if state.visible {
            bitmap.present(&state.window, id == popup::id())?;
            state.has_frame = true;
            if state.window.is_visible() != Some(true) {
                state.window.set_visible(true);
                // Winit's show operation rewrites the extended styles. Restore
                // per-pixel alpha and retained pixels after that operation.
                bitmap.present(&state.window, id == popup::id())?;
            }
            if state.focus_after_paint {
                state.window.focus_window();
                state.focus_after_paint = false;
            }
            self.application.as_mut().unwrap().did_present();
        }
        self.trace
            .record(id, began.elapsed(), self.started.elapsed());
        if self.trace.path.is_some() {
            for cause in self.context.repaint_causes() {
                let cause = cause.to_string();
                if self.trace.causes.len() < 32 || self.trace.causes.contains_key(&cause) {
                    *self.trace.causes.entry(cause).or_default() += 1;
                }
            }
        }
        Ok(())
    }
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.error = Some(error);
        event_loop.exit();
    }
}
impl ApplicationHandler<Event> for Host {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.application.is_some() {
            return;
        }
        let builder = egui::ViewportBuilder::default()
            .with_title(popup::SETTINGS_TITLE)
            .with_inner_size(popup::SETTINGS_SIZE)
            .with_resizable(false)
            .with_visible(std::env::args().any(|a| a == "--settings"))
            .with_icon(egui::IconData {
                rgba: crate::tray::icon_rgba(64),
                width: 64,
                height: 64,
            });
        if let Err(error) = self.create(event_loop, ViewportId::ROOT, builder, None) {
            self.fail(event_loop, error);
            return;
        }
        let window = self.viewports[&ViewportId::ROOT].window.clone();
        let cc = CreationContext {
            egui_ctx: self.context.clone(),
            window: &window,
        };
        self.application = Some(Application::new(App::new(
            &cc,
            self.startup.take().unwrap(),
        )));
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, window: WindowId, event: WindowEvent) {
        let Some((&id, state)) = self
            .viewports
            .iter_mut()
            .find(|(_, state)| state.window.id() == window)
        else {
            return;
        };
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            state.has_frame = false;
        }
        if matches!(
            event,
            WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            state.frame_interval = display_interval(&state.window);
        }
        if matches!(event, WindowEvent::RedrawRequested) && state.has_frame {
            if state.visible
                && state.window.is_minimized() != Some(true)
                && let Some(bitmap) = &state.bitmap
                && let Err(error) = bitmap.present(&state.window, id == popup::id())
            {
                self.fail(event_loop, error);
            }
            return;
        }
        if matches!(event, WindowEvent::CloseRequested) {
            state.info.events.push(egui::ViewportEvent::Close);
            state.close = true;
        }
        let response = state.input.on_window_event(&state.window, &event);
        if response.repaint
            || matches!(
                event,
                WindowEvent::RedrawRequested | WindowEvent::CloseRequested
            )
        {
            state.request(Instant::now());
        }
    }
    fn user_event(&mut self, _: &ActiveEventLoop, event: Event) {
        let event = match event {
            Event::Repaint(event) => event,
            Event::Accessibility(event) => {
                let Some((_, state)) = self
                    .viewports
                    .iter_mut()
                    .find(|(_, state)| state.window.id() == event.window_id)
                else {
                    return;
                };
                match event.window_event {
                    accesskit_winit::WindowEvent::InitialTreeRequested => {
                        self.context.enable_accesskit();
                        state.force_ui = true;
                        state.request(Instant::now());
                    }
                    accesskit_winit::WindowEvent::ActionRequested(request) => {
                        state.input.on_accesskit_action_request(request);
                        state.request(Instant::now());
                    }
                    accesskit_winit::WindowEvent::AccessibilityDeactivated => {
                        self.context.disable_accesskit()
                    }
                }
                return;
            }
        };
        let current = self.context.cumulative_pass_nr_for(event.id);
        if (current == event.pass || current == event.pass + 1)
            && let Some(state) = self.viewports.get_mut(&event.id)
        {
            state.request(event.when);
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if self.stop_at.is_some_and(|stop| now >= stop) {
            event_loop.exit();
            return;
        }
        if self.next_logic.is_some_and(|when| when <= now) {
            self.next_logic = None;
            if let Err(error) = self.draw(ViewportId::ROOT, event_loop, true) {
                self.fail(event_loop, error);
                return;
            }
        }
        let due: Vec<_> = self
            .viewports
            .iter()
            .filter(|(_, state)| state.next.is_some_and(|when| when <= now))
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            if let Err(error) = self.draw(id, event_loop, false) {
                self.fail(event_loop, error);
                return;
            }
        }
        let deadline = self
            .viewports
            .values()
            .filter_map(|v| v.next)
            .chain(self.stop_at)
            .chain(self.next_logic)
            .min();
        event_loop.set_control_flow(deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
    fn exiting(&mut self, _: &ActiveEventLoop) {
        if let Some(app) = &mut self.application {
            app.exit();
        }
        let visible:Vec<_>=self.viewports.iter().map(|(id,state)|serde_json::json!({"surface":if *id==ViewportId::ROOT {"settings"}else{"popup"},"visible":state.window.is_visible(),"minimized":state.window.is_minimized()})).collect();
        self.trace.finish(self.started.elapsed(), visible);
        self.pending.clear();
    }
}
#[derive(Default)]
struct Trace {
    path: Option<std::path::PathBuf>,
    frames: Vec<f64>,
    root: u64,
    popup: u64,
    logic: u64,
    logic_after_warmup: u64,
    causes: BTreeMap<String, u64>,
    input_events: BTreeMap<&'static str, u64>,
    unchanged: u64,
}
impl Trace {
    fn record(&mut self, id: ViewportId, elapsed: Duration, age: Duration) {
        if self.path.is_none() || age < Duration::from_secs(2) {
            return;
        }
        self.frames.push(elapsed.as_secs_f64() * 1000.0);
        if id == ViewportId::ROOT {
            self.root += 1;
        } else {
            self.popup += 1;
        }
    }
    fn finish(&mut self, elapsed: Duration, windows: Vec<serde_json::Value>) {
        let Some(path) = &self.path else {
            return;
        };
        self.frames.sort_unstable_by(f64::total_cmp);
        let percentile = |p: usize| {
            self.frames
                .get(self.frames.len().saturating_sub(1) * p / 100)
                .copied()
        };
        let report = serde_json::json!({"renderer":"CPU / Windows GDI","skipped_unchanged_frames":self.unchanged,"windows_at_exit":windows,"repaint_causes":self.causes,"input_event_counts":self.input_events,"seconds":elapsed.as_secs_f64(),"warmup_seconds":2,"painted_root_frames":self.root,"painted_popup_frames":self.popup,"logic_calls_including_warmup":self.logic,"logic_calls_after_warmup":self.logic_after_warmup,"frame_median_ms":percentile(50),"frame_p95_ms":percentile(95),"frame_max_ms":self.frames.last()});
        if let Err(error) = std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()) {
            eprintln!("Could not save render timings: {error}");
        }
    }
}
pub fn run(startup: update::Startup) -> Result<()> {
    #[cfg(feature = "ui-capture")]
    if std::env::args().any(|arg| arg == "--preview")
        && std::env::var_os("SOUND_LOCK_CAPTURE_TO").is_some()
        && let Ok(view) = std::env::var("SOUND_LOCK_CAPTURE_VIEW")
    {
        anyhow::ensure!(
            ["popup", "audio", "driver", "updates", "dialog"].contains(&view.as_str()),
            "Unknown capture view: {view}"
        );
    }
    let event_loop = EventLoop::<Event>::with_user_event().build()?;
    let context = egui::Context::default();
    context.set_embed_viewports(false);
    let proxy = event_loop.create_proxy();
    let repaint_proxy = proxy.clone();
    context.set_request_repaint_callback(move |request| {
        if let Some(when) = Instant::now().checked_add(request.delay) {
            let _ = repaint_proxy.send_event(Event::Repaint(Repaint {
                id: request.viewport_id,
                when,
                pass: request.current_cumulative_pass_nr,
            }));
        }
    });
    let started = Instant::now();
    let preview = std::env::args().any(|a| a == "--preview")
        || (cfg!(any(feature = "ui-capture", feature = "tray-smoke-test"))
            && std::env::var("SOUND_LOCK_PREVIEW").as_deref() == Ok("1"));
    let stop_at = preview
        .then(|| {
            std::env::var("SOUND_LOCK_RENDER_SECONDS")
                .ok()?
                .parse::<u64>()
                .ok()
                .filter(|v| (1..=120).contains(v))
                .map(|s| started + Duration::from_secs(s))
        })
        .flatten();
    let trace = Trace {
        path: std::env::var_os("SOUND_LOCK_RENDER_REPORT")
            .filter(|p| !p.is_empty())
            .map(Into::into),
        ..Default::default()
    };
    let mut host = Host {
        proxy,
        context,
        application: None,
        startup: Some(startup),
        viewports: BTreeMap::new(),
        renderer: EguiSoftwareRender::new(ColorFieldOrder::Bgra),
        pending: Default::default(),
        error: None,
        next_logic: None,
        started,
        stop_at,
        trace,
    };
    event_loop
        .run_app(&mut host)
        .context("Windows UI event loop failed")?;
    if let Some(error) = host.error {
        return Err(error);
    }
    Ok(())
}

fn display_interval(window: &Window) -> Duration {
    let refresh = window
        .current_monitor()
        .and_then(|m| m.refresh_rate_millihertz())
        .unwrap_or(60_000)
        .clamp(24_000, 360_000);
    Duration::from_nanos(1_000_000_000_000 / u64::from(refresh))
}
