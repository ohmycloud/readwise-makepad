//! A page turn changes pixels, never the HTML layout or its selection geometry.
//! One render pass writes to alternating targets: the outgoing texture is no
//! longer attached to a pass, so recycled paragraph draw lists cannot alter it.
use makepad_widgets::*;

script_mod! {
    use mod.prelude.widgets.*
    mod.draw.DrawReaderPageTurn = set_type_default() do #(DrawReaderPageTurn::script_shader(vm)){
        ..mod.draw.DrawQuad
        outgoing: texture_2d(float)
        incoming: texture_2d(float)

        texture_uv: fn(pos){
            let dpi = self.draw_pass.dpi_factor
            let size = ceil(self.rect_size * dpi) / dpi
            let origin = floor(self.rect_pos * dpi) / dpi
            let uv = pos * self.rect_size / size + (self.rect_pos - origin) / size
            return vec2(uv.x, mix(uv.y, 1.0 - uv.y, self.flip_y))
        }

        pixel: fn(){
            let uv = self.texture_uv(self.pos)
            let old = self.outgoing.sample(uv)
            let next = self.incoming.sample(uv)
            if self.progress >= 1.0 { return next }
            if self.progress <= 0.0 { return old }

            // Backward turns run the incoming sheet's curl in reverse. Text
            // itself is never mirrored on the front of either page.
            let phase = mix(self.progress, 1.0 - self.progress, self.backward)
            let front = mix(old, next, self.backward)
            let under = mix(next, old, self.backward)
            let radius = 0.12 * sin(phase * 3.14159265)
            let crease = 1.0 - phase - radius * 0.35 +
                (self.pos.y - 0.5) * sin(phase * 3.14159265) * 0.055
            let distance = self.pos.x - crease
            if distance <= 0.0 { return front }

            let shade = 0.23 * exp(-distance / max(radius * 0.6, 0.001)) *
                sin(phase * 3.14159265)
            let revealed = vec4(under.rgb * (1.0 - shade), under.a)
            if distance >= radius { return revealed }

            // Cylindrical fold: a faint impression of the reverse side,
            // warm/cool paper from the reader palette, and a rounded edge.
            let angle = asin(clamp(distance / max(radius, 0.001), 0.0, 1.0))
            let source_x = clamp(crease - radius * angle, 0.0, 1.0)
            let reverse_uv = self.texture_uv(vec2(source_x, self.pos.y))
            let reverse_old = self.outgoing.sample(reverse_uv)
            let reverse_new = self.incoming.sample(reverse_uv)
            let reverse = mix(reverse_old, reverse_new, self.backward)
            let paper = mix(self.paper.rgb, reverse.rgb, 0.07)
            let lighting = 0.88 + 0.13 * cos(angle) - 0.16 * pow(sin(angle), 8.0)
            let folded = vec4(paper * lighting, 1.0)
            let edge = smoothstep(radius - 1.0 / max(self.rect_size.x, 1.0), radius, distance)
            return mix(folded, revealed, edge)
        }
    }
}

#[derive(Script, ScriptHook)]
#[repr(C)]
pub struct DrawReaderPageTurn {
    #[deref]
    draw_super: DrawQuad,
    #[live(1.0_f32)]
    progress: f32,
    #[live]
    backward: f32,
    #[live]
    paper: Vec4f,
    #[live]
    flip_y: f32,
}

const TURN_SECONDS: f64 = 0.32;

#[derive(Default)]
struct Transition {
    available: bool,
    forward: bool,
    waiting: bool,
    ready_frames: usize,
    started: Option<f64>,
}

impl Transition {
    fn busy(&self) -> bool {
        self.waiting || self.started.is_some()
    }

    fn request(&mut self, forward: bool) -> bool {
        if !self.available || self.busy() {
            return false;
        }
        self.forward = forward;
        self.waiting = true;
        self.ready_frames = 0;
        true
    }

