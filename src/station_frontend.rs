#[path = "station_frontend/hardware.rs"]
mod hardware;
#[path = "station_frontend/hardware_settings.rs"]
mod hardware_settings;
#[path = "station_frontend/screens.rs"]
mod screens;
mod station_protocol;
#[path = "station_frontend/system_menu.rs"]
mod system_menu;
#[path = "station_frontend/textures.rs"]
mod textures;

use anyhow::{Context, Result};
use bevy::{
    prelude::*,
    window::{MonitorSelection, PrimaryWindow, WindowCloseRequested, WindowMode},
    winit::{UpdateMode, WinitSettings},
};
use station_protocol::{GameCard, Reply};
use std::{
    env,
    fs::File,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Copy, PartialEq)]
enum Screen {
    Library,
    Recent,
    Settings,
}

#[derive(Resource)]
struct Library {
    games: Vec<GameCard>,
    entries: Vec<usize>,
    selected: usize,
    screen: Screen,
    details: bool,
    preferences: Preferences,
    preferences_path: Option<PathBuf>,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)]
struct Preferences {
    recent_first: bool,
    motion: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            recent_first: false,
            motion: true,
        }
    }
}

impl Library {
    fn browse(&mut self) {
        self.entries = (0..self.games.len())
            .filter(|&index| {
                self.screen != Screen::Recent || self.games[index].last_played.is_some()
            })
            .collect();
        if self.screen == Screen::Recent || self.preferences.recent_first {
            self.entries
                .sort_by_key(|&index| std::cmp::Reverse(self.games[index].last_played));
        }
        self.selected = 0;
    }

    fn selected_game(&self) -> Option<&GameCard> {
        self.entries
            .get(self.selected)
            .map(|&index| &self.games[index])
    }

    fn save_preferences(&self) -> Result<()> {
        if let Some(path) = &self.preferences_path {
            let parent = path
                .parent()
                .context("Missing Station settings directory")?;
            std::fs::create_dir_all(parent)?;
            let mut file = tempfile::NamedTempFile::new_in(parent)?;
            serde_json::to_writer(file.as_file_mut(), &self.preferences)?;
            file.persist(path)?;
        }
        Ok(())
    }
}

#[derive(Resource, Clone, Copy, Default, PartialEq)]
struct Layout {
    columns: usize,
    rows: usize,
    cover_width: f32,
    cover_height: f32,
}

#[derive(Resource, Default)]
struct Selection(Arc<Mutex<Option<Reply>>>);

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let library = args.next().context("Start Station with drabi station")?;
    let reply = args.next().context("Missing Station reply path")?;
    let system = args.next().is_some_and(|arg| arg == "--system");
    let games = serde_json::from_reader(File::open(library)?)?;
    let preferences_path = env::var_os("DRABI_STATION_PREFERENCES").map(PathBuf::from);
    let preferences = preferences_path
        .as_ref()
        .and_then(|path| serde_json::from_reader(File::open(path).ok()?).ok())
        .unwrap_or_default();
    let mut library = Library {
        games,
        entries: Vec::new(),
        selected: 0,
        screen: Screen::Library,
        details: false,
        preferences,
        preferences_path,
    };
    library.browse();

    let selection = Selection::default();
    let selected = Arc::clone(&selection.0);
    let mut app = App::new();
    app.insert_resource(library)
        .insert_resource(selection)
        .init_resource::<Layout>()
        .init_resource::<hardware_settings::State>()
        .init_resource::<hardware_settings::Jobs>()
        .init_resource::<textures::Cache>()
        .insert_resource(WinitSettings {
            focused_mode: UpdateMode::reactive(Duration::from_millis(32)),
            unfocused_mode: UpdateMode::reactive_low_power(Duration::from_millis(250)),
        })
        .insert_resource(ClearColor(Color::srgb_u8(9, 15, 25)))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Drabi Station".into(),
                mode: WindowMode::BorderlessFullscreen(MonitorSelection::Primary),
                ..default()
            }),
            close_when_requested: false,
            ..default()
        }))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                measure,
                navigate,
                hardware_settings::tick,
                screens::render.run_if(
                    resource_changed::<Library>
                        .or(resource_changed::<Layout>)
                        .or(resource_changed::<hardware_settings::State>),
                ),
                screens::update_focus
                    .run_if(resource_changed::<Library>.or(resource_changed::<Layout>)),
                textures::update,
            )
                .chain(),
        );
    if system {
        app.init_resource::<system_menu::SystemMenu>().add_systems(
            Update,
            system_menu::render
                .after(navigate)
                .run_if(resource_changed::<system_menu::SystemMenu>),
        );
    }
    match app.run() {
        AppExit::Success => {
            let response = selected.lock().unwrap().take().unwrap_or(Reply::Exit);
            serde_json::to_writer(File::create(reply)?, &response)?;
            Ok(())
        }
        AppExit::Error(code) => anyhow::bail!("Station rendering exited with code {code}"),
    }
}

fn setup(mut commands: Commands, system: Option<Res<system_menu::SystemMenu>>) {
    commands.spawn(Camera2d);
    commands.spawn((screens::Root, screens::fullscreen()));
    if system.is_some() {
        commands.spawn((system_menu::Overlay, screens::fullscreen()));
    }
}

fn measure(window: Single<&Window, With<PrimaryWindow>>, mut layout: ResMut<Layout>) {
    let width = window.width() * 0.9;
    let height = window.height();
    let columns = (width / 180.).floor().clamp(1., 7.) as usize;
    let rows = if height >= 860. { 2 } else { 1 };
    let cover_height = ((height - 210.) / rows as f32 - 54.).clamp(100., 320.);
    let cover_width =
        (cover_height * 2. / 3.).min((width - (columns - 1) as f32 * 16.) / columns as f32);
    let next = Layout {
        columns,
        rows,
        cover_width,
        cover_height: cover_width * 1.5,
    };
    layout.set_if_neq(next);
}

