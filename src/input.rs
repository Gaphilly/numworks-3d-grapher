use crate::camera::Camera;
use crate::eadk::keyboard;

pub enum Action {
    None,
    Redraw,
    Exit,
}

pub fn update(camera: &mut Camera, state: keyboard::State) -> Action {
    if keyboard::key_down(state, keyboard::BACK) {
        return Action::Exit;
    }

    let mut changed = false;
    if keyboard::key_down(state, keyboard::LEFT) {
        camera.yaw -= 0.08;
        changed = true;
    }
    if keyboard::key_down(state, keyboard::RIGHT) {
        camera.yaw += 0.08;
        changed = true;
    }
    if keyboard::key_down(state, keyboard::UP) {
        camera.pitch += 0.06;
        if camera.pitch > 1.35 {
            camera.pitch = 1.35;
        }
        changed = true;
    }
    if keyboard::key_down(state, keyboard::DOWN) {
        camera.pitch -= 0.06;
        if camera.pitch < -1.35 {
            camera.pitch = -1.35;
        }
        changed = true;
    }
    if keyboard::key_down(state, keyboard::PLUS) {
        camera.zoom *= 1.06;
        if camera.zoom > 2.0 {
            camera.zoom = 2.0;
        }
        changed = true;
    }
    if keyboard::key_down(state, keyboard::MINUS) {
        camera.zoom *= 0.94;
        if camera.zoom < 0.45 {
            camera.zoom = 0.45;
        }
        changed = true;
    }

    if changed {
        Action::Redraw
    } else {
        Action::None
    }
}
