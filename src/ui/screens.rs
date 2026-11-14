//! Keeps the window on the screen it is on when it changes size. A show
//! machine often has a second display running a fullscreen output, and the
//! window must never grow across onto it.

use eframe::egui::{self, Pos2, Rect, Vec2};

/// Every screen, in the units the system positions windows in.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn screens() -> Vec<Rect> {
    display_info::DisplayInfo::all()
        .map(|all| {
            all.iter()
                .map(|d| {
                    Rect::from_min_size(
                        egui::pos2(d.x as f32, d.y as f32),
                        Vec2::new(d.width as f32, d.height as f32),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Elsewhere the window is left where the window manager puts it.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn screens() -> Vec<Rect> {
    Vec::new()
}

/// How many of the system's units make one of egui's points. macOS places
/// windows in its own points, Windows in real pixels.
fn unit(ctx: &egui::Context) -> f32 {
    let ppp = ctx.pixels_per_point();
    if cfg!(target_os = "macos") {
        ppp / ctx.native_pixels_per_point().unwrap_or(ppp)
    } else {
        ppp
    }
}

/// Where a window at `old` should sit on `screen` once it is `size`.
/// `in_proportion` keeps it the same fraction of the way across and down, so
/// a window against the right edge stays against the right edge; otherwise
/// it only moves as far as it must to stay on the screen.
fn moved(screen: Rect, old: Rect, size: Vec2, in_proportion: bool) -> Pos2 {
    let axis = |lo: f32, hi: f32, old_min: f32, old_len: f32, len: f32| {
        let room = (hi - lo - len).max(0.0);
        if !in_proportion {
            return old_min.clamp(lo, lo + room);
        }
        let old_room = hi - lo - old_len;
        let part = if old_room > 1.0 {
            ((old_min - lo) / old_room).clamp(0.0, 1.0)
        } else {
            0.5
        };
        lo + part * room
    };
    egui::pos2(
        axis(
            screen.left(),
            screen.right(),
            old.left(),
            old.width(),
            size.x,
        ),
        axis(
            screen.top(),
            screen.bottom(),
            old.top(),
            old.height(),
            size.y,
        ),
    )
}

/// The position to move the window to when its inside becomes `inner`
/// (in egui points), or `None` when the screens are not known.
pub fn place(ctx: &egui::Context, inner: Vec2, in_proportion: bool) -> Option<Pos2> {
    let (outer, old_inner) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
    let (outer, old_inner) = (outer?, old_inner?);
    let k = unit(ctx);
    let old = Rect::from_min_size((outer.min.to_vec2() * k).to_pos2(), outer.size() * k);
    // The title bar and borders stay the same size.
    let size = (inner + (outer.size() - old_inner.size()).max(Vec2::ZERO)) * k;

    let all = screens();
    let overlap = |s: &Rect| {
        let both = s.intersect(old);
        if both.is_positive() {
            both.area()
        } else {
            0.0
        }
    };
    let mut screen = *all
        .iter()
        .find(|s| s.contains(old.center()))
        .or_else(|| all.iter().max_by(|a, b| overlap(a).total_cmp(&overlap(b))))?;
    if cfg!(target_os = "macos") {
        // Stay below the menu bar, unless the window already sits where one
        // would be (a screen without one).
        screen.min.y += (old.top() - screen.top()).clamp(0.0, 38.0);
    }
    let to = moved(screen, old, size, in_proportion);
    Some((to.to_vec2() / k).to_pos2())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(egui::pos2(x, y), Vec2::new(w, h))
    }

    #[test]
    fn a_window_on_the_right_edge_stays_on_its_screen() {
        // Two screens side by side; a small window against the right edge of
        // the left one grows to the large size.
        let screen = rect(0.0, 0.0, 1920.0, 1080.0);
        let small = rect(1640.0, 400.0, 280.0, 400.0);
        let to = moved(screen, small, Vec2::new(1280.0, 820.0), true);
        assert_eq!(to.x, 1920.0 - 1280.0, "still against the right edge");
        assert!(to.y >= 0.0 && to.y + 820.0 <= 1080.0);
    }

    #[test]
    fn a_centred_window_stays_centred() {
        let screen = rect(-1920.0, 0.0, 1920.0, 1080.0);
        let old = rect(-1920.0 + 820.0, 440.0, 280.0, 200.0);
        let to = moved(screen, old, Vec2::new(440.0, 400.0), true);
        assert_eq!(to.x, -1920.0 + 740.0);
        assert_eq!(to.y, 340.0);
    }

    #[test]
    fn growing_taller_only_moves_as_far_as_it_must() {
        let screen = rect(0.0, 0.0, 1920.0, 1080.0);
        let old = rect(100.0, 800.0, 280.0, 200.0);
        let to = moved(screen, old, Vec2::new(280.0, 400.0), false);
        assert_eq!(to, egui::pos2(100.0, 680.0));
        let fits = moved(screen, old, Vec2::new(280.0, 250.0), false);
        assert_eq!(fits, old.min);
    }

    #[test]
    fn a_window_bigger_than_the_screen_starts_at_its_corner() {
        let screen = rect(1920.0, 0.0, 1024.0, 768.0);
        let old = rect(2600.0, 300.0, 280.0, 200.0);
        let to = moved(screen, old, Vec2::new(1280.0, 820.0), true);
        assert_eq!(to, egui::pos2(1920.0, 0.0));
    }
}
