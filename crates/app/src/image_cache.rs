//! Artwork loading for the library: display-sized and size-limited.
//!
//! GPUI's default for `img()` decodes every image at full resolution and keeps it (and its GPU
//! texture) for the life of the app. Local fanart is often 4K (33 MB once decoded), so a few
//! detail pages added hundreds of megabytes, and evicting them only meant decoding them again.
//!
//! Instead, artwork is requested with [`art_image`] and the width it is shown at. The first
//! load scales it down and saves the copy under `artwork/scaled`, so later loads decode a small
//! file. Decoded images are kept while recently drawn; past a budget, the ones not drawn for a
//! while are dropped from RAM and from the GPU atlas.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    App, AppContext as _, Entity, EntityId, ImageCache, ImageCacheError, Img, RenderImage,
    Resource, SharedString, SharedUri, WeakEntity, Window, img,
};

/// Decoded images kept beyond what is on screen.
const BUDGET_BYTES: usize = 96 * 1024 * 1024;
/// Images drawn this recently are never evicted. Long enough to cover a full turn of the
/// home hero (5 backdrops, 9 s each), so its backdrops are not decoded again on every rotation.
const KEEP_RECENT: Duration = Duration::from_secs(60);
/// How often eviction runs; it walks every entry, so not on every image load.
const EVICT_INTERVAL: Duration = Duration::from_millis(500);
const SCHEME: &str = "miru-art://";
/// Images decoded at the same time (see `BoundedImageCache::slots`).
const DECODE_SLOTS: usize = 2;

/// An image of the artwork at `path`, decoded no wider than `max_width` pixels (twice the
/// layout width, for high-DPI screens). Must be drawn inside a [`BoundedImageCache`].
pub fn art_image(path: impl AsRef<str>, max_width: u32) -> Img {
    img(SharedUri::from(format!(
        "{SCHEME}{max_width}/{}",
        path.as_ref()
    )))
}

/// Like [`art_image`], cropped to a centred square first (biased towards the top, where faces
/// are in portraits), for round avatars: a rounded image is clipped to its own bounds, so a
/// portrait in a circle frame would show as an arch.
pub fn art_image_square(path: impl AsRef<str>, size: u32) -> Img {
    img(SharedUri::from(format!(
        "{SCHEME}{size}s/{}",
        path.as_ref()
    )))
}

/// How artwork is decoded: no wider than `max_width`, optionally cropped square.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Fit {
    max_width: u32,
    square: bool,
}

impl Fit {
    fn width(max_width: u32) -> Self {
        Self {
            max_width,
            square: false,
        }
    }
}

fn parse(resource: &Resource) -> Option<(PathBuf, Fit)> {
    match resource {
        Resource::Uri(uri) => {
            let (width, path) = uri.as_ref().strip_prefix(SCHEME)?.split_once('/')?;
            let (width, square) = match width.strip_suffix('s') {
                Some(width) => (width, true),
                None => (width, false),
            };
            let max_width = width.parse().ok()?;
            Some((PathBuf::from(path), Fit { max_width, square }))
        }
        Resource::Path(path) => Some((path.to_path_buf(), Fit::width(u32::MAX))),
        Resource::Embedded(_) => None,
    }
}

enum State {
    Loading { waiters: Vec<EntityId> },
    Ready(Arc<RenderImage>),
    Failed(SharedString),
}

struct Entry {
    state: State,
    last_used: Instant,
}

pub struct BoundedImageCache {
    entries: HashMap<(PathBuf, Fit), Entry>,
    scaled_dir: PathBuf,
    this: WeakEntity<Self>,
    /// Decode slots: a load takes one before decoding and returns it after, so only
    /// `DECODE_SLOTS` images decode at once instead of one per core in a burst.
    slots: (async_channel::Sender<()>, async_channel::Receiver<()>),
    last_evict: Instant,
}

impl BoundedImageCache {
    /// `artwork_dir` is the artwork cache; scaled copies go in a `scaled` folder inside it.
    pub fn new(artwork_dir: &Path, cx: &mut App) -> Entity<Self> {
        let scaled_dir = artwork_dir.join("scaled");
        let cache = cx.new(|cx| Self {
            entries: HashMap::new(),
            scaled_dir,
            this: cx.weak_entity(),
            slots: {
                let slots = async_channel::bounded(DECODE_SLOTS);
                for _ in 0..DECODE_SLOTS {
                    let _ = slots.0.try_send(());
                }
                slots
            },
            last_evict: Instant::now(),
        });
        // Free the textures too if the cache itself goes away.
        cx.observe_release(&cache, |cache, cx| {
            for (_, entry) in cache.entries.drain() {
                if let State::Ready(image) = entry.state {
                    cx.drop_image(image, None);
                }
            }
        })
        .detach();
        cache
    }

