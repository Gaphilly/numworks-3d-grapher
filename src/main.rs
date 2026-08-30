#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

mod app;
mod camera;
pub mod eadk;
mod editor;
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
    let mut function = match expression::CompiledExpression::compile("sin(x) * cos(y)") {
        Ok(expression) => expression,
        Err(_) => loop {},
    };
    let mut app = app::AppState::new();

    loop {
        if app.dirty.content {
            match app.active_tab {
                app::Tab::Graph => rendering::render(&app.camera, &function),
                app::Tab::Equation => {
                    ui::draw_equation_editor(&app.editor, app.focus == app::Focus::Content)
                }
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
        let update = app.update(keys);
        if update == app::UpdateResult::Exit {
            return;
        }

        if app.pressed_keys() != 0 {
            let semantic_event = poll_semantic_event(keys);
            if update == app::UpdateResult::Continue
                && app.active_tab == app::Tab::Equation
                && app.focus == app::Focus::Content
            {
                if let Some(event) = semantic_event {
                    let _ = app.handle_editor_event(event, &mut function);
                }
            }
        }
        app.update_editor_repeat(keys, eadk::timing::millis(), &mut function);
        eadk::timing::msleep(20);
    }
}

#[cfg(not(test))]
fn poll_semantic_event(keys: eadk::keyboard::State) -> Option<eadk::event::Event> {
    let first = eadk::event::poll();
    let modifier_mask = (1_u64 << eadk::keyboard::SHIFT) | (1_u64 << eadk::keyboard::ALPHA);
    if matches!(first, Some(eadk::event::SHIFT) | Some(eadk::event::ALPHA))
        && keys & !modifier_mask != 0
    {
        eadk::event::poll()
    } else {
        first
    }
}
