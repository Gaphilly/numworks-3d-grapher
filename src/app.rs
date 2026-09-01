//! Application-level tab/focus state machine and dirty-state ownership.
//!
//! There are four user interaction contexts: Graph content, Equation content,
//! Settings content, and the shared tab bar. Content OK moves focus to the tab
//! bar; tab Left/Right changes selection; tab OK atomically activates content;
//! tab Back cancels navigation. Equation Back returns to Graph without compiling,
//! while Graph Back exits. These transitions stay in the cooperative main loop—
//! no tab or editor owns a nested/blocking input loop.
//!
//! Raw keyboard state owns application transitions and continuous graph controls.
//! Semantic EADK events own calculator-style editor characters. Keeping those
//! responsibilities separate prevents a physical key from being applied twice.

use crate::camera::Camera;
use crate::eadk::keyboard;
use crate::editor::EditorAction;
use crate::editor::EditorKeyRepeat;
use crate::editor::EquationEditor;
use crate::expression::CompiledExpression;
use crate::graph::GraphOptions;
use crate::input;
use crate::settings::{SettingsAction, SettingsState};
use crate::surface::Domain;

#[derive(Clone, Copy, Debug, PartialEq)]
/// Top-level application views.
pub enum Tab {
    Graph,
    Equation,
    Settings,
}

impl Tab {
    /// Stable display index used by the three-segment header.
    pub fn index(self) -> usize {
        match self {
            Tab::Graph => 0,
            Tab::Equation => 1,
            Tab::Settings => 2,
        }
    }

    fn previous(self) -> Tab {
        match self {
            Tab::Graph => Tab::Settings,
            Tab::Equation => Tab::Graph,
            Tab::Settings => Tab::Equation,
        }
    }

