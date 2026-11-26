use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
    window::PrimaryWindow,
};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use crate::{Layout, Library};

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub enum Kind {
    Cover,
    Hero,
    Logo,
    Icon,
}

#[derive(Component, Clone, Eq, Hash, PartialEq)]
pub struct Artwork(pub PathBuf, pub Kind);

enum Texture {
    Pending(Task<Option<Image>>),
    Ready(Handle<Image>, Vec2),
    Invalid,
}

#[derive(Resource, Default)]
pub struct Cache(HashMap<Artwork, Texture>);

pub fn update(
    mut images: ResMut<Assets<Image>>,
    mut cache: ResMut<Cache>,
    mut slots: Query<(&Artwork, &ComputedNode, &mut ImageNode, &mut Node)>,
    library: Res<Library>,
    time: Res<Time>,
    layout: Res<Layout>,
    window: Single<&Window, With<PrimaryWindow>>,
) {
    let mut visible = HashSet::new();
    for (artwork, computed, mut node, mut style) in &mut slots {
        visible.insert(artwork.clone());
        let texture = cache.0.entry(artwork.clone()).or_insert_with(|| {
            let artwork = artwork.clone();
            Texture::Pending(AsyncComputeTaskPool::get().spawn(async move { load(&artwork) }))
        });
        if let Texture::Pending(task) = texture
            && let Some(result) = block_on(poll_once(task))
        {
            *texture = match result {
                Some(image) => {
                    let size = image.size().as_vec2();
                    Texture::Ready(images.add(image), size)
                }
                None => Texture::Invalid,
            };
        }
        if let Texture::Ready(handle, size) = texture {
            if node.image != *handle {
                node.image = handle.clone();
                node.color
                    .set_alpha(if library.preferences.motion { 0. } else { 1. });
            }
            if node.color.alpha() < 1. {
                let opacity = (node.color.alpha() + time.delta_secs() * 6.).min(1.);
                node.color.set_alpha(opacity);
            }
            if matches!(artwork.1, Kind::Hero | Kind::Cover) {
                let dest = computed.size();
                if dest.x > 0. && dest.y > 0. {
                    let scale = (dest.x / size.x).max(dest.y / size.y);
                    let half = (dest / scale) * 0.5;
                    let center = *size * 0.5;
                    let rect = Some(Rect::from_corners(center - half, center + half));
                    if node.rect != rect {
                        node.rect = rect;
                    }
                }
            } else {
                let bounds = match artwork.1 {
                    Kind::Logo => Vec2::new((window.width() * 0.38).min(360.), 122.),
                    _ => Vec2::new((layout.cover_width * 0.52).min(84.), 84.),
                };
                let scale = (bounds / *size).min_element().min(1.);
                let (width, height) = (px(size.x * scale), px(size.y * scale));
                if style.width != width || style.height != height {
                    style.width = width;
                    style.height = height;
                }
            }
        }
    }
    cache.0.retain(|artwork, _| visible.contains(artwork));
}

fn load(Artwork(path, kind): &Artwork) -> Option<Image> {
    let mut reader = image::ImageReader::open(Path::new(path))
        .ok()?
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    let (width, height) = match kind {
        Kind::Cover => (520, 780),
        Kind::Hero => (1600, 900),
        Kind::Logo => (600, 220),
        Kind::Icon => (256, 256),
    };
    Some(Image::from_dynamic(
        image.thumbnail(width, height),
        true,
        RenderAssetUsages::default(),
    ))
}