    /// Total decoded size of the loaded images, and how many there are.
    pub fn usage(&self) -> (usize, usize) {
        self.entries
            .values()
            .filter_map(|e| match &e.state {
                State::Ready(image) => Some(image_bytes(image)),
                _ => None,
            })
            .fold((0, 0), |(bytes, n), b| (bytes + b, n + 1))
    }

    fn start_load(&self, path: PathBuf, fit: Fit, cx: &mut App) {
        let (this, scaled_dir) = (self.this.clone(), self.scaled_dir.clone());
        let (give_back, take) = self.slots.clone();
        cx.spawn(async move |cx| {
            let key = (path.clone(), fit);
            // Wait for a free slot; the channel only closes if the cache is gone.
            if take.recv().await.is_err() {
                return;
            }
            let decoded = cx
                .background_executor()
                .spawn(async move {
                    let started = Instant::now();
                    let result = decode(&path, fit, &scaled_dir);
                    if let Ok((_, how)) = &result {
                        tracing::debug!(
                            path = %path.display(),
                            how,
                            ms = started.elapsed().as_secs_f32() * 1000.,
                            "artwork decoded"
                        );
                    }
                    result.map(|(image, _)| image)
                })
                .await;
            let _ = give_back.try_send(());
            cx.update(|cx| {
                let waiters = this
                    .update(cx, |cache, _| {
                        let entry = cache.entries.get_mut(&key)?;
                        let state = match decoded {
                            Ok(image) => State::Ready(Arc::new(image)),
                            Err(e) => {
                                tracing::warn!(path = %key.0.display(), "artwork failed: {e:#}");
                                State::Failed(e.to_string().into())
                            }
                        };
                        match std::mem::replace(&mut entry.state, state) {
                            State::Loading { waiters } => Some(waiters),
                            _ => None,
                        }
                    })
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                for view in waiters {
                    cx.notify(view);
                }
            });
        })
        .detach();
    }

    fn evict(&mut self, window: &mut Window, cx: &mut App) {
        self.last_evict = Instant::now();
        let (mut total, _) = self.usage();
        if total <= BUDGET_BYTES {
            return;
        }
        let mut candidates: Vec<((PathBuf, Fit), Instant)> = self
            .entries
            .iter()
            .filter(|(_, e)| {
                matches!(e.state, State::Ready(_)) && e.last_used.elapsed() >= KEEP_RECENT
            })
            .map(|(key, e)| (key.clone(), e.last_used))
            .collect();
        candidates.sort_by_key(|(_, used)| *used);
        for (key, _) in candidates {
            if total <= BUDGET_BYTES {
                break;
            }
            if let Some(Entry {
                state: State::Ready(image),
                ..
            }) = self.entries.remove(&key)
            {
                total = total.saturating_sub(image_bytes(&image));
                cx.drop_image(image, Some(window));
            }
        }
    }
}

impl ImageCache for BoundedImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let Some((path, fit)) = parse(resource) else {
            return Some(Err(ImageCacheError::Asset(
                "unsupported image source".into(),
            )));
        };
        let view = window.current_view();
        let key = (path, fit);
        let result = match self.entries.get_mut(&key) {
            Some(entry) => {
                entry.last_used = Instant::now();
                match &mut entry.state {
                    State::Ready(image) => Some(Ok(image.clone())),
                    State::Failed(e) => Some(Err(ImageCacheError::Asset(e.clone()))),
                    State::Loading { waiters } => {
                        if !waiters.contains(&view) {
                            waiters.push(view);
                        }
                        None
                    }
                }
            }
            None => {
                self.entries.insert(
                    key.clone(),
                    Entry {
                        state: State::Loading {
                            waiters: vec![view],
                        },
                        last_used: Instant::now(),
                    },
                );
                self.start_load(key.0, key.1, cx);
                None
            }
        };
        if self.last_evict.elapsed() >= EVICT_INTERVAL {
            self.evict(window, cx);
        }
        result
    }
}

