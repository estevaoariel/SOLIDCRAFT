use super::ball::{Ball, DOTS};
use super::*;

fn reading(axis: usize, value: f32) -> [f32; 6] {
    let mut travel = [0.0; 6];
    travel[axis] = value;
    travel
}

/// Where a point on the cap lands on screen for a reading.
fn on_screen(travel: [f32; 6], p: P3) -> [f32; 2] {
    project(Pose::new(travel).place(p))
}

const TOP_CENTRE: P3 = [0.0, 0.0, CAP_HEIGHT];
const FRONT: P3 = [0.0, -CAP_RADIUS, CAP_HEIGHT / 2.0];

#[test]
fn the_cap_moves_the_way_the_reading_says() {
    let rest = on_screen([0.0; 6], TOP_CENTRE);
    // Slid right: right on screen.
    assert!(on_screen(reading(0, 1.0), TOP_CENTRE)[0] > rest[0]);
    // Pressed down: lower on screen; pulled up: higher.
    assert!(on_screen(reading(2, 1.0), TOP_CENTRE)[1] > rest[1]);
    assert!(on_screen(reading(2, -1.0), TOP_CENTRE)[1] < rest[1]);
    // Pulled toward the user: nearer, so lower on screen.
    assert!(Pose::new(reading(1, 1.0)).place(TOP_CENTRE)[1] < 0.0);
}

#[test]
fn tilts_and_twists_turn_the_cap_the_right_way() {
    let axis = |travel| apply(&Pose::new(travel).turn, [0.0, 0.0, 1.0]);
    assert!((axis([0.0; 6])[2] - 1.0).abs() < 1e-6);
    // +Rx: the top leans toward the user (the near edge down); tilted left: to the left.
    assert!(axis(reading(3, 1.0))[1] < -0.1);
    assert!(axis(reading(4, 1.0))[0] < -0.1);
    // Twisted clockwise (from above): the groove on the front swings to the user's left.
    let rest = Pose::new([0.0; 6]).place(FRONT);
    let twisted = Pose::new(reading(5, 1.0)).place(FRONT);
    assert!(twisted[0] < rest[0] - 0.1);
    assert!((twisted[2] - rest[2]).abs() < 1e-6, "a twist alone keeps the cap's height");
}

#[test]
fn hostile_readings_stay_finite() {
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30] {
        let pose = Pose::new([value; 6]);
        assert!(pose.place(TOP_CENTRE).iter().all(|v| v.is_finite()));
    }
}

#[test]
fn the_hull_keeps_only_the_outline() {
    let square = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0), (1.0, 1.0), (1.0, 0.0)];
    let hull = hull(square.iter().map(|&(x, y)| Pos2::new(x, y)).collect());
    assert_eq!(hull.len(), 4);
    assert!(!hull.contains(&Pos2::new(1.0, 1.0)));
    assert!(super::hull(Vec::new()).is_empty());
}

#[test]
fn the_ball_rolls_the_way_it_is_pushed() {
    let top_after = |rate: [f32; 3]| {
        let mut ball = Ball::new();
        // A quarter turn, in small steps as frames give it.
        for _ in 0..25 {
            ball.roll([rate[0], rate[1], rate[2], 0.0], 0.01);
        }
        apply(&ball.turn, [0.0, 0.0, 1.0])
    };
    assert!(top_after([1.0, 0.0, 0.0])[1] > 0.9, "rolled away: the top goes away");
    assert!(top_after([0.0, 1.0, 0.0])[0] > 0.9, "rolled right: the top goes right");
    let mut ball = Ball::new();
    for _ in 0..25 {
        ball.roll([0.0, 0.0, 1.0, 0.0], 0.01);
    }
    // Spun clockwise from above: what faced the user turns to their left.
    assert!(apply(&ball.turn, [0.0, -1.0, 0.0])[0] < -0.9);
}

#[test]
fn the_ball_stays_a_rotation() {
    let mut ball = Ball::new();
    for i in 0..20_000 {
        ball.roll([0.7, -0.3, (i % 7) as f32 * 0.2, 0.5], 0.016);
    }
    for (i, a) in ball.turn.iter().enumerate() {
        for (j, b) in ball.turn.iter().enumerate() {
            let expected = if i == j { 1.0 } else { 0.0 };
            assert!((dot(*a, *b) - expected).abs() < 1e-4);
        }
    }
    for value in [f32::NAN, f32::INFINITY, -1e30] {
        ball.roll([value; 4], value);
        assert!(ball.turn.iter().flatten().all(|v| v.is_finite()) && ball.wheel.is_finite());
        assert!((0..DOTS).all(|i| ball.dot(i).iter().all(|v| v.is_finite())));
    }
}

