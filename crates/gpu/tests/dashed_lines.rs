//! A dashed line (the trail lines of an exploded view) is drawn in dashes on screen: the pixels
//! along it come and go, where a solid line runs on unbroken. Skipped, with the reason printed,
//! on a machine without a GPU adapter (`SOLIDCRAFT_REQUIRE_GPU=1` makes that a failure).

use solidcraft_engine::geom::Vec3;
use solidcraft_engine::render::{Camera, Rgb, Scene, SceneLine, StandardView};
use solidcraft_gpu::GpuError;
use solidcraft_gpu::capture::Capturer;

fn capturer() -> Option<&'static Capturer> {
    match solidcraft_gpu::capture::shared() {
        Ok(c) => Some(c),
        Err(e @ GpuError::NoAdapter(_)) if std::env::var_os("SOLIDCRAFT_REQUIRE_GPU").is_none() => {
            eprintln!("skipping: {e}");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

/// A line across a front view, drawn alone: `dashed` as a dashed line, otherwise as a plain one.
/// The row it runs along, and where it is drawn.
fn row(c: &Capturer, dashed: bool) -> Vec<bool> {
    let mut cam = Camera { distance: 62.5, fov: 0.0, ..Default::default() };
    cam.set_view(StandardView::Front);
    let line = || SceneLine { points: vec![Vec3::new(-8.0, 0.0, 0.0), Vec3::new(8.0, 0.0, 0.0)], color: Rgb(255, 0, 0), width: 2.0, on_top: true };
    let scene = Scene {
        lines: if dashed { vec![] } else { vec![line()] },
        dashed: if dashed { vec![line()] } else { vec![] },
        radius: 30.0,
        ..Default::default()
    };
    let img = c.rgba(&scene, &cam, 200, 200).unwrap();
    (0..200)
        .map(|x| {
            let i = (100 * 200 + x) * 4;
            img[i] > 120 && img[i] > img[i + 1].saturating_add(40) && img[i] > img[i + 2].saturating_add(40)
        })
        .collect()
}

#[test]
fn a_dashed_line_is_drawn_in_dashes() {
    let Some(c) = capturer() else { return };
    let solid = row(c, false);
    let dashed = row(c, true);
    let drawn = |r: &[bool]| r.iter().filter(|d| **d).count();
    assert!(drawn(&solid) > 50, "the solid line is drawn: {} pixels", drawn(&solid));
    assert!(drawn(&dashed) > 15, "the dashed line is drawn: {} pixels", drawn(&dashed));
    assert!(drawn(&dashed) + 10 < drawn(&solid), "the dashed line has gaps: {} vs {} pixels", drawn(&dashed), drawn(&solid));
    // The line comes and goes along its length: a gap in the middle of it.
    assert!(dashed.windows(2).any(|w| w[0] != w[1]), "the dashes are apart");
}

/// A line that goes away from the eye is dashed evenly: the dash is measured on the screen, so
/// the dashes are the same length whatever the depth of the line.
#[test]
fn a_dashed_line_is_dashed_evenly_in_perspective() {
    let Some(c) = capturer() else { return };
    let mut cam = Camera { distance: 62.5, fov: 0.6, ..Default::default() };
    cam.set_view(StandardView::Front);
    // A line that crosses the view and goes away from the eye at the same time: its parts lie
    // at different depths, so the world does not divide it evenly on the screen.
    let line = SceneLine { points: vec![Vec3::new(-6.0, -30.0, 0.0), Vec3::new(6.0, 30.0, 0.0)], color: Rgb(255, 0, 0), width: 2.0, on_top: true };
    let scene = Scene { dashed: vec![line], radius: 60.0, ..Default::default() };
    let img = c.rgba(&scene, &cam, 200, 200).unwrap();
    let red = |x: usize| {
        let i = (100 * 200 + x) * 4;
        img[i] > 150 && img[i + 1] < 100
    };
    // The dashes: the stretches where the line is drawn.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for x in 0..200 {
        match (red(x), start) {
            (true, None) => start = Some(x),
            (false, Some(s)) => {
                runs.push((s, x));
                start = None;
            }
            _ => {}
        }
    }
    assert!(runs.len() >= 4, "the line is dashed: {runs:?}");
    for w in runs.windows(2) {
        let period = w[1].0 as i64 - w[0].0 as i64;
        assert!((period - 12).abs() <= 2, "the dashes are 12 px apart: {runs:?}");
    }
}
