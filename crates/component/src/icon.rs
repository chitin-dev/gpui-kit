use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash as _, Hasher as _},
    sync::Arc,
};

use crate::{ActiveTheme, Sizable, Size};
use gpui::{
    AnyElement, App, AppContext, Context, Entity, IntoElement, Pixels, Radians, Render, RenderOnce,
    SharedString, StyleRefinement, Styled, TextColor, Transformation, Window,
    prelude::FluentBuilder as _, svg,
};
use gpui_base::compat::svg_data;
pub use gpui_kit_assets::IconNamed;

// Preserve the original enum (including exhaustive matches and inherent view)
// while the complete, shared catalog is owned by gpui-kit-assets.
macro_rules! component_icon_names {
    ($($name:ident => $path:literal,)*) => {
        /// Default component icon names, retained for source compatibility.
        /// For the complete Lucide catalog, use `gpui_kit_assets::IconName`.
        #[derive(Clone, IntoElement)]
        pub enum IconName {
            $($name,)*
        }

        impl From<IconName> for gpui_kit_assets::IconName {
            fn from(name: IconName) -> Self {
                match name {
                    $(IconName::$name => Self::$name,)*
                }
            }
        }

        impl IconNamed for IconName {
            fn path(self) -> SharedString {
                match self { $(Self::$name => $path,)* }.into()
            }
        }
    };
}

gpui_kit_assets::__component_icon_names!(component_icon_names);

impl IconName {
    /// Return the icon as an Entity<Icon>.
    pub fn view(self, cx: &mut App) -> Entity<Icon> {
        Icon::build(self).view(cx)
    }
}

impl From<IconName> for AnyElement {
    fn from(name: IconName) -> Self {
        Icon::build(name).into_any_element()
    }
}

impl RenderOnce for IconName {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        Icon::build(self)
    }
}

impl<T: IconNamed> From<T> for Icon {
    fn from(value: T) -> Self {
        Icon::build(value)
    }
}

/// Component view construction for the shared `gpui_kit_assets::IconName`.
/// The legacy component `IconName` retains its inherent `view` method.
pub trait IconNameExt {
    fn view(self, cx: &mut App) -> Entity<Icon>;
}

impl IconNameExt for gpui_kit_assets::IconName {
    fn view(self, cx: &mut App) -> Entity<Icon> {
        Icon::build(self).view(cx)
    }
}

#[derive(Clone)]
pub(crate) enum IconSource {
    Path(SharedString),
    Data(Arc<[u8]>),
}

#[derive(Clone, IntoElement)]
pub struct Icon {
    style: StyleRefinement,
    source: IconSource,
    text_color: Option<TextColor>,
    size: Option<Size>,
    transformation: Option<Transformation>,
}

impl Default for Icon {
    fn default() -> Self {
        Self {
            style: StyleRefinement::default(),
            source: IconSource::Path("".into()),
            text_color: None,
            size: None,
            transformation: None,
        }
    }
}

impl Icon {
    pub fn new(icon: impl Into<Icon>) -> Self {
        icon.into()
    }

    fn build(name: impl IconNamed) -> Self {
        Self::default().path(name.path())
    }

    /// Set the icon path of the Assets bundle
    ///
    /// For example: `icons/foo.svg`
    /// Replaces any previously set path or SVG data.
    pub fn path(mut self, path: impl Into<SharedString>) -> Self {
        self.source = IconSource::Path(path.into());
        self
    }

    /// Set raw SVG bytes without registering an asset path.
    ///
    /// Copies the bytes into shared storage; the input need not be static.
    /// Cloning the icon shares those bytes. Replaces any previously set path or data.
    /// Parsing and rendering follow GPUI's SVG behavior, except that
    /// [`Icon::transform`] and [`Icon::rotate`] do not apply to a data icon:
    /// WGPUI can only transform an `Svg` built from an asset path.
    ///
    /// ```
    /// use gpui_component::Icon;
    ///
    /// let bytes = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
    ///     <path d="M4 12h16" stroke="currentColor"/>
    /// </svg>"#;
    /// let icon = Icon::default().data(bytes);
    /// ```
    pub fn data(mut self, data: &[u8]) -> Self {
        self.source = IconSource::Data(Arc::from(data));
        self
    }

    #[cfg(any(target_os = "macos", target_os = "windows", test))]
    pub(crate) fn source_ref(&self) -> &IconSource {
        &self.source
    }

    /// Whether `other` takes up exactly the same space as this icon. The
    /// source, color and transformation change only what it paints.
    pub(crate) fn same_layout(&self, other: &Self) -> bool {
        self.style == other.style && self.size == other.size
    }

    /// Create a new view for the icon
    pub fn view(self, cx: &mut App) -> Entity<Icon> {
        cx.new(|_| self)
    }

    /// Set the SVG transformation, replacing any previous transformation or rotation.
    ///
    /// Only an icon built from an asset path is transformed; see [`Icon::data`].
    pub fn transform(mut self, transformation: gpui::Transformation) -> Self {
        self.transformation = Some(transformation);
        self
    }

    pub fn empty() -> Self {
        Self::default()
    }

    /// Rotate the icon by the given angle
    ///
    /// Replaces any previous transformation or rotation. Only an icon built
    /// from an asset path rotates; see [`Icon::data`].
    pub fn rotate(mut self, radians: impl Into<Radians>) -> Self {
        self.transformation = Some(Transformation::rotate(radians));
        self
    }

