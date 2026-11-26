use crate::{label, station_protocol::SystemAction};
use bevy::prelude::*;

const ACTIONS: [(SystemAction, &str); 3] = [
    (SystemAction::Logout, "Logout"),
    (SystemAction::Restart, "Restart"),
    (SystemAction::Shutdown, "Shutdown"),
];

#[derive(Resource, PartialEq, Default)]
pub enum SystemMenu {
    #[default]
    Closed,
    Actions(usize),
    Confirm {
        action: usize,
        confirmed: bool,
    },
}

pub enum Input {
    Open,
    Back,
    Up,
    Down,
    Accept,
}

impl SystemMenu {
    pub fn handle(&mut self, input: Input) -> Option<SystemAction> {
        use Input::*;
        use SystemMenu::*;
        *self = match (&*self, input) {
            (Closed, Open) => Actions(0),
            (Actions(_), Back | Open) => Closed,
            (Actions(index), Up) => Actions((index + ACTIONS.len() - 1) % ACTIONS.len()),
            (Actions(index), Down) => Actions((index + 1) % ACTIONS.len()),
            (Actions(0), Accept) => return Some(SystemAction::Logout),
            (Actions(index), Accept) => Confirm {
                action: *index,
                confirmed: false,
            },
            (Confirm { action, .. }, Back | Open)
            | (
                Confirm {
                    action,
                    confirmed: false,
                },
                Accept,
            ) => Actions(*action),
            (Confirm { action, confirmed }, Up | Down) => Confirm {
                action: *action,
                confirmed: !confirmed,
            },
            (
                Confirm {
                    action,
                    confirmed: true,
                },
                Accept,
            ) => return Some(ACTIONS[*action].0),
            (Closed, _) => Closed,
        };
        None
    }
}

#[derive(Component)]
pub struct Overlay;

pub fn render(
    mut commands: Commands,
    menu: Res<SystemMenu>,
    overlay: Single<Entity, With<Overlay>>,
) {
    commands
        .entity(*overlay)
        .despawn_children()
        .with_children(|root| {
            if *menu == SystemMenu::Closed {
                return;
            }
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                GlobalZIndex(1),
                BackgroundColor(Color::srgba(0., 0., 0., 0.75)),
            ))
            .with_children(|backdrop| {
                backdrop
                    .spawn((
                        Node {
                            width: percent(80),
                            max_width: px(460),
                            padding: UiRect::all(px(24)),
                            flex_direction: FlexDirection::Column,
                            row_gap: px(8),
                            ..default()
                        },
                        BackgroundColor(Color::srgb_u8(23, 32, 47)),
                        BorderRadius::all(px(16)),
                    ))
                    .with_children(|panel| {
                        match *menu {
                            SystemMenu::Closed => {}
                            SystemMenu::Actions(selected) => {
                                label(panel, "System", 30., Color::WHITE);
                                for (index, (_, name)) in ACTIONS.iter().enumerate() {
                                    row(panel, name, selected == index);
                                }
                            }
                            SystemMenu::Confirm { action, confirmed } => {
                                let name = ACTIONS[action].1;
                                label(panel, format!("{name} this system?"), 30., Color::WHITE);
                                row(panel, "Cancel", !confirmed);
                                row(panel, name, confirmed);
                            }
                        }
                        label(
                            panel,
                            "Enter / A   Select      Esc / B   Back",
                            17.,
                            Color::WHITE,
                        );
                    });
            });
        });
}

fn row(parent: &mut ChildSpawnerCommands, name: &str, selected: bool) {
    let (background, foreground) = if selected {
        (Color::srgb_u8(162, 237, 201), Color::srgb_u8(13, 24, 30))
    } else {
        (Color::srgb_u8(36, 47, 63), Color::WHITE)
    };
    parent
        .spawn((
            Node {
                padding: UiRect::axes(px(14), px(8)),
                ..default()
            },
            BackgroundColor(background),
            BorderRadius::all(px(8)),
        ))
        .with_children(|row| label(row, name, 22., foreground));
}

#[cfg(test)]
mod tests {
    use super::{Input::*, SystemAction, SystemMenu, SystemMenu::*};

    #[test]
    fn power_actions_require_moving_off_cancel_and_accepting() {
        for (direction, action, expected) in [
            (Down, 1, SystemAction::Restart),
            (Up, 2, SystemAction::Shutdown),
        ] {
            let mut menu = SystemMenu::default();
            assert!(menu.handle(Open).is_none());
            assert!(menu.handle(direction).is_none());
            assert!(menu.handle(Accept).is_none());
            assert!(matches!(
                menu,
                Confirm {
                    confirmed: false,
                    ..
                }
            ));
            assert!(menu.handle(Accept).is_none());
            assert!(matches!(menu, Actions(index) if index == action));

            assert!(menu.handle(Accept).is_none());
            assert!(menu.handle(Down).is_none());
            assert_eq!(menu.handle(Accept), Some(expected));
        }
    }
}
