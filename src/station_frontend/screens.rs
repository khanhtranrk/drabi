use crate::{
    GameCard, Layout, Library, Screen, hardware_settings as hardware, label,
    system_menu::SystemMenu,
    textures::{Artwork, Kind},
};
use bevy::{prelude::*, ui::widget::NodeImageMode};

const TEXT: Color = Color::srgb(0.95, 0.97, 1.);
const MUTED: Color = Color::srgb(0.64, 0.71, 0.81);
const ACCENT: Color = Color::srgb(0.59, 0.93, 0.80);
const INK: Color = Color::srgb(0.04, 0.09, 0.12);
const PANEL: Color = Color::srgb(0.09, 0.14, 0.21);

#[derive(Component)]
pub struct Root;

#[derive(PartialEq)]
pub enum View {
    Grid {
        screen: Screen,
        start: usize,
        layout: Layout,
    },
    Details(usize),
    Settings,
}

#[derive(Component)]
pub enum Focus {
    Title,
    Position,
    Tile(usize),
}

fn page_start(library: &Library, layout: &Layout) -> usize {
    let size = (layout.columns * layout.rows).max(1);
    library.selected / size * size
}

pub fn fullscreen() -> Node {
    Node {
        position_type: PositionType::Absolute,
        width: percent(100),
        height: percent(100),
        ..default()
    }
}

pub fn render(
    mut commands: Commands,
    library: Res<Library>,
    layout: Res<Layout>,
    root: Single<Entity, With<Root>>,
    system: Option<Res<SystemMenu>>,
    devices: Res<hardware::State>,
    mut current: Local<Option<View>>,
) {
    let view = if library.details {
        View::Details(library.selected)
    } else if library.screen == Screen::Settings {
        View::Settings
    } else {
        View::Grid {
            screen: library.screen,
            start: page_start(&library, &layout),
            layout: *layout,
        }
    };
    // Keep artwork entities (and their current opacity) through focus changes.
    // Settings still redraw when a setting changes or a different row is selected.
    if current.as_ref() == Some(&view) && !matches!(view, View::Settings) {
        return;
    }
    *current = Some(view);
    commands.entity(*root).despawn_children().with_children(|root| {
        if library.details && let Some(game) = library.selected_game() {
            details(root, game);
            return;
        }
        root.spawn(Node {
            width: percent(100),
            height: percent(100),
            padding: UiRect::axes(percent(5), percent(4)),
            flex_direction: FlexDirection::Column,
            row_gap: px(16),
            overflow: Overflow::clip(),
            ..default()
        }).with_children(|page| {
            page.spawn(Node {
                align_items: AlignItems::Center,
                column_gap: px(36),
                ..default()
            }).with_children(|bar| {
                label(bar, "Drabi Station", 30., TEXT);
                for (screen, name) in [(Screen::Library, "Library"), (Screen::Recent, "Recently played"), (Screen::Settings, "Settings")] {
                    if library.screen == screen {
                        pill(bar, name);
                    } else {
                        label(bar, name, 18., MUTED);
                    }
                }
            });
            if library.screen == Screen::Settings {
                settings(page, &library, &devices);
            } else {
                shelf(page, &library, &layout);
            }
            let footer = if library.screen == Screen::Settings {
                "Up / Down  Select     Left / Right  Adjust     Enter / A  Activate     Q / E or LB / RB  Tabs     Esc / B  Back"
            } else {
                "Arrows / D-pad  Browse     Enter / A  Details     Q / E or LB / RB  Tabs     Esc / B  Back"
            };
            label(page, format!("{footer}{}", if system.is_some() { "     F1 / Start  System" } else { "" }), 15., MUTED);
        });
    });
}

fn pill(parent: &mut ChildSpawnerCommands, text: &str) {
    parent
        .spawn((
            Node {
                padding: UiRect::axes(px(16), px(8)),
                ..default()
            },
            BackgroundColor(ACCENT),
            BorderRadius::all(px(8)),
        ))
        .with_children(|item| label(item, text, 19., INK));
}

