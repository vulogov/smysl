//! `smysl-render` - the Voice plane (§10, §20).
//!
//! Profiles live outside the wire format. Rendering is deterministic: connectives are
//! template selection driven by relation kind and seeded by uid, never model inference.
//!
//! Rule V1 is enforced at profile *load*, not at emit, so a profile that would flatten
//! epistemic status cannot produce an artifact at all.
//!
//! Filled by SM-P12.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub use smysl_core::error::RenderError;

/// Render targets (§10). Availability depends on compiled-in backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Target {
    Markdown,
    Typst,
    Html,
    Slides,
    Json,
    Text,
}

impl Target {
    pub const ALL: &'static [Target] = &[
        Target::Markdown,
        Target::Typst,
        Target::Html,
        Target::Slides,
        Target::Json,
        Target::Text,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Target::Markdown => "markdown",
            Target::Typst => "typst",
            Target::Html => "html",
            Target::Slides => "slides",
            Target::Json => "json",
            Target::Text => "text",
        }
    }

    pub fn parse(s: &str) -> Option<Target> {
        match s {
            "md" => Some(Target::Markdown),
            other => Target::ALL.iter().copied().find(|t| t.as_str() == other),
        }
    }

    /// Whether this build can emit the target. `slides` is Typst-backed, so it tracks
    /// the same feature.
    pub const fn available(self) -> bool {
        match self {
            Target::Markdown | Target::Json | Target::Text => true,
            Target::Typst | Target::Slides => cfg!(feature = "typst"),
            Target::Html => cfg!(feature = "html"),
        }
    }
}

impl core::fmt::Display for Target {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_is_always_available() {
        assert!(Target::Markdown.available());
        assert!(Target::Json.available());
        assert!(Target::Text.available());
    }

    #[test]
    fn slides_track_the_typst_backend() {
        assert_eq!(Target::Slides.available(), Target::Typst.available());
    }

    #[test]
    fn target_names_round_trip_and_md_is_an_alias() {
        for &t in Target::ALL {
            assert_eq!(Target::parse(t.as_str()), Some(t));
        }
        assert_eq!(Target::parse("md"), Some(Target::Markdown));
        assert_eq!(Target::parse("pdf"), None);
    }
}
