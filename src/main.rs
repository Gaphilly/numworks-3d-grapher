#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

//! NumWorks application entry point and cooperative main loop.
//!
//! Production builds have neither `std` nor a conventional runtime entry point.
//! nwlink reads the metadata symbols below, resolves EADK imports against the
//! calculator firmware, packages the relocatable object as an NWA, and invokes
//! this exported C-ABI `main`.

mod app;
mod camera;
pub mod eadk;
mod editor;
mod expression;
mod function;
mod graph;
mod input;
mod math;
mod rendering;
mod settings;
mod surface;
mod ui;

// Set false to disable all freeze-diagnostic header breadcrumbs. This is kept
// separate from the Settings Performance readout: a stuck frame cannot publish
// its final duration, so the breadcrumb is useful even when that readout is off.
const RENDER_FREEZE_DIAGNOSTICS: bool = false;

#[cfg(not(test))]
/// NUL-terminated name exposed in the NWA metadata section.
#[used]
#[link_section = ".rodata.eadk_app_name"]
pub static EADK_APP_NAME: [u8; 10] = *b"3DGrapher\0";

#[cfg(not(test))]
/// EADK ABI level requested by this external application.
#[used]
#[link_section = ".rodata.eadk_api_level"]
pub static EADK_APP_API_LEVEL: u32 = 0;

#[cfg(not(test))]
/// Build-script-generated NWI icon embedded for nwlink packaging.
#[used]
#[link_section = ".rodata.eadk_app_icon"]
// The NWI payload is LZ4-compressed, so its byte length legitimately changes
// when the PNG artwork changes. Keep the array length derived from the actual
// generated asset rather than coupling Rust source to one icon revision.
pub static EADK_APP_ICON: [u8; include_bytes!("../target/icon.nwi").len()] =
    *include_bytes!("../target/icon.nwi");

#[cfg(not(test))]
/// Firmware-invoked C entry point. It never depends on a Rust standard runtime.
#[no_mangle]
pub extern "C" fn main() {
    let mut function = match expression::CompiledExpression::compile("sin(x) * cos(y)") {
        Ok(expression) => expression,
        Err(_) => loop {},
    };
    let mut app = app::AppState::new();
    // Heights are separate from the compiled bytecode and projection. Camera
    // movement therefore pays only transformation/rasterization cost.
    let mut surface = surface::SurfaceGrid::sample(app.domain, &function);
    app.dirty.surface = false;

    loop {
        if app.dirty.content {
            match app.active_tab {
                app::Tab::Graph => {
                    if app.dirty.surface {
                        surface.resample(app.domain, &function);
                        app.dirty.surface = false;
                    }
                    let render_started_ms = eadk::timing::millis();
                    rendering::render(
                        &app.camera,
                        app.domain,
                        &surface,
                        app.graph_options,
                        RENDER_FREEZE_DIAGNOSTICS,
                    );
                    app.record_graph_render_ms(
                        eadk::timing::millis().saturating_sub(render_started_ms),
                    );
                    app.dirty.graph = false;
                }
                app::Tab::Equation => {
                    ui::draw_equation_editor(&app.editor, app.focus == app::Focus::Content)
                }
                app::Tab::Settings => ui::draw_settings(
                    &app.settings,
                    app.graph_options,
                    app.domain,
                    app.focus == app::Focus::Content,
                    app.graph_render_profile_ms(),
                ),
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

        // Raw state owns continuous camera motion and application focus changes.
        // Semantic events are polled only where text/tab input needs them, so a
        // graph camera key cannot stall this 20 ms loop in eadk_event_get().
        let keys = eadk::keyboard::scan();
        let editor_was_active =
            app.active_tab == app::Tab::Equation && app.focus == app::Focus::Content;
        let settings_editor_was_active = app.active_tab == app::Tab::Settings
            && app.focus == app::Focus::Content
            && app.settings.is_editing();
        let tabs_were_focused = app.focus == app::Focus::Tabs;
        let update = app.update(keys);
        if update == app::UpdateResult::Exit {
            return;
        }

        let semantic_input_context = editor_was_active
            || settings_editor_was_active
            || tabs_were_focused
            || app.focus == app::Focus::Tabs;
        if app.pressed_keys() != 0 && semantic_input_context {
            let semantic_event = poll_semantic_event(keys);
            if update == app::UpdateResult::Continue
                && app.active_tab == app::Tab::Equation
                && app.focus == app::Focus::Content
            {
                if let Some(event) = semantic_event {
                    let _ = app.handle_editor_event(event, &mut function);
                }
            } else if update == app::UpdateResult::Continue
                && settings_editor_was_active
                && app.active_tab == app::Tab::Settings
                && app.focus == app::Focus::Content
                && app.settings.is_editing()
            {
                if let Some(event) = semantic_event {
                    let _ = app.handle_settings_event(event);
                }
            }
        }
        app.update_key_repeat(keys, eadk::timing::millis(), &mut function);
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