fn shelf(parent: &mut ChildSpawnerCommands, library: &Library, layout: &Layout) {
    let page_size = (layout.columns * layout.rows).max(1);
    let start = page_start(library, layout);
    parent
        .spawn(Node {
            flex_grow: 1.,
            flex_direction: FlexDirection::Column,
            row_gap: px(12),
            ..default()
        })
        .with_children(|body| {
            if library.entries.is_empty() {
                let (title, message) = if library.screen == Screen::Recent {
                    ("No recent games", "Play a registered game to see it here.")
                } else {
                    (
                        "Your library is empty",
                        "Register a game with drabi game add or drabi game install.",
                    )
                };
                label(body, title, 34., TEXT);
                label(body, message, 18., MUTED);
                return;
            }
            body.spawn((
                Focus::Title,
                Text::default(),
                TextFont::from_font_size(36.),
                TextColor(TEXT),
            ));
            let page = &library.entries[start..library.entries.len().min(start + page_size)];
            for (row, games) in page.chunks(layout.columns.max(1)).enumerate() {
                body.spawn(Node {
                    column_gap: px(16),
                    ..default()
                })
                .with_children(|line| {
                    for (column, &index) in games.iter().enumerate() {
                        tile(
                            line,
                            &library.games[index],
                            layout,
                            start + row * layout.columns + column,
                        );
                    }
                });
            }
            body.spawn((
                Focus::Position,
                Text::default(),
                TextFont::from_font_size(16.),
                TextColor(MUTED),
            ));
        });
}

fn tile(parent: &mut ChildSpawnerCommands, game: &GameCard, layout: &Layout, index: usize) {
    parent
        .spawn((Node {
            width: px(layout.cover_width),
            flex_direction: FlexDirection::Column,
            flex_shrink: 0.,
            row_gap: px(6),
            ..default()
        },))
        .with_children(|tile| {
            tile.spawn((
                Node {
                    height: px(layout.cover_height),
                    width: percent(100),
                    border: UiRect::all(px(3)),
                    padding: UiRect::all(px(3)),
                    ..default()
                },
                Focus::Tile(index),
                BorderColor::all(Color::NONE),
                BorderRadius::all(px(12)),
            ))
            .with_children(|frame| {
                frame
                    .spawn((
                        Node {
                            width: percent(100),
                            height: percent(100),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        BackgroundColor(PANEL),
                        BorderRadius::all(px(8)),
                    ))
                    .with_children(|cover| {
                        let initials: String = game
                            .name
                            .split_whitespace()
                            .filter_map(|word| word.chars().next())
                            .take(2)
                            .flat_map(char::to_uppercase)
                            .collect();
                        label(cover, initials, 44., MUTED);
                        if let Some(path) = &game.grid {
                            cover.spawn((
                                Artwork(path.clone(), Kind::Cover),
                                full_image(),
                                fullscreen(),
                            ));
                        } else if let Some(path) = &game.icon {
                            cover.spawn((
                                Artwork(path.clone(), Kind::Icon),
                                ImageNode::default(),
                                Node {
                                    position_type: PositionType::Absolute,
                                    ..default()
                                },
                            ));
                        }
                    });
            });
            tile.spawn(Node {
                width: percent(100),
                height: px(24),
                overflow: Overflow::clip(),
                ..default()
            })
            .with_children(|caption| {
                caption.spawn((
                    Focus::Tile(index),
                    Text::new(&game.name),
                    TextFont::from_font_size(17.),
                    TextColor(TEXT),
                ));
            });
        });
}

type FocusElements<'a> = (
    &'a Focus,
    Option<&'a mut Text>,
    Option<&'a mut BorderColor>,
    Option<&'a mut TextColor>,
);

pub fn update_focus(library: Res<Library>, mut elements: Query<FocusElements>) {
    for (focus, text, border, color) in &mut elements {
        match focus {
            Focus::Title => {
                if let (Some(mut text), Some(game)) = (text, library.selected_game()) {
                    text.0.clone_from(&game.name);
                }
            }
            Focus::Position => {
                if let Some(mut text) = text {
                    text.0 = format!("{} / {}", library.selected + 1, library.entries.len());
                }
            }
            Focus::Tile(index) => {
                let selected = *index == library.selected;
                if let Some(mut border) = border {
                    *border = BorderColor::all(if selected { ACCENT } else { Color::NONE });
                }
                if let Some(mut color) = color {
                    color.0 = if selected { ACCENT } else { TEXT };
                }
            }
        }
    }
}

fn full_image() -> ImageNode {
    ImageNode {
        image_mode: NodeImageMode::Stretch,
        ..default()
    }
}

