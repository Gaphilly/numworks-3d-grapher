#[derive(Clone, Copy)]
#[repr(C)]
pub struct Color {
    pub rgb565: u16,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

pub mod backlight {
    pub fn set_brightness(brightness: u8) {
        unsafe {
            eadk_backlight_set_brightness(brightness);
        }
    }
    pub fn brightness() -> u8 {
        unsafe {
            return eadk_backlight_brightness();
        }
    }

    extern "C" {
        fn eadk_backlight_set_brightness(brightness: u8);
        fn eadk_backlight_brightness() -> u8;
    }
}

pub mod display {
    use super::Color;
    use super::Rect;

    pub fn push_rect(rect: Rect, pixels: &[Color]) {
        let required = rect.width as usize * rect.height as usize;
        if pixels.len() < required {
            return;
        }
        unsafe {
            eadk_display_push_rect(rect, pixels.as_ptr());
        }
    }

    pub fn push_rect_uniform(rect: Rect, color: Color) {
        unsafe {
            eadk_display_push_rect_uniform(rect, color);
        }
    }

    pub fn wait_for_vblank() {
        unsafe {
            eadk_display_wait_for_vblank();
        }
    }

    extern "C" {
        fn eadk_display_push_rect_uniform(rect: Rect, color: Color);
        fn eadk_display_push_rect(rect: Rect, color: *const Color);
        fn eadk_display_wait_for_vblank();
    }
}

pub mod keyboard {
    pub type State = u64;

    pub const LEFT: u8 = 0;
    pub const UP: u8 = 1;
    pub const DOWN: u8 = 2;
    pub const RIGHT: u8 = 3;
    pub const BACK: u8 = 5;
    pub const PLUS: u8 = 45;
    pub const MINUS: u8 = 46;

    pub fn scan() -> State {
        unsafe { eadk_keyboard_scan() }
    }

    pub fn key_down(state: State, key: u8) -> bool {
        key < 64 && ((state >> key) & 1) != 0
    }

    extern "C" {
        fn eadk_keyboard_scan() -> State;
    }
}

pub mod timing {
    pub fn usleep(us: u32) {
        unsafe {
            eadk_timing_usleep(us);
        }
    }

    pub fn msleep(ms: u32) {
        unsafe {
            eadk_timing_msleep(ms);
        }
    }

    pub fn millis() -> u64 {
        unsafe {
            return eadk_timing_millis();
        }
    }

    extern "C" {
        fn eadk_timing_usleep(us: u32);
        fn eadk_timing_msleep(us: u32);
        fn eadk_timing_millis() -> u64;
    }
}

pub fn random() -> u32 {
    unsafe { return eadk_random() }
}

extern "C" {
    fn eadk_random() -> u32;
}

#[cfg(not(test))]
use core::panic::PanicInfo;

#[cfg(not(test))]
#[panic_handler]
fn panic(_panic: &PanicInfo<'_>) -> ! {
    loop {} // FIXME: Do something better. Exit the app maybe?
}
