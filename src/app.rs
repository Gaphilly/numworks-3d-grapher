use crate::camera::Camera;
use crate::eadk::keyboard;
use crate::input;

#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Graph,
    Equation,
    Settings,
}

impl Tab {
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

#[derive(Clone, Copy, PartialEq)]
pub enum Focus {
    Content,
    Tabs,
}

pub struct DirtyFlags {
    pub header: bool,
    pub content: bool,
}

pub enum UpdateResult {
    Continue,
    Exit,
}

pub struct AppState {
    pub camera: Camera,
    pub active_tab: Tab,
    pub selected_tab: Tab,
    pub focus: Focus,
    pub dirty: DirtyFlags,
    previous_keys: keyboard::State,
}

impl AppState {
    pub const fn new() -> AppState {
        AppState {
            camera: Camera::new(),
            active_tab: Tab::Graph,
            selected_tab: Tab::Graph,
            focus: Focus::Content,
            dirty: DirtyFlags {
                header: true,
                content: true,
            },
            previous_keys: 0,
        }
    }

    pub fn update(&mut self, keys: keyboard::State) -> UpdateResult {
        let pressed = keys & !self.previous_keys;
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
            }
            return UpdateResult::Continue;
        }

        if keyboard::key_down(pressed, keyboard::OK) {
            self.selected_tab = self.active_tab;
            self.focus = Focus::Tabs;
            self.dirty.header = true;
            return UpdateResult::Continue;
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
            return UpdateResult::Continue;
        }
        if self.active_tab != Tab::Graph {
            self.active_tab = Tab::Graph;
            self.selected_tab = Tab::Graph;
            self.dirty.header = true;
            self.dirty.content = true;
            return UpdateResult::Continue;
        }
        UpdateResult::Exit
    }
}