    fn present(&mut self, now: f64, ready: bool) -> f32 {
        self.available = ready;
        if self.waiting {
            // Let native highlight geometry and newly loaded images settle
            // before the incoming page becomes visible in the curl.
            self.ready_frames = if ready { self.ready_frames + 1 } else { 0 };
            if self.ready_frames >= 2 {
                self.waiting = false;
                self.started = Some(now);
            } else {
                return 0.0;
            }
        }
        let Some(started) = self.started else {
            return 1.0;
        };
        let linear = ((now - started) / TURN_SECONDS).clamp(0.0, 1.0);
        if linear >= 1.0 {
            self.started = None;
        }
        // Smooth start/stop, independent of refresh rate.
        (linear * linear * (3.0 - 2.0 * linear)) as f32
    }
}

fn can_snapshot(size: DVec2, dpi: f64) -> bool {
    let width = size.x * dpi;
    let height = size.y * dpi;
    dpi.is_finite()
        && dpi > 0.0
        && width.is_finite()
        && height.is_finite()
        && width >= 1.0
        && height >= 1.0
        && width <= 4096.0
        && height <= 4096.0
        && width.ceil() * height.ceil() <= 8_000_000.0
}

fn render_texture(cx: &mut Cx) -> Texture {
    Texture::new_with_format(
        cx,
        TextureFormat::RenderBGRAu8 {
            size: TextureSize::Auto,
            initial: true,
        },
    )
}

struct PageSurface {
    pass: DrawPass,
    list: DrawList2d,
    targets: [Texture; 2],
    current: usize,
}

#[derive(Default)]
pub struct PageTurn {
    surface: Option<PageSurface>,
    // Includes viewport origin/DPI/palette/fonts/column count, not the page
    // anchor or chapter. Explicit next/previous may animate across chapters.
    appearance: Option<u64>,
    transition: Transition,
    frame: NextFrame,
}

impl PageTurn {
    pub fn busy(&self) -> bool {
        self.transition.busy()
    }

    pub fn cancel(&mut self) {
        self.transition = Transition::default();
        self.frame = NextFrame::default();
    }

    pub fn finish(&mut self) {
        let available = self.transition.available;
        self.cancel();
        self.transition.available = available;
    }

    pub fn request(&mut self, cx: &mut Cx, forward: bool) {
        if self.transition.request(forward) {
            if let Some(surface) = &mut self.surface {
                surface.current = 1 - surface.current;
                self.frame = cx.new_next_frame();
            }
        }
    }

    pub fn handle_event(&mut self, cx: &mut Cx, event: &Event, area: Area) {
        if self.frame.is_event(event).is_some() {
            area.redraw(cx);
        }
    }

    pub fn can_draw_cached(&self, appearance: u64) -> bool {
        self.appearance == Some(appearance) && self.transition.started.is_some()
    }

    pub fn begin(&mut self, cx: &mut Cx2d, viewport: Rect, appearance: u64, paper: Vec4f) -> bool {
        if !can_snapshot(viewport.size, cx.current_dpi_factor()) {
            self.cancel();
            self.appearance = None;
            return false;
        }
        if self.appearance != Some(appearance) {
            self.cancel();
            self.appearance = Some(appearance);
            // Keep the draw-list/pass identity stable while areas in native
            // paragraph widgets still refer to it. Replace only the textures.
            if let Some(surface) = &mut self.surface {
                surface.targets = [render_texture(cx), render_texture(cx)];
            }
        }
        let surface = self.surface.get_or_insert_with(|| PageSurface {
            pass: DrawPass::new(cx),
            list: DrawList2d::new(cx),
            targets: [render_texture(cx), render_texture(cx)],
            current: 0,
        });
        surface.pass.set_color_texture(
            cx,
            &surface.targets[surface.current],
            DrawPassClearColor::ClearWith(paper),
        );
        cx.make_child_pass(&surface.pass);
        cx.begin_pass(&surface.pass, None);
        surface.list.begin_always(cx);
        true
    }

    pub fn end(
        &mut self,
        cx: &mut Cx2d,
        draw: &mut DrawReaderPageTurn,
        viewport: Rect,
        paper: Vec4f,
        ready: bool,
    ) {
        let Some(surface) = &mut self.surface else {
            return;
        };
        surface.list.end(cx);
        cx.end_pass(&surface.pass);
        self.draw_cached(cx, draw, viewport, paper, ready);
    }

