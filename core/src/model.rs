use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ClipKind {
    Text,
    Image,
    File,
}

impl ClipKind {
    pub const ALL: [ClipKind; 3] = [ClipKind::Text, ClipKind::Image, ClipKind::File];

    pub fn as_str(self) -> &'static str {
        match self {
            ClipKind::Text => "text",
            ClipKind::Image => "image",
            ClipKind::File => "file",
        }
    }

    pub fn parse(s: &str) -> Option<ClipKind> {
        match s {
            "text" => Some(ClipKind::Text),
            "image" => Some(ClipKind::Image),
            "file" | "files" => Some(ClipKind::File),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RichFormat {
    Rtf,
    Html,
}

/// Optional formatted payload that accompanies a text clip.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rich {
    pub format: RichFormat,
    pub data: Vec<u8>,
}

impl Rich {
    /// The `rich` column stores only bytes; RTF always starts with `{\rtf`.
    pub fn sniff(data: Vec<u8>) -> Rich {
        let format = if data.starts_with(b"{\\rtf") { RichFormat::Rtf } else { RichFormat::Html };
        Rich { format, data }
    }
}

/// What was on the pasteboard, already classified (design section 5.2).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Payload {
    Text { text: String, rich: Option<Rich> },
    Image { png: Vec<u8> },
    Files { paths: Vec<String> },
}

impl Payload {
    pub fn text(s: impl Into<String>) -> Payload {
        Payload::Text { text: s.into(), rich: None }
    }

    pub fn kind(&self) -> ClipKind {
        match self {
            Payload::Text { .. } => ClipKind::Text,
            Payload::Image { .. } => ClipKind::Image,
            Payload::Files { .. } => ClipKind::File,
        }
    }

    /// Size of the primary payload in bytes.
    pub fn byte_size(&self) -> usize {
        match self {
            Payload::Text { text, .. } => text.len(),
            Payload::Image { png } => png.len(),
            Payload::Files { paths } => paths.iter().map(|p| p.len()).sum(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Payload::Text { text, .. } => text.trim().is_empty(),
            Payload::Image { png } => png.is_empty(),
            Payload::Files { paths } => paths.is_empty(),
        }
    }

    /// BLAKE3 of the canonical payload, prefixed by kind so that a text clip
    /// can never collide with a file list of the same bytes.
    pub fn hash(&self) -> String {
        let mut h = blake3::Hasher::new();
        h.update(self.kind().as_str().as_bytes());
        h.update(&[0]);
        match self {
            Payload::Text { text, .. } => {
                h.update(text.as_bytes());
            }
            Payload::Image { png } => {
                h.update(png);
            }
            Payload::Files { paths } => {
                for p in paths {
                    h.update(p.as_bytes());
                    h.update(&[0]);
                }
            }
        }
        h.finalize().to_hex().to_string()
    }
}

/// Row summary sent to the UI (design section 9.1).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: i64,
    pub kind: ClipKind,
    /// Truncated text, or a description for images/files.
    pub preview: String,
    pub is_url: bool,
    pub byte_size: i64,
    pub app_name: Option<String>,
    pub app_bundle: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
    pub has_rich: bool,
}

/// A clip with its full content.
#[derive(Clone, PartialEq, Debug)]
pub struct ClipDetail {
    pub clip: Clip,
    /// Text content; JSON array of paths for files; `[image WxH]` for images.
    pub text: Option<String>,
    pub rich: Option<Rich>,
    /// Absolute path of the image blob.
    pub blob_path: Option<std::path::PathBuf>,
}

impl ClipDetail {
    /// File paths of a file clip.
    pub fn paths(&self) -> Vec<String> {
        if self.clip.kind != ClipKind::File {
            return Vec::new();
        }
        self.text
            .as_deref()
            .and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or_default()
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub sort_order: i64,
    pub count: i64,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub id: i64,
    pub collection_id: i64,
    pub name: String,
    pub keyword: Option<String>,
    pub body: String,
    /// `text` or `image`.
    pub kind: String,
    pub has_blob: bool,
    pub origin_app: Option<String>,
    pub use_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Fields the user supplies when creating or promoting a snippet.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SnippetDraft {
    pub collection_id: i64,
    pub name: String,
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub body: String,
}

/// Partial update for a snippet; absent fields are left alone.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SnippetPatch {
    pub collection_id: Option<i64>,
    pub name: Option<String>,
    /// `Some(None)` or `Some("")` clears the keyword.
    #[serde(default, deserialize_with = "double_option")]
    pub keyword: Option<Option<String>>,
    pub body: Option<String>,
}

fn double_option<'de, D>(d: D) -> std::result::Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(d).map(Some)
}

/// Does this text look like a single URL?
pub fn looks_like_url(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.chars().any(char::is_whitespace) {
        return false;
    }
    let Some((scheme, rest)) = t.split_once("://") else {
        return t.starts_with("mailto:") && t.len() > 7;
    };
    !rest.is_empty()
        && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
}

/// Width and height from a PNG header, if the bytes are a PNG.
pub fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" || &png[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(png[20..24].try_into().ok()?);
    Some((w, h))
}

/// Truncate to at most `max` characters, appending an ellipsis if cut.
pub fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((idx, _)) => format!("{}…", &s[..idx]),
        None => s.to_string(),
    }
}
