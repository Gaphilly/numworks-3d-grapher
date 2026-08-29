#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

mod app;
mod camera;
pub mod eadk;
mod expression;
mod function;
mod input;
mod math;
mod rendering;
mod surface;
mod ui;

#[cfg(not(test))]
#[used]
#[link_section = ".rodata.eadk_app_name"]
pub static EADK_APP_NAME: [u8; 10] = *b"3DGrapher\0";

#[cfg(not(test))]
#[used]
#[link_section = ".rodata.eadk_api_level"]
pub static EADK_APP_API_LEVEL: u32 = 0;

#[cfg(not(test))]
#[used]
#[link_section = ".rodata.eadk_app_icon"]
pub static EADK_APP_ICON: [u8; 4250] = *include_bytes!("../target/icon.nwi");

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn main() {
    let function = match expression::CompiledExpression::compile("sin(x) * cos(y)") {
        Ok(expression) => expression,
        Err(_) => loop {},
    };
    let mut app = app::AppState::new();

    loop {
        if app.dirty.content {
            match app.active_tab {
                app::Tab::Graph => rendering::render(&app.camera, &function),
                app::Tab::Equation => ui::draw_equation_placeholder(),
                app::Tab::Settings => ui::draw_settings_placeholder(),
            }
            app.dirty.content = false;
        }
        if app.dirty.header {
            ui::draw_header(
                app.active_tab.index(),
                app.selected_tab.index(),
                app.focus == app::Focus::Tabs,
            );
            app.dirty.header = false;
        }

        let keys = eadk::keyboard::scan();
        if let app::UpdateResult::Exit = app.update(keys) {
            return;
        }
        eadk::timing::msleep(20);
    }
}
