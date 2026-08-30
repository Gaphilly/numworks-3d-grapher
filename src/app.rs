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
use crate::input;
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
/// `content` means the current tab body needs drawing. `surface` specifically
/// means equation/domain heights must be resampled; camera changes set only
/// `content`. `header` is isolated so camera motion never redraws static UI.
pub struct DirtyFlags {
    pub header: bool,
    pub content: bool,
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
    pub active_tab: Tab,
    pub selected_tab: Tab,
    pub focus: Focus,
    pub dirty: DirtyFlags,
    pub editor: EquationEditor,
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
            active_tab: Tab::Graph,
            selected_tab: Tab::Graph,
            focus: Focus::Content,
            dirty: DirtyFlags {
                header: true,
                content: true,
                surface: true,
            },
            editor: EquationEditor::new(),
            editor_repeat: EditorKeyRepeat::new(),
            previous_keys: 0,
            pressed_keys: 0,
        }
    }

    /// Raw down edges discovered during the most recent `update` call.
    pub fn pressed_keys(&self) -> keyboard::State {
        self.pressed_keys
    }

    /// Atomically returns to Graph content and invalidates its viewport/header.
    pub fn show_graph(&mut self) {
        self.active_tab = Tab::Graph;
        self.selected_tab = Tab::Graph;
        self.focus = Focus::Content;
        self.dirty.header = true;
        self.dirty.content = true;
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
    /// keys without blocking the main loop.
    pub fn update_editor_repeat(
        &mut self,
        keys: keyboard::State,
        now_ms: u64,
        active_expression: &mut CompiledExpression,
    ) {
        if self.active_tab != Tab::Equation || self.focus != Focus::Content {
            self.editor_repeat.reset();
            return;
        }
        if let Some(event) = self.editor_repeat.update(keys, now_ms) {
            let _ = self.handle_editor_event(event, active_expression);
        }
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
            self.selected_tab = self.active_tab;
            self.focus = Focus::Tabs;
            self.dirty.header = true;
            if self.active_tab == Tab::Equation {
                self.dirty.content = true;
            }
            return UpdateResult::StateChanged;
        }

        if self.active_tab == Tab::Graph {
            if let input::Action::Redraw = input::update(&mut self.camera, keys) {
                self.dirty.content = true;
            }
        }
        UpdateResult::Continue
    }

    fn handle_back(&mut self) -> UpdateResult {
        if self.focus == Focus::Tabs {
            self.selected_tab = self.active_tab;
            self.focus = Focus::Content;
            self.dirty.header = true;
            if self.active_tab == Tab::Equation {
                self.dirty.content = true;
            }
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
    fn graph_back_exits() {
        let mut app = AppState::new();
        assert!(matches!(
            app.update(key(keyboard::BACK)),
            UpdateResult::Exit
        ));
    }

    #[test]
    fn camera_motion_invalidates_projection_but_not_surface_samples() {
        let mut app = AppState::new();
        app.dirty.content = false;
        app.dirty.surface = false;
        assert!(matches!(
            app.update(key(keyboard::RIGHT)),
            UpdateResult::Continue
        ));
        assert!(app.dirty.content);
        assert!(!app.dirty.surface);
    }
}
