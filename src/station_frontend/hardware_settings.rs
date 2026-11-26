use crate::{
    Library, Screen,
    hardware::{self, Audio, Display, DisplayId, Level},
};
use bevy::{
    prelude::*,
    tasks::{IoTaskPool, Task, block_on, poll_once},
};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub const DEVICE: usize = 0;
pub const BRIGHTNESS: usize = 1;
pub const VOLUME: usize = 2;
pub const MUTE: usize = 3;
pub const REFRESH: usize = 4;
pub const SORT: usize = 5;
pub const MOTION: usize = 6;

#[derive(Resource, Default)]
pub struct State {
    pub displays: Option<Vec<Display>>,
    pub audio: Option<Result<Audio, String>>,
    pub selected: usize,
    pub display_error: Option<(DisplayId, String)>,
    pub audio_error: Option<String>,
}

impl State {
    pub fn display(&self) -> Option<&Display> {
        self.displays.as_ref()?.get(self.selected)
    }

    pub fn cycle(&mut self, direction: i16) {
        if let Some(displays) = &self.displays
            && !displays.is_empty()
        {
            self.selected = if direction < 0 {
                (self.selected + displays.len() - 1) % displays.len()
            } else {
                (self.selected + 1) % displays.len()
            };
        }
    }
}

#[derive(Clone)]
enum DisplayRequest {
    Refresh,
    Set(DisplayId, u32, u16),
}

enum DisplayResult {
    Devices(Vec<Display>),
    Level(DisplayId, Result<Level, String>),
}

#[derive(Clone)]
enum AudioRequest {
    Refresh,
    Volume(u16),
    Mute(bool),
}

struct Queue<Request, Response> {
    pending: VecDeque<Request>,
    active: Option<(Request, Task<Response>)>,
    next: Instant,
}

impl<Request: Clone, Response: Send + 'static> Queue<Request, Response> {
    fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            active: None,
            next: Instant::now(),
        }
    }

    fn push(&mut self, request: Request, same: impl Fn(&Request) -> bool) {
        self.pending.retain(|old| !same(old));
        self.pending.push_back(request);
    }

    fn latest(&self) -> impl Iterator<Item = &Request> {
        self.pending
            .iter()
            .rev()
            .chain(self.active.iter().map(|(request, _)| request))
    }

    fn tick(
        &mut self,
        interval: impl Fn(&Request) -> Duration,
        execute: fn(Request) -> Response,
    ) -> Option<Response>
    where
        Request: Send + 'static,
    {
        let result = self
            .active
            .as_mut()
            .and_then(|(_, task)| block_on(poll_once(task)));
        if result.is_some() {
            self.active = None;
        }
        if self.active.is_none()
            && Instant::now() >= self.next
            && let Some(request) = self.pending.pop_front()
        {
            self.next = Instant::now() + interval(&request);
            let work = request.clone();
            self.active = Some((
                request,
                IoTaskPool::get().spawn(async move { execute(work) }),
            ));
        }
        result
    }
}

#[derive(Resource)]
pub struct Jobs {
    display: Queue<DisplayRequest, DisplayResult>,
    audio: Queue<AudioRequest, (bool, Result<Audio, String>)>,
}

impl Default for Jobs {
    fn default() -> Self {
        Self {
            display: Queue::new(),
            audio: Queue::new(),
        }
    }
}

impl Jobs {
    pub fn refresh(&mut self) {
        self.display.push(DisplayRequest::Refresh, |old| {
            matches!(old, DisplayRequest::Refresh)
        });
        self.audio.push(AudioRequest::Refresh, |old| {
            matches!(old, AudioRequest::Refresh)
        });
    }

    pub fn adjust_brightness(&mut self, display: &Display, direction: i16) {
        let Some(max) = display.max() else { return };
        let previous = self
            .display
            .latest()
            .find_map(|entry| match entry {
                DisplayRequest::Set(id, _, percent) if id == &display.id => Some(*percent),
                _ => None,
            })
            .unwrap_or_else(|| display.level.as_ref().unwrap().percent);
        let next = step(previous, direction);
        if previous != next {
            self.display.push(
                DisplayRequest::Set(display.id.clone(), max, next),
                |old| matches!(old, DisplayRequest::Set(id, ..) if id == &display.id),
            );
        }
    }

