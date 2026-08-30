//! Continuous raw-key controls for the graph camera.
//!
//! Camera motion is intentionally driven by `eadk_keyboard_scan`, not semantic
//! events. The main loop samples this state every 20 ms, so held keys move
//! smoothly without entering EADK's potentially blocking event/repeat path.

use crate::camera::Camera;
use crate::eadk::keyboard;

const YAW_STEP: f32 = 0.08;
const PITCH_STEP: f32 = 0.06;
const TRANSLATION_STEP: f32 = 0.12;
const DOLLY_STEP: f32 = 0.15;
const FOCAL_LENGTH_STEP: f32 = 5.0;

/// Result of applying one raw keyboard sample to the camera.
pub enum Action {
    None,
    Redraw,
    Exit,
}

/// Applies the graph-content key mapping to one keyboard state.
///
/// Plain arrows orbit; Shift+arrows translate the orbit target; `+`/`-` dolly
/// the camera. Alpha+`+`/`-` changes focal length/FOV without moving the camera.
/// OK and tab behavior are owned by `AppState` and never handled here.
pub fn update(camera: &mut Camera, state: keyboard::State) -> Action {
    if keyboard::key_down(state, keyboard::BACK) {
        return Action::Exit;
    }

    let shift = keyboard::key_down(state, keyboard::SHIFT);
    let alpha = keyboard::key_down(state, keyboard::ALPHA);
    let mut changed = false;

    if shift {
        if keyboard::key_down(state, keyboard::LEFT) {
            camera.truck(-TRANSLATION_STEP);
            changed = true;
        }
        if keyboard::key_down(state, keyboard::RIGHT) {
            camera.truck(TRANSLATION_STEP);
            changed = true;
        }
        if keyboard::key_down(state, keyboard::UP) {
            camera.pedestal(TRANSLATION_STEP);
            changed = true;
        }
        if keyboard::key_down(state, keyboard::DOWN) {
            camera.pedestal(-TRANSLATION_STEP);
            changed = true;
        }
    } else {
        if keyboard::key_down(state, keyboard::LEFT) {
            camera.orbit(-YAW_STEP, 0.0);
            changed = true;
        }
        if keyboard::key_down(state, keyboard::RIGHT) {
            camera.orbit(YAW_STEP, 0.0);
            changed = true;
        }
        if keyboard::key_down(state, keyboard::UP) {
            camera.orbit(0.0, PITCH_STEP);
            changed = true;
        }
        if keyboard::key_down(state, keyboard::DOWN) {
            camera.orbit(0.0, -PITCH_STEP);
            changed = true;
        }
    }

    if alpha {
        if keyboard::key_down(state, keyboard::PLUS) {
            camera.adjust_focal_length(FOCAL_LENGTH_STEP);
            changed = true;
        }
        if keyboard::key_down(state, keyboard::MINUS) {
            camera.adjust_focal_length(-FOCAL_LENGTH_STEP);
            changed = true;
        }
    } else {
        if keyboard::key_down(state, keyboard::PLUS) {
            camera.dolly(-DOLLY_STEP);
            changed = true;
        }
        if keyboard::key_down(state, keyboard::MINUS) {
            camera.dolly(DOLLY_STEP);
            changed = true;
        }
    }

    if changed {
        Action::Redraw
    } else {
        Action::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(values: &[u8]) -> keyboard::State {
        let mut state = 0_u64;
        for value in values {
            state |= 1_u64 << value;
        }
        state
    }

    #[test]
    fn plain_arrows_orbit_without_translating() {
        let mut camera = Camera::new();
        let target = (camera.target_x, camera.target_y, camera.target_z);
        let yaw = camera.yaw;
        assert!(matches!(
            update(&mut camera, keys(&[keyboard::RIGHT])),
            Action::Redraw
        ));
        assert!(camera.yaw > yaw);
        assert_eq!((camera.target_x, camera.target_y, camera.target_z), target);
    }

    #[test]
    fn shift_arrows_translate_without_orbiting() {
        let mut camera = Camera::new();
        let angles = (camera.yaw, camera.pitch);
        assert!(matches!(
            update(
                &mut camera,
                keys(&[keyboard::SHIFT, keyboard::RIGHT, keyboard::UP])
            ),
            Action::Redraw
        ));
        assert_eq!((camera.yaw, camera.pitch), angles);
        assert!(camera.target_x != 0.0 || camera.target_y != 0.0);
        assert!(camera.target_z > 0.0);
    }

    #[test]
    fn plus_is_dolly_and_alpha_plus_is_fov() {
        let mut camera = Camera::new();
        let initial_distance = camera.distance;
        let initial_focal = camera.focal_length;
        let _ = update(&mut camera, keys(&[keyboard::PLUS]));
        assert!(camera.distance < initial_distance);
        assert_eq!(camera.focal_length, initial_focal);

        let distance = camera.distance;
        let _ = update(&mut camera, keys(&[keyboard::ALPHA, keyboard::PLUS]));
        assert_eq!(camera.distance, distance);
        assert!(camera.focal_length > initial_focal);
    }
}
