use std::collections::{HashSet, VecDeque};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalCapabilities {
    pub kitty_graphics: bool,
}

pub fn kitty_graphics_command(kind: &str, id: usize) -> String {
    format!("\u{1b}_Gmdtui={kind},id={id}\u{1b}\\")
}

pub fn contains_forbidden_text_sizing(sequence: &str) -> bool {
    sequence.contains("\u{1b}]66") || sequence.contains("OSC-66")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadlineImageCache {
    next_id: usize,
    entries: Vec<(String, usize)>,
}

impl Default for HeadlineImageCache {
    fn default() -> Self {
        Self {
            next_id: 1,
            entries: Vec::new(),
        }
    }
}

impl HeadlineImageCache {
    pub fn id_for(&mut self, key: &str) -> usize {
        if let Some((_, id)) = self.entries.iter().find(|(entry_key, _)| entry_key == key) {
            return *id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push((key.to_string(), id));
        id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageCache {
    budget: usize,
    entries: VecDeque<(String, usize)>,
}

impl ImageCache {
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            entries: VecDeque::new(),
        }
    }

    pub fn insert(&mut self, key: impl Into<String>, bytes: usize) {
        self.entries.push_back((key.into(), bytes));
        while self.entries.iter().map(|(_, size)| *size).sum::<usize>() > self.budget {
            self.entries.pop_front();
        }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.iter().any(|(entry_key, _)| entry_key == key)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    KeyDown(String),
    KeyRepeat(String),
    KeyUp(String),
    MouseMove { x: u16, y: u16 },
    Text(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputEngine {
    down: HashSet<String>,
    pending: Vec<InputEvent>,
}

impl InputEngine {
    pub fn push(&mut self, event: InputEvent) {
        match &event {
            InputEvent::KeyDown(key) => {
                self.down.insert(key.clone());
                self.pending.push(event);
            }
            InputEvent::KeyRepeat(key) => {
                if self.down.contains(key) {
                    self.pending.push(event);
                }
            }
            InputEvent::KeyUp(key) => {
                self.down.remove(key);
                self.pending.retain(|pending| match pending {
                    InputEvent::KeyRepeat(repeat) => repeat != key,
                    _ => true,
                });
                self.pending.push(event);
            }
            InputEvent::MouseMove { .. } | InputEvent::Text(_) => self.pending.push(event),
        }
    }

    pub fn drain_frame(&mut self) -> Vec<InputEvent> {
        let mut latest_motion = None;
        let mut out = Vec::new();
        for event in self.pending.drain(..) {
            match event {
                InputEvent::MouseMove { x, y } => {
                    latest_motion = Some(InputEvent::MouseMove { x, y })
                }
                other => out.push(other),
            }
        }
        if let Some(motion) = latest_motion {
            out.push(motion);
        }
        out
    }
}
