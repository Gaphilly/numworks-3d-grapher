#![no_std]
#![no_main]

mod camera;
pub mod eadk;
mod function;
mod input;
mod math;
mod rendering;
mod surface;

#[used]
#[link_section = ".rodata.eadk_app_name"]
pub static EADK_APP_NAME: [u8; 10] = *b"3DGrapher\0";

#[used]
#[link_section = ".rodata.eadk_api_level"]
pub static EADK_APP_API_LEVEL: u32 = 0;

#[used]
#[link_section = ".rodata.eadk_app_icon"]
pub static EADK_APP_ICON: [u8; 4250] = *include_bytes!("../target/icon.nwi");

#[no_mangle]
pub extern "C" fn main() {
    let mut camera = camera::Camera::new();
    let function = function::SinCosSurface;
    rendering::render(&camera, &function);

    loop {
        let state = eadk::keyboard::scan();
        match input::update(&mut camera, state) {
            input::Action::Exit => return,
            input::Action::Redraw => rendering::render(&camera, &function),
            input::Action::None => eadk::timing::msleep(20),
        }
    }
}