fn label(parent: &mut ChildSpawnerCommands, text: impl Into<String>, size: f32, color: Color) {
    parent.spawn((
        Text::new(text),
        TextFont::from_font_size(size),
        TextColor(color),
    ));
}

fn navigate(
    (keyboard, controllers): (Res<ButtonInput<KeyCode>>, Query<&Gamepad>),
    mut close: MessageReader<WindowCloseRequested>,
    (layout, mut hardware, mut jobs): (
        Res<Layout>,
        ResMut<hardware_settings::State>,
        ResMut<hardware_settings::Jobs>,
    ),
    mut library: ResMut<Library>,
    selection: Res<Selection>,
    menu: Option<ResMut<system_menu::SystemMenu>>,
    mut exit: MessageWriter<AppExit>,
) {
    use system_menu::{Input, SystemMenu};
    let pressed = |button| controllers.iter().any(|pad| pad.just_pressed(button));
    if close.read().next().is_some() {
        exit.write(AppExit::Success);
        return;
    }
    let input = if keyboard.just_pressed(KeyCode::Escape) || pressed(GamepadButton::East) {
        Some(Input::Back)
    } else if menu.is_some()
        && (keyboard.just_pressed(KeyCode::F1) || pressed(GamepadButton::Start))
    {
        Some(Input::Open)
    } else if keyboard.just_pressed(KeyCode::ArrowUp) || pressed(GamepadButton::DPadUp) {
        Some(Input::Up)
    } else if keyboard.just_pressed(KeyCode::ArrowDown) || pressed(GamepadButton::DPadDown) {
        Some(Input::Down)
    } else if keyboard.just_pressed(KeyCode::Enter) || pressed(GamepadButton::South) {
        Some(Input::Accept)
    } else {
        None
    };
    if let Some(mut menu) = menu
        && (*menu != SystemMenu::Closed || matches!(input, Some(Input::Open)))
    {
        if let Some(input) = input
            && let Some(action) = menu.handle(input)
        {
            *selection.0.lock().unwrap() = Some(Reply::System(action));
            exit.write(AppExit::Success);
        }
        return;
    }
    if library.details {
        match input {
            Some(Input::Back) => library.details = false,
            Some(Input::Accept) if let Some(game) = library.selected_game() => {
                *selection.0.lock().unwrap() = Some(Reply::Select(game.id.clone()));
                exit.write(AppExit::Success);
            }
            _ => {}
        }
        return;
    }
    let tab = if keyboard.just_pressed(KeyCode::KeyQ) || pressed(GamepadButton::LeftTrigger) {
        Some(2)
    } else if keyboard.just_pressed(KeyCode::KeyE) || pressed(GamepadButton::RightTrigger) {
        Some(1)
    } else {
        None
    };
    if let Some(step) = tab {
        let tabs = [Screen::Library, Screen::Recent, Screen::Settings];
        let index = tabs
            .iter()
            .position(|&item| item == library.screen)
            .unwrap();
        library.screen = tabs[(index + step) % tabs.len()];
        library.browse();
        return;
    }
    if matches!(input, Some(Input::Back)) {
        if library.screen == Screen::Library {
            exit.write(AppExit::Success);
        } else {
            library.screen = Screen::Library;
            library.browse();
        }
        return;
    }
    let left = keyboard.just_pressed(KeyCode::ArrowLeft) || pressed(GamepadButton::DPadLeft);
    let right = keyboard.just_pressed(KeyCode::ArrowRight) || pressed(GamepadButton::DPadRight);
    if library.screen == Screen::Settings {
        use hardware_settings::*;
        let direction = i16::from(right) - i16::from(left);
        match input {
            Some(Input::Up) => library.selected = (library.selected + MOTION) % (MOTION + 1),
            Some(Input::Down) => library.selected = (library.selected + 1) % (MOTION + 1),
            Some(Input::Accept) if library.selected == REFRESH => jobs.refresh(),
            Some(Input::Accept) if library.selected == SORT || library.selected == MOTION => {
                if library.selected == SORT {
                    library.preferences.recent_first = !library.preferences.recent_first;
                } else {
                    library.preferences.motion = !library.preferences.motion;
                }
                if let Err(error) = library.save_preferences() {
                    eprintln!("warning: saving Station settings: {error:#}");
                }
            }
            _ => match library.selected {
                DEVICE if direction != 0 => hardware.cycle(direction),
                BRIGHTNESS if direction != 0 => {
                    if let Some(display) = hardware.display() {
                        jobs.adjust_brightness(display, direction);
                    }
                }
                VOLUME if direction != 0 => {
                    if let Some(Ok(audio)) = hardware.audio.as_ref() {
                        jobs.adjust_volume(audio, direction);
                    }
                }
                MUTE if matches!(input, Some(Input::Accept)) => {
                    if let Some(Ok(audio)) = hardware.audio.as_ref() {
                        jobs.toggle_mute(audio);
                    }
                }
                _ => {}
            },
        }
        return;
    }
    let count = library.entries.len();
    if count == 0 {
        return;
    }
    let selected = library.selected;
    if left {
        library.selected = (selected + count - 1) % count;
    } else if right {
        library.selected = (selected + 1) % count;
    } else {
        match input {
            Some(Input::Up) => library.selected = selected.saturating_sub(layout.columns),
            Some(Input::Down) => library.selected = (selected + layout.columns).min(count - 1),
            Some(Input::Accept) => library.details = true,
            _ => {}
        }
    }
}