fn details(parent: &mut ChildSpawnerCommands, game: &GameCard) {
    if let Some(path) = &game.hero {
        parent.spawn((
            Artwork(path.clone(), Kind::Hero),
            full_image(),
            fullscreen(),
        ));
        parent.spawn((
            fullscreen(),
            BackgroundColor(Color::srgba(0.03, 0.05, 0.08, 0.64)),
        ));
    }
    parent
        .spawn(Node {
            width: percent(100),
            height: percent(100),
            padding: UiRect::axes(percent(6), percent(6)),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Start,
            justify_content: JustifyContent::End,
            row_gap: px(14),
            ..default()
        })
        .with_children(|body| {
            if let Some(path) = game.logo.as_ref().or(game.icon.as_ref()) {
                let kind = if game.logo.is_some() {
                    Kind::Logo
                } else {
                    Kind::Icon
                };
                body.spawn((
                    Artwork(path.clone(), kind),
                    ImageNode::default(),
                    Node::default(),
                ));
            }
            label(body, &game.name, 48., TEXT);
            let metadata: Vec<_> = [game.publisher.as_deref(), game.version.as_deref()]
                .into_iter()
                .flatten()
                .collect();
            if !metadata.is_empty() {
                label(body, metadata.join("  /  "), 20., TEXT);
            }
            if let Some(seconds) = game.playtime {
                label(
                    body,
                    format!("Played {} h {} min", seconds / 3600, seconds / 60 % 60),
                    18.,
                    MUTED,
                );
            }
            pill(body, "Play   Enter / A");
            label(body, "Esc / B  Back", 17., TEXT);
        });
}

fn settings(parent: &mut ChildSpawnerCommands, library: &Library, devices: &hardware::State) {
    let sort = if library.preferences.recent_first {
        "Recently played first"
    } else {
        "Name"
    };
    let motion = if library.preferences.motion {
        "On"
    } else {
        "Off"
    };
    parent
        .spawn(Node {
            flex_grow: 1.,
            min_height: px(0),
            flex_direction: FlexDirection::Column,
            row_gap: px(12),
            ..default()
        })
        .with_children(|body| {
            label(body, "Display & Audio", 30., TEXT);
            body.spawn(Node {
                column_gap: px(16),
                align_items: AlignItems::Start,
                ..default()
            })
            .with_children(|columns| {
                panel(columns, "Display", |panel| {
                    display_settings(panel, library.selected, devices)
                });
                panel(columns, "Audio", |panel| {
                    audio_settings(panel, library.selected, devices)
                });
            });
            setting_row(
                body,
                "Refresh devices",
                "",
                library.selected == hardware::REFRESH,
                None,
            );
            setting_row(
                body,
                "Library order",
                sort,
                library.selected == hardware::SORT,
                None,
            );
            setting_row(
                body,
                "Artwork transitions",
                motion,
                library.selected == hardware::MOTION,
                None,
            );
        });
}

fn display_settings(parent: &mut ChildSpawnerCommands, selected: usize, devices: &hardware::State) {
    let display = devices.display();
    let name = match (display, &devices.displays) {
        (Some(display), Some(all)) => {
            let position = format!("{}  ({}/{})", display.name, devices.selected + 1, all.len());
            if all.len() > 1 {
                format!("< {position} >")
            } else {
                position
            }
        }
        (_, Some(_)) => "No backlight device".into(),
        _ => "Checking devices…".into(),
    };
    setting_row(parent, "Device", &name, selected == hardware::DEVICE, None);
    let level = display
        .and_then(|display| display.level.as_ref().ok())
        .map(|level| level.percent);
    slider(
        parent,
        "Brightness",
        level,
        selected == hardware::BRIGHTNESS,
    );
    if let Some(display) = display {
        let error = devices
            .display_error
            .as_ref()
            .filter(|(id, _)| *id == display.id)
            .map(|(_, error)| error)
            .or_else(|| display.level.as_ref().err());
        if let Some(error) = error {
            diagnostic(parent, error);
        }
    }
}

fn audio_settings(parent: &mut ChildSpawnerCommands, selected: usize, devices: &hardware::State) {
    let audio = devices.audio.as_ref().and_then(|state| state.as_ref().ok());
    let name = audio.map(|audio| audio.name.as_str()).unwrap_or_else(|| {
        if devices.audio.is_some() {
            "Output unavailable"
        } else {
            "Checking output…"
        }
    });
    setting_row(parent, "Output", name, false, None);
    slider(
        parent,
        "Volume",
        audio.map(|audio| audio.volume),
        selected == hardware::VOLUME,
    );
    let mute = audio
        .map(|audio| if audio.muted { "On" } else { "Off" })
        .unwrap_or("—");
    setting_row(parent, "Mute", mute, selected == hardware::MUTE, None);
    if let Some(error) = devices
        .audio_error
        .as_ref()
        .or_else(|| devices.audio.as_ref()?.as_ref().err())
    {
        diagnostic(parent, error);
    }
}