    fn next(self) -> Tab {
        match self {
            Tab::Graph => Tab::Equation,
            Tab::Equation => Tab::Settings,
            Tab::Settings => Tab::Graph,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// Whether input belongs to the selected tab's content or to tab navigation.
pub enum Focus {
    Content,
    Tabs,
}

/// Independent invalidation domains.
///
/// `content` means the current tab body needs drawing. `graph` durably records
/// that the graph viewport must be recomposed, even when Settings is currently
/// covering it. `surface` specifically means equation/domain heights must be
/// resampled. `header` is isolated so camera motion never redraws static UI.
pub struct DirtyFlags {
    pub header: bool,
    pub content: bool,
    pub graph: bool,
    pub surface: bool,
}

#[derive(Clone, Copy, PartialEq)]
/// Outcome consumed by the cooperative main loop.
pub enum UpdateResult {
    Continue,
    StateChanged,
    Exit,
}

/// Complete allocation-free application/UI state.
pub struct AppState {
    pub camera: Camera,
    pub domain: Domain,
    pub graph_options: GraphOptions,
    pub settings: SettingsState,
    pub active_tab: Tab,
    pub selected_tab: Tab,
    pub focus: Focus,
    pub dirty: DirtyFlags,
    pub editor: EquationEditor,
    last_graph_render_ms: u32,
    editor_repeat: EditorKeyRepeat,
    previous_keys: keyboard::State,
    pressed_keys: keyboard::State,
}

impl AppState {
    /// Starts in Graph content with the default domain and equation editor text.
    pub fn new() -> AppState {
        AppState {
            camera: Camera::new(),
            domain: Domain::DEFAULT,
            graph_options: GraphOptions::DEFAULT,
            settings: SettingsState::new(),
            active_tab: Tab::Graph,
            selected_tab: Tab::Graph,
            focus: Focus::Content,
            dirty: DirtyFlags {
                header: true,
                content: true,
                graph: true,
                surface: true,
            },
            editor: EquationEditor::new(),
            last_graph_render_ms: 0,
            editor_repeat: EditorKeyRepeat::new(),
            previous_keys: 0,
            pressed_keys: 0,
        }
    }

    /// Raw down edges discovered during the most recent `update` call.
    pub fn pressed_keys(&self) -> keyboard::State {
        self.pressed_keys
    }

    /// Duration of the latest complete graph redraw, including display transfer.
    pub fn graph_render_profile_ms(&self) -> u32 {
        self.last_graph_render_ms
    }

    /// Records one complete graph render for the temporary hardware profiler.
    pub fn record_graph_render_ms(&mut self, elapsed_ms: u64) {
        self.last_graph_render_ms = elapsed_ms.min(u32::MAX as u64) as u32;
    }

    /// Atomically returns to Graph content and invalidates its viewport/header.
    pub fn show_graph(&mut self) {
        self.active_tab = Tab::Graph;
        self.selected_tab = Tab::Graph;
        self.focus = Focus::Content;
        self.dirty.header = true;
        self.dirty.content = true;
        self.dirty.graph = true;
    }

    /// Applies one semantic calculator event to Equation content.
    ///
    /// Successful EXE replaces the active bytecode and marks surface samples
    /// dirty. Failed compilation leaves the active bytecode untouched.
    pub fn handle_editor_event(
        &mut self,
        event: crate::eadk::event::Event,
        active_expression: &mut CompiledExpression,
    ) -> UpdateResult {
        match self.editor.handle_event(event) {
            EditorAction::None => UpdateResult::Continue,
            EditorAction::Changed => {
                self.dirty.content = true;
                UpdateResult::Continue
            }
            EditorAction::Submit => {
                if self.editor.compile_into(active_expression) {
                    self.dirty.surface = true;
                    self.show_graph();
                    UpdateResult::StateChanged
                } else {
                    self.dirty.content = true;
                    UpdateResult::Continue
                }
            }
            EditorAction::Cancel => {
                self.editor.dismiss_error();
                self.show_graph();
                UpdateResult::StateChanged
            }
            EditorAction::FocusTabs => {
                self.selected_tab = self.active_tab;
                self.focus = Focus::Tabs;
                self.dirty.header = true;
                self.dirty.content = true;
                UpdateResult::StateChanged
            }
        }
    }

    /// Generates bounded-time Backspace/Left/Right repeat events from raw held
    /// keys without blocking the main loop. The same timer is routed to whichever
    /// fixed-capacity text field currently owns semantic input.
    pub fn update_key_repeat(
        &mut self,
        keys: keyboard::State,
        now_ms: u64,
        active_expression: &mut CompiledExpression,
    ) {
        let equation_active = self.active_tab == Tab::Equation && self.focus == Focus::Content;
        let settings_active = self.active_tab == Tab::Settings
            && self.focus == Focus::Content
            && self.settings.is_editing();
        if !equation_active && !settings_active {
            self.editor_repeat.reset();
            return;
        }
        if let Some(event) = self.editor_repeat.update(keys, now_ms) {
            if equation_active {
                let _ = self.handle_editor_event(event, active_expression);
            } else {
                let _ = self.handle_settings_event(event);
            }
        }
    }

    /// Applies one semantic numeric event to the active Settings domain field.
    pub fn handle_settings_event(&mut self, event: crate::eadk::event::Event) -> UpdateResult {
        let action = self.settings.handle_editor_event(event, self.domain);
        self.apply_settings_action(action)
    }

    /// Advances the focus/tab/camera state from one raw keyboard sample.
    pub fn update(&mut self, keys: keyboard::State) -> UpdateResult {
        let pressed = keys & !self.previous_keys;
        self.pressed_keys = pressed;
        self.previous_keys = keys;

        if keyboard::key_down(pressed, keyboard::BACK) {
            return self.handle_back();
        }

        if self.focus == Focus::Tabs {
            if keyboard::key_down(pressed, keyboard::LEFT) {
                self.selected_tab = self.selected_tab.previous();
                self.dirty.header = true;
            }
            if keyboard::key_down(pressed, keyboard::RIGHT) {
                self.selected_tab = self.selected_tab.next();
                self.dirty.header = true;
            }
            if keyboard::key_down(pressed, keyboard::OK) {
                if self.active_tab != self.selected_tab {
                    self.active_tab = self.selected_tab;
                    self.dirty.content = true;
                }
                self.focus = Focus::Content;
                self.dirty.header = true;
                self.dirty.content = true;
                return UpdateResult::StateChanged;
            }
            return UpdateResult::Continue;
        }

        if keyboard::key_down(pressed, keyboard::OK) {
            // Leaving a numeric field for the tab bar cancels its draft exactly
            // like Back; the active domain remains transactional and unchanged.
            if self.active_tab == Tab::Settings && self.settings.is_editing() {
                let _ = self.settings.back();
            }
            if self.active_tab == Tab::Equation {
                let _ = self.editor.close_function_picker();
            }
            self.selected_tab = self.active_tab;
            self.focus = Focus::Tabs;
            self.dirty.header = true;
            if self.active_tab == Tab::Equation || self.active_tab == Tab::Settings {
                self.dirty.content = true;
            }
            return UpdateResult::StateChanged;
        }

        if self.active_tab == Tab::Graph {
            if let input::Action::Redraw = input::update(&mut self.camera, keys) {
                self.dirty.content = true;
                self.dirty.graph = true;
            }
        } else if self.active_tab == Tab::Settings && !self.settings.is_editing() {
            let action = if keyboard::key_down(pressed, keyboard::UP) {
                self.settings.select_previous()
            } else if keyboard::key_down(pressed, keyboard::DOWN) {
                self.settings.select_next()
            } else if keyboard::key_down(pressed, keyboard::LEFT) {
                self.settings.adjust_left(&mut self.graph_options)
            } else if keyboard::key_down(pressed, keyboard::RIGHT) {
                self.settings.adjust_right(&mut self.graph_options)
            } else if keyboard::key_down(pressed, keyboard::EXE) {
                self.settings.activate(&mut self.graph_options, self.domain)
            } else {
                SettingsAction::None
            };
            return self.apply_settings_action(action);
        }
        UpdateResult::Continue
    }

    fn handle_back(&mut self) -> UpdateResult {
        if self.focus == Focus::Tabs {
            self.selected_tab = self.active_tab;
            self.focus = Focus::Content;
            self.dirty.header = true;
            if self.active_tab == Tab::Equation || self.active_tab == Tab::Settings {
                self.dirty.content = true;
            }
            return UpdateResult::StateChanged;
        }
        if self.active_tab == Tab::Settings {
            let action = self.settings.back();
            return self.apply_settings_action(action);
        }
        if self.active_tab == Tab::Equation && self.editor.close_function_picker() {
            self.dirty.content = true;
            return UpdateResult::StateChanged;
        }
        if self.active_tab != Tab::Graph {
            self.active_tab = Tab::Graph;
            self.selected_tab = Tab::Graph;
            self.dirty.header = true;
            self.dirty.content = true;
            return UpdateResult::StateChanged;
        }
        UpdateResult::Exit
    }

    fn apply_settings_action(&mut self, action: SettingsAction) -> UpdateResult {
        match action {
            SettingsAction::None => UpdateResult::Continue,
            SettingsAction::Redraw => {
                self.dirty.content = true;
                UpdateResult::Continue
            }
            SettingsAction::GraphChanged => {
                self.dirty.content = true;
                self.dirty.graph = true;
                UpdateResult::Continue
            }
            SettingsAction::DomainChanged(domain) => {
                self.domain = domain;
                self.dirty.content = true;
                self.dirty.graph = true;
                self.dirty.surface = true;
                UpdateResult::Continue
            }
            SettingsAction::ResetCamera => {
                self.camera.reset();
                self.dirty.content = true;
                self.dirty.graph = true;
                UpdateResult::Continue
            }
            SettingsAction::LeaveSettings => {
                self.show_graph();
                UpdateResult::StateChanged
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eadk::event;
    use crate::function::SurfaceFunction;

    fn key(key: u8) -> keyboard::State {
        1_u64 << key
    }

    fn release(app: &mut AppState) {
        let _ = app.update(0);
    }

    fn enter_equation(app: &mut AppState) {
        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.focus, Focus::Tabs);
        release(app);

        assert!(matches!(
            app.update(key(keyboard::RIGHT)),
            UpdateResult::Continue
        ));
        assert_eq!(app.selected_tab, Tab::Equation);
        release(app);

        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.active_tab, Tab::Equation);
        assert_eq!(app.focus, Focus::Content);
        release(app);
    }

    fn enter_settings(app: &mut AppState) {
        assert_eq!(app.graph_options, GraphOptions::DEFAULT);
        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        release(app);
        let _ = app.update(key(keyboard::RIGHT));
        release(app);
        let _ = app.update(key(keyboard::RIGHT));
        assert_eq!(app.selected_tab, Tab::Settings);
        release(app);
        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.active_tab, Tab::Settings);
        assert_eq!(app.focus, Focus::Content);
        release(app);
    }

    fn settings_move_down(app: &mut AppState, rows: usize) {
        let mut row = 0;
        while row < rows {
            let _ = app.update(key(keyboard::DOWN));
            release(app);
            row += 1;
        }
    }

    #[test]
    fn graph_tabs_and_equation_focus_transitions() {
        let mut app = AppState::new();
        enter_equation(&mut app);

        let before = app.editor.source().len();
        let mut active = CompiledExpression::compile("x").expect("valid expression");
        assert!(matches!(
            app.handle_editor_event(event::ONE, &mut active),
            UpdateResult::Continue
        ));
        assert_eq!(app.active_tab, Tab::Equation);
        assert_eq!(app.focus, Focus::Content);
        assert_eq!(app.editor.source().len(), before + 1);

        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.focus, Focus::Tabs);
    }

    #[test]
    fn exe_compiles_and_returns_to_graph() {
        let mut app = AppState::new();
        enter_equation(&mut app);
        let mut active = CompiledExpression::compile("x").expect("valid expression");
        app.dirty.surface = false;

        assert!(matches!(
            app.handle_editor_event(event::EXE, &mut active),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.active_tab, Tab::Graph);
        assert_eq!(app.focus, Focus::Content);
        assert!(app.dirty.surface);
        let expected = CompiledExpression::compile("sin(x) * cos(y)")
            .expect("valid expression")
            .evaluate(0.5, 0.25);
        assert!((active.evaluate(0.5, 0.25) - expected).abs() < 0.0001);
    }

    #[test]
    fn equation_back_preserves_active_expression() {
        let mut app = AppState::new();
        enter_equation(&mut app);
        let active = CompiledExpression::compile("x+y").expect("valid expression");
        let before = active.evaluate(2.0, 3.0);

        assert!(matches!(
            app.update(key(keyboard::BACK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.active_tab, Tab::Graph);
        assert_eq!(active.evaluate(2.0, 3.0), before);
    }

    #[test]
    fn equation_picker_back_and_ok_preserve_equation_focus_rules() {
        let mut app = AppState::new();
        enter_equation(&mut app);
        let mut active = CompiledExpression::compile("x").expect("valid expression");
        assert!(matches!(
            app.handle_editor_event(event::TOOLBOX, &mut active),
            UpdateResult::Continue
        ));
        assert!(app.editor.function_picker_open());
        assert!(matches!(
            app.update(key(keyboard::BACK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.active_tab, Tab::Equation);
        assert_eq!(app.focus, Focus::Content);
        assert!(!app.editor.function_picker_open());

        release(&mut app);
        let _ = app.handle_editor_event(event::TOOLBOX, &mut active);
        assert!(app.editor.function_picker_open());
        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.focus, Focus::Tabs);
        assert!(!app.editor.function_picker_open());
    }

    #[test]
    fn graph_back_exits() {
        let mut app = AppState::new();
        assert!(matches!(
            app.update(key(keyboard::BACK)),
            UpdateResult::Exit
        ));
    }

    #[test]
    fn app_state_storage_remains_bounded() {
        assert_eq!(core::mem::size_of::<AppState>(), 312);
    }

    #[test]
    fn camera_motion_invalidates_projection_but_not_surface_samples() {
        let mut app = AppState::new();
        app.dirty.content = false;
        app.dirty.graph = false;
        app.dirty.surface = false;
        assert!(matches!(
            app.update(key(keyboard::RIGHT)),
            UpdateResult::Continue
        ));
        assert!(app.dirty.content);
        assert!(app.dirty.graph);
        assert!(!app.dirty.surface);
    }

    #[test]
    fn settings_mode_change_dirties_hidden_graph_but_not_surface() {
        let mut app = AppState::new();
        enter_settings(&mut app);
        app.dirty.graph = false;
        app.dirty.surface = false;
        app.dirty.content = false;

        assert!(matches!(
            app.update(key(keyboard::RIGHT)),
            UpdateResult::Continue
        ));
        assert_eq!(
            app.graph_options.rendering_mode,
            crate::graph::RenderingMode::Solid
        );
        assert!(app.dirty.graph);
        assert!(app.dirty.content);
        assert!(!app.dirty.surface);
    }

    #[test]
    fn settings_appearance_change_dirties_graph_without_resampling() {
        let mut app = AppState::new();
        enter_settings(&mut app);
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        assert_eq!(
            app.settings.page(),
            crate::settings::SettingsPage::Appearance
        );

        app.dirty.graph = false;
        app.dirty.surface = false;
        app.dirty.content = false;
        let _ = app.update(key(keyboard::RIGHT));
        assert_eq!(
            app.graph_options.lighting,
            crate::graph::LightingPreset::Soft
        );
        assert!(app.dirty.graph);
        assert!(app.dirty.content);
        assert!(!app.dirty.surface);

        release(&mut app);
        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.focus, Focus::Tabs);
        assert_eq!(
            app.settings.page(),
            crate::settings::SettingsPage::Appearance
        );
    }

    #[test]
    fn custom_color_apply_dirties_graph_without_resampling() {
        let mut app = AppState::new();
        enter_settings(&mut app);
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        let _ = app.update(key(keyboard::DOWN));
        release(&mut app);
        let _ = app.update(key(keyboard::LEFT));
        release(&mut app);
        assert_eq!(
            app.graph_options.surface_palette,
            crate::graph::SurfacePalette::Custom
        );
        let original = app.graph_options.custom_rgb;
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        assert_eq!(
            app.settings.page(),
            crate::settings::SettingsPage::CustomColor
        );

        let _ = app.update(key(keyboard::RIGHT));
        release(&mut app);
        assert_eq!(app.graph_options.custom_rgb, original);
        settings_move_down(&mut app, 3);
        app.dirty.content = false;
        app.dirty.graph = false;
        app.dirty.surface = false;
        let _ = app.update(key(keyboard::EXE));
        assert_eq!(app.graph_options.custom_rgb.red, original.red + 8);
        assert!(app.dirty.content);
        assert!(app.dirty.graph);
        assert!(!app.dirty.surface);
    }

    #[test]
    fn custom_color_ok_focuses_tabs_without_committing_draft() {
        let mut app = AppState::new();
        enter_settings(&mut app);
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        let _ = app.update(key(keyboard::DOWN));
        release(&mut app);
        let _ = app.update(key(keyboard::LEFT));
        release(&mut app);
        let original = app.graph_options.custom_rgb;
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        let _ = app.update(key(keyboard::RIGHT));
        release(&mut app);

        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.focus, Focus::Tabs);
        assert_eq!(
            app.settings.page(),
            crate::settings::SettingsPage::CustomColor
        );
        assert_eq!(app.graph_options.custom_rgb, original);
    }

    #[test]
    fn settings_camera_reset_invalidates_projection_only() {
        let mut app = AppState::new();
        app.camera.orbit(0.4, -0.2);
        app.camera.truck(1.0);
        enter_settings(&mut app);
        settings_move_down(&mut app, 6);
        app.dirty.graph = false;
        app.dirty.surface = false;

        let _ = app.update(key(keyboard::EXE));
        let default = Camera::new();
        assert_eq!(app.camera.yaw, default.yaw);
        assert_eq!(app.camera.pitch, default.pitch);
        assert_eq!(app.camera.target_x, default.target_x);
        assert_eq!(app.camera.target_y, default.target_y);
        assert!(app.dirty.graph);
        assert!(!app.dirty.surface);
    }

    #[test]
    fn accepted_settings_domain_is_transactional_and_resamples() {
        let mut app = AppState::new();
        enter_settings(&mut app);
        settings_move_down(&mut app, 5);
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        assert_eq!(app.settings.page(), crate::settings::SettingsPage::Domain);
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        assert!(app.settings.is_editing());

        let _ = app.handle_settings_event(event::CLEAR);
        let _ = app.handle_settings_event(event::MINUS);
        let _ = app.handle_settings_event(event::TWO);
        app.dirty.graph = false;
        app.dirty.surface = false;
        let _ = app.handle_settings_event(event::EXE);

        assert_eq!(app.domain.x_min, -2.0);
        assert!(app.dirty.graph);
        assert!(app.dirty.surface);
        assert!(!app.settings.is_editing());
    }

    #[test]
    fn settings_back_unwinds_edit_page_then_returns_to_graph() {
        let mut app = AppState::new();
        enter_settings(&mut app);
        settings_move_down(&mut app, 5);
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        let _ = app.update(key(keyboard::EXE));
        release(&mut app);
        assert!(app.settings.is_editing());

        assert!(matches!(
            app.update(key(keyboard::BACK)),
            UpdateResult::Continue
        ));
        assert!(!app.settings.is_editing());
        release(&mut app);
        assert!(matches!(
            app.update(key(keyboard::BACK)),
            UpdateResult::Continue
        ));
        assert_eq!(app.settings.page(), crate::settings::SettingsPage::Main);
        release(&mut app);
        assert!(matches!(
            app.update(key(keyboard::BACK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.active_tab, Tab::Graph);
        assert_eq!(app.focus, Focus::Content);
    }

    #[test]
    fn settings_tab_back_redraws_the_restored_content_focus() {
        let mut app = AppState::new();
        enter_settings(&mut app);
        assert!(matches!(
            app.update(key(keyboard::OK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.focus, Focus::Tabs);
        release(&mut app);

        // Model the main loop having drawn the unfocused Settings content.
        app.dirty.content = false;
        assert!(matches!(
            app.update(key(keyboard::BACK)),
            UpdateResult::StateChanged
        ));
        assert_eq!(app.focus, Focus::Content);
        assert!(app.dirty.content);
    }
}