    pub fn adjust_volume(&mut self, audio: &Audio, direction: i16) {
        let previous = self
            .audio
            .latest()
            .find_map(|entry| match entry {
                AudioRequest::Volume(percent) => Some(*percent),
                _ => None,
            })
            .unwrap_or(audio.volume);
        let next = step(previous, direction);
        if previous != next {
            self.audio.push(AudioRequest::Volume(next), |old| {
                matches!(old, AudioRequest::Volume(_))
            });
        }
    }

    pub fn toggle_mute(&mut self, audio: &Audio) {
        let previous = self
            .audio
            .latest()
            .find_map(|entry| match entry {
                AudioRequest::Mute(muted) => Some(*muted),
                _ => None,
            })
            .unwrap_or(audio.muted);
        self.audio.push(AudioRequest::Mute(!previous), |old| {
            matches!(old, AudioRequest::Mute(_))
        });
    }
}

fn step(previous: u16, direction: i16) -> u16 {
    (i32::from(previous) + i32::from(direction) * 5).clamp(0, 100) as u16
}

pub fn tick(
    library: Res<Library>,
    mut was_open: Local<bool>,
    mut state: ResMut<State>,
    mut jobs: ResMut<Jobs>,
) {
    let open = library.screen == Screen::Settings;
    if open && !*was_open {
        jobs.refresh();
    }
    *was_open = open;
    let jobs = jobs.bypass_change_detection();
    if let Some(result) = jobs.display.tick(
        |request| {
            Duration::from_millis(
                if matches!(request, DisplayRequest::Set(DisplayId::Ddc(_), ..)) {
                    300
                } else {
                    85
                },
            )
        },
        |request| match request {
            DisplayRequest::Refresh => DisplayResult::Devices(hardware::displays()),
            DisplayRequest::Set(id, max, percent) => {
                let reading =
                    hardware::brightness(&id, max, percent).map_err(|error| format!("{error:#}"));
                DisplayResult::Level(id, reading)
            }
        },
    ) {
        match result {
            DisplayResult::Devices(displays) => {
                let selected = state.display().and_then(|previous| {
                    displays
                        .iter()
                        .position(|display| display.id == previous.id)
                });
                state.selected = selected.unwrap_or(0);
                state.displays = Some(displays);
                state.display_error = None;
            }
            DisplayResult::Level(id, Ok(level)) => {
                if let Some(display) = state
                    .displays
                    .as_mut()
                    .and_then(|displays| displays.iter_mut().find(|display| display.id == id))
                {
                    display.level = Ok(level);
                }
                if state
                    .display_error
                    .as_ref()
                    .is_some_and(|(device, _)| *device == id)
                {
                    state.display_error = None;
                }
            }
            DisplayResult::Level(id, Err(error)) => state.display_error = Some((id, error)),
        }
    }
    if let Some((refresh, result)) = jobs.audio.tick(
        |_| Duration::from_millis(85),
        |request| {
            let refresh = matches!(request, AudioRequest::Refresh);
            let result = match request {
                AudioRequest::Refresh => hardware::audio(),
                AudioRequest::Volume(percent) => hardware::volume(percent),
                AudioRequest::Mute(muted) => hardware::mute(muted),
            };
            (refresh, result.map_err(|error| format!("{error:#}")))
        },
    ) {
        if !refresh && let Err(error) = result {
            state.audio_error = Some(error);
        } else {
            state.audio = Some(result);
            state.audio_error = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_volume_steps_coalesce_and_keep_the_mute_request() {
        let audio = Audio {
            name: "Fixture".into(),
            volume: 40,
            muted: false,
        };
        let mut jobs = Jobs::default();
        jobs.adjust_volume(&audio, 1);
        jobs.toggle_mute(&audio);
        jobs.adjust_volume(&audio, 1);
        assert_eq!(jobs.audio.pending.len(), 2);
        assert!(matches!(jobs.audio.pending[0], AudioRequest::Mute(true)));
        assert!(matches!(jobs.audio.pending[1], AudioRequest::Volume(50)));
    }
}
