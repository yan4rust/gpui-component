use gpui::{rems, App, ElementId, IntoElement, Rems, RenderOnce, SharedString, Window};
use html::HtmlElement;
use markdown::MarkdownElement;

mod element;
mod html;
mod markdown;
mod utils;

#[derive(IntoElement, Clone)]
pub enum Text {
    String(SharedString),
    TextView(TextView),
}

impl From<SharedString> for Text {
    fn from(s: SharedString) -> Self {
        Self::String(s)
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Self::String(SharedString::from(s.to_string()))
    }
}

impl From<String> for Text {
    fn from(s: String) -> Self {
        Self::String(s.into())
    }
}

impl From<TextView> for Text {
    fn from(e: TextView) -> Self {
        Self::TextView(e)
    }
}

impl RenderOnce for Text {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        match self {
            Self::String(s) => s.into_any_element(),
            Self::TextView(e) => e.into_any_element(),
        }
    }
}

/// TextViewStyle used to customize the style for [`TextView`].
#[derive(Copy, Clone)]
pub struct TextViewStyle {
    paragraph_gap: Rems,
}

impl Default for TextViewStyle {
    fn default() -> Self {
        Self {
            paragraph_gap: rems(1.),
        }
    }
}

impl TextViewStyle {
    /// Default style for inline text.
    ///
    /// This style has no paragraph gap.
    pub fn inline() -> Self {
        Self {
            paragraph_gap: rems(0.),
        }
    }

    /// Set paragraph gap, default is 1 rem.
    pub fn paragraph_gap(mut self, gap: Rems) -> Self {
        self.paragraph_gap = gap;
        self
    }
}

/// A text view that can render Markdown or HTML.
#[allow(private_interfaces)]
#[derive(IntoElement, Clone)]
pub enum TextView {
    Markdown(MarkdownElement),
    Html(HtmlElement),
}

impl TextView {
    /// Create a new markdown text view.
    pub fn markdown(id: impl Into<ElementId>, raw: impl Into<SharedString>) -> Self {
        Self::Markdown(MarkdownElement::new(id, raw))
    }

    /// Create a new html text view.
    pub fn html(id: impl Into<ElementId>, raw: impl Into<SharedString>) -> Self {
        Self::Html(HtmlElement::new(id, raw))
    }

    /// Set the source text of the text view.
    pub fn text(self, raw: impl Into<SharedString>) -> Self {
        match self {
            Self::Markdown(el) => Self::Markdown(el.text(raw)),
            Self::Html(el) => Self::Html(el.text(raw)),
        }
    }

    /// Set [`TextViewStyle`].
    pub fn style(self, style: TextViewStyle) -> Self {
        match self {
            Self::Markdown(el) => Self::Markdown(el.style(style)),
            Self::Html(el) => Self::Html(el.style(style)),
        }
    }

    /// Set to use [`TextViewStyle::inline`].
    pub fn inline(self) -> Self {
        self.style(TextViewStyle::inline())
    }
}

impl RenderOnce for TextView {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        match self {
            Self::Markdown(el) => el.into_any_element(),
            Self::Html(el) => el.into_any_element(),
        }
    }
}