fn panel(
    parent: &mut ChildSpawnerCommands,
    title: &str,
    build: impl FnOnce(&mut ChildSpawnerCommands),
) {
    parent
        .spawn((
            Node {
                flex_grow: 1.,
                flex_basis: px(0),
                min_width: px(0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(12)),
                row_gap: px(6),
                ..default()
            },
            BackgroundColor(PANEL),
            BorderRadius::all(px(10)),
        ))
        .with_children(|column| {
            label(column, title, 23., TEXT);
            build(column);
        });
}

fn slider(parent: &mut ChildSpawnerCommands, title: &str, value: Option<u16>, selected: bool) {
    let text = value
        .map(|value| format!("{value}%"))
        .unwrap_or_else(|| "—".into());
    setting_row(parent, title, &text, selected, value);
}

fn setting_row(
    parent: &mut ChildSpawnerCommands,
    title: &str,
    value: &str,
    selected: bool,
    slider: Option<u16>,
) {
    parent
        .spawn((
            Node {
                width: percent(100),
                padding: UiRect::axes(px(12), px(6)),
                flex_direction: FlexDirection::Column,
                row_gap: px(5),
                ..default()
            },
            BackgroundColor(if selected { ACCENT } else { PANEL }),
            BorderRadius::all(px(6)),
        ))
        .with_children(|row| {
            let color = if selected { INK } else { TEXT };
            row.spawn(Node {
                width: percent(100),
                justify_content: JustifyContent::SpaceBetween,
                column_gap: px(12),
                ..default()
            })
            .with_children(|caption| {
                label(caption, title, 17., color);
                label(caption, value, 17., color);
            });
            if let Some(value) = slider {
                row.spawn((
                    Node {
                        width: percent(100),
                        height: px(7),
                        ..default()
                    },
                    BackgroundColor(if selected {
                        Color::srgb(0.40, 0.71, 0.62)
                    } else {
                        INK
                    }),
                    BorderRadius::all(px(4)),
                ))
                .with_children(|track| {
                    track.spawn((
                        Node {
                            width: percent(value.min(100)),
                            height: percent(100),
                            ..default()
                        },
                        BackgroundColor(if selected { INK } else { ACCENT }),
                        BorderRadius::all(px(4)),
                    ));
                });
            }
        });
}

fn diagnostic(parent: &mut ChildSpawnerCommands, text: &str) {
    let line = text.lines().next().unwrap_or_default();
    let mut message: String = line.chars().take(140).collect();
    if line.chars().count() > 140 {
        message.push('…');
    }
    label(parent, message, 15., MUTED);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_focus_on_a_page_keeps_artwork_entities() {
        let games: Vec<GameCard> = serde_json::from_value(serde_json::json!([
            {"id": "first", "name": "First", "grid": "first.jpg"},
            {"id": "second", "name": "Second", "grid": "second.jpg"}
        ]))
        .unwrap();
        let mut app = App::new();
        let mut library = Library {
            games,
            entries: Vec::new(),
            selected: 0,
            screen: Screen::Library,
            details: false,
            preferences: Default::default(),
            preferences_path: None,
        };
        library.browse();
        app.insert_resource(library)
            .init_resource::<hardware::State>()
            .insert_resource(Layout {
                columns: 2,
                rows: 1,
                cover_width: 160.,
                cover_height: 240.,
            })
            .add_systems(Update, (render, update_focus).chain());
        app.world_mut().spawn((Root, fullscreen()));
        app.update();
        let world = app.world_mut();
        let mut artwork = world.query_filtered::<Entity, With<Artwork>>();
        let before: Vec<_> = artwork.iter(world).collect();
        assert_eq!(before.len(), 2);

        world.resource_mut::<Library>().selected = 1;
        app.update();
        let world = app.world_mut();
        assert_eq!(artwork.iter(world).collect::<Vec<_>>(), before);
        let mut focus = world.query::<(&Focus, &BorderColor)>();
        assert!(focus.iter(world).any(|(marker, border)| {
            matches!(marker, Focus::Tile(1)) && *border == BorderColor::all(ACCENT)
        }));
    }
}