    pub fn draw_cached(
        &mut self,
        cx: &mut Cx2d,
        draw: &mut DrawReaderPageTurn,
        viewport: Rect,
        paper: Vec4f,
        ready: bool,
    ) {
        let Some(surface) = &self.surface else { return };
        let was_busy = self.transition.busy();
        draw.progress = self.transition.present(cx.time(), ready);
        draw.backward = if self.transition.forward { 0.0 } else { 1.0 };
        draw.paper = paper;
        draw.flip_y = if cfg!(any(
            target_os = "android",
            target_os = "linux",
            target_arch = "wasm32"
        )) {
            1.0
        } else {
            0.0
        };
        let old = if self.transition.busy() {
            1 - surface.current
        } else {
            surface.current
        };
        draw.draw_vars.set_texture(0, &surface.targets[old]);
        draw.draw_vars
            .set_texture(1, &surface.targets[surface.current]);
        draw.draw_abs(cx, viewport);
        cx.set_pass_area(&surface.pass, draw.area());
        // Also redraw once after the last frame, resuming chapter measurement
        // and native widget updates that were paused during the transition.
        if was_busy {
            self.frame = cx.new_next_frame();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_animation_before_a_ready_snapshot() {
        let mut turn = Transition::default();
        assert!(!turn.request(true));
        assert_eq!(turn.present(0.0, false), 1.0);
        assert!(!turn.request(true));
        turn.present(0.0, true);
        assert!(turn.request(true));
    }

    #[test]
    fn waits_for_two_consecutive_ready_frames() {
        let mut turn = Transition {
            available: true,
            ..Default::default()
        };
        assert!(turn.request(true));
        assert_eq!(turn.present(1.0, false), 0.0);
        assert_eq!(turn.present(2.0, true), 0.0);
        assert_eq!(turn.present(3.0, false), 0.0);
        assert_eq!(turn.present(4.0, true), 0.0);
        assert_eq!(turn.present(5.0, true), 0.0);
        assert!(!turn.waiting);
        assert_eq!(turn.started, Some(5.0));
    }

    #[test]
    fn bounded_monotone_animation_in_both_directions() {
        for forward in [true, false] {
            let mut turn = Transition {
                available: true,
                ..Default::default()
            };
            assert!(turn.request(forward));
            turn.present(0.0, true);
            assert_eq!(turn.present(0.0, true), 0.0);
            let mut previous = 0.0;
            for frame in 1..=60 {
                let progress = turn.present(frame as f64 / 60.0, true);
                assert!(progress >= previous && progress <= 1.0);
                previous = progress;
            }
            assert_eq!(previous, 1.0);
            assert!(!turn.busy());
            assert!(turn.request(!forward));
        }
    }

    #[test]
    fn repeated_input_cannot_replace_the_frozen_page() {
        let mut turn = Transition {
            available: true,
            ..Default::default()
        };
        assert!(turn.request(true));
        assert!(!turn.request(false));
        turn.present(1.0, true);
        turn.present(1.0, true);
        assert!(!turn.request(false));
        assert!(turn.forward);
    }

    #[test]
    fn cancels_pending_or_running_frames() {
        let mut turn = PageTurn::default();
        turn.transition.available = true;
        turn.transition.request(true);
        turn.cancel();
        assert!(!turn.busy());
        assert!(!turn.transition.available);
    }

    #[test]
    fn snapshot_budget_preserves_native_dpi_or_falls_back() {
        assert!(can_snapshot(dvec2(930.0, 580.0), 2.0));
        assert!(can_snapshot(dvec2(400.0, 800.0), 3.0));
        assert!(!can_snapshot(dvec2(4000.0, 2000.0), 2.0));
        assert!(!can_snapshot(dvec2(0.0, 580.0), 2.0));
        assert!(!can_snapshot(dvec2(f64::NAN, 580.0), 2.0));
        assert!(!can_snapshot(dvec2(930.0, 580.0), f64::INFINITY));
    }
}