#[test]
fn the_scroll_ring_turns_and_keeps_its_turn() {
    let mut ball = Ball::new();
    for _ in 0..10 {
        ball.roll([0.0, 0.0, 0.0, 1.0], 0.01);
    }
    let turned = ball.wheel;
    assert!((turned - std::f32::consts::TAU * 0.1).abs() < 1e-4);
    // Let go: a scroll ring stays where it was turned to, and the ball doesn't move with it.
    ball.roll([0.0; 4], 0.01);
    assert_eq!(ball.wheel, turned);
    assert!((apply(&ball.turn, [0.0, 0.0, 1.0])[2] - 1.0).abs() < 1e-6);
}

#[test]
fn each_step_starts_at_rest_and_eases_out_to_full() {
    let [out, hold, back, rest] = DEMO_SECONDS;
    assert_eq!(demo_swing(0.0), 0.0);
    assert!((demo_swing(out) - 1.0).abs() < 1e-6);
    assert_eq!(demo_swing(out + hold / 2.0), 1.0);
    assert_eq!(demo_swing(out + hold + back + rest / 2.0), 0.0);
    // Gently: it creeps away from rest rather than jumping, and never goes back on itself on the way out.
    assert!(demo_swing(out * 0.05) < 0.01);
    let samples: Vec<f32> = (0..=100).map(|i| demo_swing(out * i as f32 / 100.0)).collect();
    assert!(samples.windows(2).all(|w| w[1] >= w[0]));
    // Again from rest after a full showing; nothing odd for odd times.
    assert_eq!(demo_swing(out + hold + back + rest), 0.0);
    for seconds in [f32::NAN, f32::INFINITY, -5.0] {
        assert_eq!(demo_swing(seconds), 0.0);
    }
}

#[test]
fn the_corner_picture_goes_where_it_is_told_and_clears_the_bars() {
    use crate::prefs::SpaceMouseGhost::{LeftViewport as Current, LeftWindow as Left, RightViewport as Right};
    let view = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
    let size = egui::vec2(160.0, 120.0);
    // Free: lower left for Current and Left, lower right for Right.
    assert_eq!(corner_rect(view, Current, &[]).left_bottom(), egui::pos2(12.0, 588.0));
    assert_eq!(corner_rect(view, Left, &[]).left_bottom(), egui::pos2(12.0, 588.0));
    assert_eq!(corner_rect(view, Right, &[]).right_bottom(), egui::pos2(788.0, 588.0));
    // The History bar along the bottom pushes each one up to clear it.
    let history = egui::Rect::from_min_max(egui::pos2(0.0, 556.0), egui::pos2(800.0, 596.0));
    for position in [Current, Left, Right] {
        let rect = corner_rect(view, position, &[history]);
        assert_eq!(rect.size(), size);
        assert!(!rect.intersects(history) && rect.bottom() <= history.top() - 12.0 + f32::EPSILON, "{position:?}");
    }
    // A bar that does not reach the picture leaves it where it was.
    let elsewhere = egui::Rect::from_min_max(egui::pos2(300.0, 556.0), egui::pos2(500.0, 596.0));
    for position in [Current, Left, Right] {
        assert_eq!(corner_rect(view, position, &[elsewhere]), corner_rect(view, position, &[]));
    }
    // The navigation bar in the middle of a narrow view is not covered either, nor is anything else stacked.
    let narrow = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(420.0, 600.0));
    let navigation = egui::Rect::from_min_max(egui::pos2(100.0, 520.0), egui::pos2(330.0, 560.0));
    let history = egui::Rect::from_min_max(egui::pos2(0.0, 566.0), egui::pos2(420.0, 600.0));
    for position in [Left, Right] {
        let rect = corner_rect(narrow, position, &[history, navigation]);
        assert!(narrow.contains_rect(rect), "{position:?} stays inside the view");
        assert!(!rect.intersects(history) && !rect.intersects(navigation), "{position:?} clears both bars: {rect:?}");
    }
    // A view narrower than the picture keeps it at the left edge rather than outside.
    let tiny = egui::Rect::from_min_size(egui::pos2(50.0, 0.0), egui::vec2(100.0, 400.0));
    assert!(corner_rect(tiny, Right, &[]).left() >= tiny.left());
    // An empty bar is no bar.
    assert_eq!(corner_rect(view, Left, &[egui::Rect::NOTHING]), corner_rect(view, Left, &[]));
}

#[test]
fn stacked_bars_in_a_short_view_do_not_push_the_picture_out_of_it() {
    use crate::prefs::SpaceMouseGhost::{LeftViewport, LeftWindow, RightViewport};
    let view = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(500.0, 220.0));
    let history = egui::Rect::from_min_max(egui::pos2(0.0, 180.0), egui::pos2(500.0, 220.0));
    let navigation = egui::Rect::from_min_max(egui::pos2(0.0, 130.0), egui::pos2(500.0, 170.0));
    let chip = egui::Rect::from_min_max(egui::pos2(0.0, 80.0), egui::pos2(500.0, 120.0));
    for position in [LeftViewport, RightViewport, LeftWindow] {
        let rect = corner_rect(view, position, &[history, navigation, chip]);
        assert!(rect.top() >= view.top() && view.contains_rect(rect), "{position:?}: {rect:?} is outside {view:?}");
    }
}
