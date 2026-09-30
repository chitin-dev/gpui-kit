use std::hash::{DefaultHasher, Hash, Hasher};

use gpui::{App, ElementId, Entity, Path, Pixels, Point, Window};

/// A tessellated path reused across frames while its shape and position are
/// unchanged.
///
/// A chart repaints on every frame it is on screen, and tessellating its
/// strokes (Catmull-Rom curves, dashes) is the bulk of that work. A plot keeps
/// one cache per shape and paints through
/// [`Line::paint_cached`](super::shape::Line::paint_cached) or
/// [`Area::paint_cached`](super::shape::Area::paint_cached): `build` is handed
/// the frame's origin and returns the path placed there, and the path is kept
/// until the shape key or the origin changes.
///
/// WGPUI is why `build` takes the origin instead of the cache moving a
/// finished path to it: `Path` keeps its bounds and its vertices private, and
/// `Window::paint_path` takes no offset, so a path built at one origin cannot
/// be moved to another. A plot whose origin does not move therefore reuses its
/// tessellation, while one that scrolls rebuilds at its new origin.
#[derive(Default)]
pub struct PathCache {
    key: Option<u64>,
    /// The origin the cached path was built for.
    origin: Point<Pixels>,
    path: Option<Path<Pixels>>,
}

impl PathCache {
    /// The path for `key` at `origin`. `build` runs only when the key or the
    /// origin differs from the last call's, and must place the path it returns
    /// at the origin it is given.
    pub fn get(
        &mut self,
        key: u64,
        origin: Point<Pixels>,
        build: impl FnOnce(Point<Pixels>) -> Option<Path<Pixels>>,
    ) -> Option<Path<Pixels>> {
        if self.key != Some(key) || self.origin != origin {
            self.path = build(origin);
            self.key = Some(key);
            self.origin = origin;
        }
        self.path.clone()
    }

    /// Whether the last [`Self::get`] reused the path built by an earlier one.
    pub fn is_warm(&self) -> bool {
        self.key.is_some()
    }
}

/// The [`PathCache`]s of a plot that is rebuilt on every render, kept in the
/// window's element state so they outlive the plot value.
///
/// Plots are plain values built by `render` and painted once, so a cache
/// held by the plot would be empty every frame; this keeps them under the
/// element id the plot paints in (plus `key`), for as long as the plot is
/// painted on consecutive frames.
///
/// ```ignore
/// fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
///     let caches = PathCaches::for_paint("lines", window, cx);
///     caches.update(cx, |caches, _| {
///         for (ix, line) in lines.iter().enumerate() {
///             line.paint_cached(&bounds, caches.slot(ix), window);
///         }
///     });
/// }
/// ```
#[derive(Default)]
pub struct PathCaches {
    slots: Vec<PathCache>,
}

impl PathCaches {
    /// The caches for the plot painting under the window's current element
    /// id; `key` tells apart several groups of shapes in one plot.
    pub fn for_paint(key: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> Entity<Self> {
        window.use_keyed_state(key, cx, |_, _| Self::default())
    }

    /// The `index`-th cache, created on first use. Paint each shape through
    /// the same index every frame.
    pub fn slot(&mut self, index: usize) -> &mut PathCache {
        if self.slots.len() <= index {
            self.slots.resize_with(index + 1, PathCache::default);
        }
        &mut self.slots[index]
    }

    /// Two caches for a shape that keeps a fill and a stroke, such as
    /// [`Area::paint_cached`](super::shape::Area::paint_cached), at
    /// `2 * index` and `2 * index + 1`.
    pub fn slot_pair(&mut self, index: usize) -> (&mut PathCache, &mut PathCache) {
        let first = 2 * index;
        if self.slots.len() <= first + 1 {
            self.slots.resize_with(first + 2, PathCache::default);
        }
        let (head, tail) = self.slots.split_at_mut(first + 1);
        (&mut head[first], &mut tail[0])
    }
}

/// A shape key from its projected points (origin-relative) and whatever else
/// shapes the tessellation (stroke width, curve style, dash pattern).
pub struct ShapeKey(DefaultHasher);

impl ShapeKey {
    pub fn new(extra: impl Hash) -> Self {
        let mut hasher = DefaultHasher::new();
        extra.hash(&mut hasher);
        Self(hasher)
    }

    pub fn point(&mut self, point: Point<Pixels>) -> &mut Self {
        point.x.to_f32().to_bits().hash(&mut self.0);
        point.y.to_f32().to_bits().hash(&mut self.0);
        self
    }

    pub fn f32(&mut self, value: f32) -> &mut Self {
        value.to_bits().hash(&mut self.0);
        self
    }

    pub fn finish(&self) -> u64 {
        self.0.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{PathBuilder, point, px};

    /// A 10px diagonal from `origin`, the way a shape builds the path its
    /// cache holds.
    fn diagonal(origin: Point<Pixels>) -> Option<Path<Pixels>> {
        let mut builder = PathBuilder::stroke(px(2.));
        builder.move_to(origin);
        builder.line_to(point(origin.x + px(10.), origin.y + px(10.)));
        builder.build().ok()
    }

    #[test]
    fn builds_once_per_key_and_origin() {
        let mut cache = PathCache::default();
        let mut builds = 0;

        let first = cache
            .get(1, point(px(100.), px(50.)), |origin| {
                builds += 1;
                diagonal(origin)
            })
            .unwrap();
        // The same key at the same origin reuses the tessellation.
        let repeated = cache
            .get(1, point(px(100.), px(50.)), |origin| {
                builds += 1;
                diagonal(origin)
            })
            .unwrap();
        assert_eq!(builds, 1);
        // A `Path` exposes no geometry, so the paths are compared as a whole.
        assert_eq!(format!("{first:?}"), format!("{repeated:?}"));

        // A new origin places the shape elsewhere, so the cache builds again.
        cache
            .get(1, point(px(200.), px(50.)), |origin| {
                builds += 1;
                diagonal(origin)
            })
            .unwrap();
        assert_eq!(builds, 2);

        // So does a new key at an origin already seen.
        cache
            .get(2, point(px(200.), px(50.)), |origin| {
                builds += 1;
                diagonal(origin)
            })
            .unwrap();
        assert_eq!(builds, 3);

        // The cache holds one path, so the first origin is a miss again — and
        // rebuilding it there reproduces the path it held before.
        let rebuilt = cache
            .get(1, point(px(100.), px(50.)), |origin| {
                builds += 1;
                diagonal(origin)
            })
            .unwrap();
        assert_eq!(builds, 4);
        assert_eq!(format!("{first:?}"), format!("{rebuilt:?}"));
        assert!(cache.is_warm());
    }

    #[test]
    fn keys_follow_points_and_extras() {
        let a = ShapeKey::new(("linear", 1.0f32.to_bits()))
            .point(point(px(1.), px(2.)))
            .finish();
        let same = ShapeKey::new(("linear", 1.0f32.to_bits()))
            .point(point(px(1.), px(2.)))
            .finish();
        let moved = ShapeKey::new(("linear", 1.0f32.to_bits()))
            .point(point(px(1.), px(3.)))
            .finish();
        let thicker = ShapeKey::new(("linear", 2.0f32.to_bits()))
            .point(point(px(1.), px(2.)))
            .finish();
        assert_eq!(a, same);
        assert_ne!(a, moved);
        assert_ne!(a, thicker);
    }
}