    /// Lay `element` out as an icon with this `style` and `size`: the caller's
    /// refinement, the resolved text color, and the box the size variant asks
    /// for. Shared by the two sources below, which differ only in how they
    /// paint.
    fn lay_out<T: Styled>(
        mut element: T,
        style: &StyleRefinement,
        size: Option<Size>,
        text_size: Pixels,
        text_color: TextColor,
    ) -> T {
        let has_base_size = style.size.width.is_some() || style.size.height.is_some();
        *element.style() = style.clone();

        let element = element.flex_shrink_0().text_color(text_color);
        let element = if has_base_size {
            element
        } else {
            element.size(text_size)
        };
        match size {
            Some(Size::Size(px)) => element.size(px),
            Some(Size::XSmall) => element.size_3(),
            Some(Size::Small) => element.size_3p5(),
            Some(Size::Medium) => element.size_4(),
            Some(Size::Large) => element.size_6(),
            None => element,
        }
    }

    /// The sprite-atlas key standing in for the asset path a data icon does
    /// not have. See [`gpui_base::compat::svg_data`] for why one is needed.
    fn data_key(data: &[u8]) -> SharedString {
        let mut hasher = DefaultHasher::new();
        data.hash(&mut hasher);
        format!("icon-data:{:016x}", hasher.finish()).into()
    }

    fn into_element(self, text_size: Pixels, fallback_color: TextColor) -> AnyElement {
        let Icon {
            style,
            source,
            text_color,
            size,
            transformation,
        } = self;
        let text_color = text_color.unwrap_or(fallback_color);
        match source {
            IconSource::Path(path) => Self::lay_out(svg(), &style, size, text_size, text_color)
                .path(path)
                .when_some(transformation, |this, transformation| {
                    this.with_transformation(transformation)
                })
                .into_any_element(),
            // A data icon is painted from its bytes and has no path for a
            // transformation to be attached to, so `transformation` is not
            // read here; `Icon::data` says what that costs.
            IconSource::Data(data) => {
                let key = Self::data_key(&data);
                Self::lay_out(
                    svg_data(key, data, text_color.to_hsla()),
                    &style,
                    size,
                    text_size,
                    text_color,
                )
                .into_any_element()
            }
        }
    }
}
impl Styled for Icon {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }

    fn text_color(mut self, color: impl Into<TextColor>) -> Self {
        self.text_color = Some(color.into());
        self
    }
}

impl Sizable for Icon {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = Some(size.into());
        self
    }
}

impl RenderOnce for Icon {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let text_size = window.text_style().font_size.to_pixels(window.rem_size());
        self.into_element(text_size, window.text_style().color)
    }
}

impl From<Icon> for AnyElement {
    fn from(val: Icon) -> Self {
        val.into_any_element()
    }
}

impl Render for Icon {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text_size = window.text_style().font_size.to_pixels(window.rem_size());
        self.clone()
            .into_element(text_size, cx.theme().foreground.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{px, size};

    const SVG: &[u8] = include_bytes!("../../assets/assets/icons/arrow-up.svg");

    #[test]
    fn test_icon_builder_preserves_owned_data_and_transform_on_clone() {
        let transformation = Transformation::scale(size(0.5, 0.5))
            .with_rotation(gpui::radians(std::f32::consts::FRAC_PI_2));
        let icon = {
            let bytes = SVG.to_vec();
            Icon::default()
                .data(&bytes)
                .large()
                .text_color(gpui::red())
                .transform(transformation)
        };
        let cloned = icon.clone();
        let (IconSource::Data(original), IconSource::Data(copy)) = (&icon.source, &cloned.source)
        else {
            panic!("cloning must preserve the data source");
        };
        assert_eq!(copy.as_ref(), SVG);
        assert!(Arc::ptr_eq(original, copy));
        assert_eq!(cloned.transformation, Some(transformation));
        assert_eq!(cloned.size, icon.size);
        assert_eq!(cloned.text_color, icon.text_color);

        // The layout a data icon gets is the one an `Svg` shows here: both
        // sources are laid out by `lay_out` before they paint.
        let text_color = cloned.text_color.unwrap_or(gpui::blue().into());
        let mut svg = Icon::lay_out(svg(), &cloned.style, cloned.size, px(12.), text_color);
        assert_eq!(
            svg.style().text.as_ref().and_then(|text| text.color),
            Some(gpui::red().into())
        );
        assert_eq!(svg.style().size.width, Some(gpui::rems(1.5).into()));

        let rotated = icon.rotate(gpui::radians(std::f32::consts::PI)).clone();
        assert_eq!(
            rotated.transformation,
            Some(Transformation::rotate(gpui::radians(std::f32::consts::PI)))
        );
    }

    #[test]
    fn a_data_key_names_one_payload_at_every_size() {
        let key = Icon::data_key(SVG);
        assert_eq!(key, Icon::data_key(SVG));
        assert_ne!(key, Icon::data_key(b"<svg/>"));
    }

    #[test]
    fn test_icon_source_builders_replace_previous_source() {
        let icon = Icon::new(IconName::Search).data(SVG);
        assert!(matches!(icon.source_ref(), IconSource::Data(bytes) if bytes.as_ref() == SVG));

        let icon = icon.path("icons/replacement.svg");
        assert!(
            matches!(icon.source_ref(), IconSource::Path(path) if path == "icons/replacement.svg")
        );

        let icon = icon.data(SVG).data(b"replacement");
        assert!(
            matches!(icon.source_ref(), IconSource::Data(bytes) if bytes.as_ref() == b"replacement")
        );

        let icon = icon.path("");
        assert!(matches!(icon.source_ref(), IconSource::Path(path) if path.is_empty()));
    }
}