/// Loads `path` as `fit` asks (no wider than its width, cropped square if set), from the copy when one was saved before.
/// Wider originals are scaled down once and the copy saved (PNG when the image has
/// transparency, as logos do; JPEG otherwise).
/// Also says which way it went ("copy", "original" or "scaled now"), for the debug log.
fn decode(path: &Path, fit: Fit, scaled_dir: &Path) -> anyhow::Result<(RenderImage, &'static str)> {
    let max_width = fit.max_width;
    let modified = std::fs::metadata(path)?.modified().ok();
    let mut hasher = DefaultHasher::new();
    (path, modified, fit).hash(&mut hasher);
    let stem = format!("{:016x}", hasher.finish());
    for ext in ["jpg", "png"] {
        let copy = scaled_dir.join(format!("{stem}.{ext}"));
        if copy.is_file() {
            match image::open(&copy) {
                Ok(image) => return Ok((to_render_image(image), "copy")),
                // A damaged copy is rebuilt from the original below.
                Err(e) => tracing::debug!(path = %copy.display(), "unreadable scaled copy: {e}"),
            }
        }
    }

    let mut original = image::open(path)?;
    if fit.square {
        original = crop_square(original);
    }
    if original.width() <= max_width && !fit.square {
        return Ok((to_render_image(original), "original"));
    }
    let max_width = max_width.min(original.width());
    let height = (u64::from(original.height()) * u64::from(max_width) / u64::from(original.width()))
        .max(1) as u32;
    // `thumbnail` averages pixels in one pass: several times faster than a filtered resize
    // for big reductions (4K fanart to backdrop width) and just as clean when shrinking.
    let scaled = original.thumbnail_exact(max_width, height);
    let (ext, saved) = if scaled.color().has_alpha() {
        ("png", scaled.clone())
    } else {
        ("jpg", image::DynamicImage::ImageRgb8(scaled.to_rgb8()))
    };
    let copy = scaled_dir.join(format!("{stem}.{ext}"));
    let save = std::fs::create_dir_all(scaled_dir)
        .map_err(anyhow::Error::from)
        .and_then(|()| {
            // Write via a temp file so a crash never leaves a truncated copy behind.
            let tmp = copy.with_extension("part");
            saved.save_with_format(
                &tmp,
                if ext == "png" {
                    image::ImageFormat::Png
                } else {
                    image::ImageFormat::Jpeg
                },
            )?;
            std::fs::rename(&tmp, &copy)?;
            Ok(())
        });
    if let Err(e) = save {
        tracing::warn!(path = %copy.display(), "cannot save scaled artwork: {e:#}");
    }
    Ok((to_render_image(scaled), "scaled now"))
}

/// The largest centred square, nudged towards the top of portraits (where faces are).
fn crop_square(image: image::DynamicImage) -> image::DynamicImage {
    let (w, h) = (image.width(), image.height());
    let side = w.min(h);
    let x = (w - side) / 2;
    let y = (h - side) / 5;
    image.crop_imm(x, y, side, side)
}

/// GPUI draws BGRA frames.
fn to_render_image(image: image::DynamicImage) -> RenderImage {
    let mut pixels = image.into_rgba8();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    RenderImage::new(vec![image::Frame::new(pixels)])
}

fn image_bytes(image: &RenderImage) -> usize {
    (0..image.frame_count())
        .map(|i| {
            let size = image.size(i);
            size.width.0.max(0) as usize * size.height.0.max(0) as usize * 4
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn art_uris_round_trip() {
        let parse_uri = |uri: String| parse(&Resource::Uri(SharedUri::from(uri)));
        let path = PathBuf::from("C:/art/movie-1-poster.jpg");
        assert_eq!(
            parse_uri(format!("{SCHEME}480/C:/art/movie-1-poster.jpg")),
            Some((path.clone(), Fit::width(480)))
        );
        assert_eq!(
            parse_uri(format!("{SCHEME}104s/C:/art/movie-1-poster.jpg")),
            Some((
                path,
                Fit {
                    max_width: 104,
                    square: true
                }
            ))
        );
    }

    #[test]
    fn portraits_are_cropped_square_for_avatars() {
        let dir = tempfile::tempdir().unwrap();
        let photo = dir.path().join("person-1.jpg");
        image::RgbImage::from_pixel(185, 278, image::Rgb([90, 90, 90]))
            .save(&photo)
            .unwrap();
        let fit = Fit {
            max_width: 104,
            square: true,
        };
        let (image, _) = decode(&photo, fit, &dir.path().join("scaled")).unwrap();
        assert_eq!((image.size(0).width.0, image.size(0).height.0), (104, 104));
    }

    #[test]
    fn wide_images_are_scaled_once_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("fanart.jpg");
        image::RgbImage::from_pixel(1000, 500, image::Rgb([200, 40, 40]))
            .save(&original)
            .unwrap();
        let scaled_dir = dir.path().join("scaled");

        let (first, how) = decode(&original, Fit::width(400), &scaled_dir).unwrap();
        assert_eq!(how, "scaled now");
        assert_eq!(first.size(0).width.0, 400);
        assert_eq!(first.size(0).height.0, 200);
        assert_eq!(std::fs::read_dir(&scaled_dir).unwrap().count(), 1);

        let (again, how) = decode(&original, Fit::width(400), &scaled_dir).unwrap();
        assert_eq!(how, "copy");
        assert_eq!(again.size(0).width.0, 400);
        // Narrow enough already: used as is, nothing saved.
        decode(&original, Fit::width(2000), &scaled_dir).unwrap();
        assert_eq!(std::fs::read_dir(&scaled_dir).unwrap().count(), 1);
    }
}
