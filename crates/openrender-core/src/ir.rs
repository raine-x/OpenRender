//! Language-neutral intermediate representation (IR) emitted by the engine.
//!
//! The host consumes a stream of [`Delta`]s. Every delta carries an ordered list
//! of [`Op`]s that are applied to the host's block list by `block_id`:
//!
//! * `put`    — insert or replace the block with the given id
//! * `append` — append text to a still-growing code block
//! * `close`  — mark a growing code block as complete
//! * `remove` — drop the block with the given id
//!
//! Block ids are dense, 0-based and monotonically assigned in document order,
//! which makes incremental UI updates cheap: a settled block keeps its id
//! forever, and only the unstable tail is re-sent.

/// Top-level block types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    Heading,
    CodeBlock,
    ListItem,
    Table,
    Rule,
    FootnoteDef,
    LatexBlock,
    Audio,
    LinkPreview,
}

impl BlockKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BlockKind::Paragraph => "paragraph",
            BlockKind::Heading => "heading",
            BlockKind::CodeBlock => "code_block",
            BlockKind::ListItem => "list_item",
            BlockKind::Table => "table",
            BlockKind::Rule => "rule",
            BlockKind::FootnoteDef => "footnote_def",
            BlockKind::LatexBlock => "latex_block",
            BlockKind::Audio => "audio",
            BlockKind::LinkPreview => "link_preview",
        }
    }
}

/// Inline span types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanKind {
    Text,
    Code,
    SoftBreak,
    HardBreak,
    Image,
    Audio,
    LatexInline,
    FootnoteRef,
}

impl SpanKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SpanKind::Text => "text",
            SpanKind::Code => "code",
            SpanKind::SoftBreak => "softbreak",
            SpanKind::HardBreak => "hardbreak",
            SpanKind::Image => "image",
            SpanKind::Audio => "audio",
            SpanKind::LatexInline => "latex_inline",
            SpanKind::FootnoteRef => "footnote_ref",
        }
    }
}

impl Default for SpanKind {
    fn default() -> Self {
        SpanKind::Text
    }
}

/// List bullet / numbering marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListMarker {
    Bullet,
    Ordered(u64),
}

/// Rasterized math attached to a formula block/span.
#[derive(Clone, Debug)]
pub struct MathImage {
    pub mime: String,
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// One inline span. Styles are flattened (no nesting) so hosts can render with
/// a single attributed-string pass.
#[derive(Clone, Debug, Default)]
pub struct Span {
    pub kind: SpanKind,
    /// Text payload / code / TeX source / image alt text.
    pub text: String,
    /// Link target or media source.
    pub url: String,
    pub title: String,
    /// Footnote label for `FootnoteRef`.
    pub name: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    /// Optional intrinsic size hint (images), 0 when unknown.
    pub width: u32,
    pub height: u32,
    pub math: Option<Box<MathImage>>,
}

impl Span {
    pub fn new(kind: SpanKind) -> Self {
        Self {
            kind,
            ..Default::default()
        }
    }
}

/// Table payload: alignment per column plus header and body cell spans.
#[derive(Clone, Debug, Default)]
pub struct Table {
    /// `none` | `left` | `center` | `right`
    pub align: Vec<String>,
    pub header: Vec<Vec<Span>>,
    pub rows: Vec<Vec<Vec<Span>>>,
}

/// One IR block.
#[derive(Clone, Debug)]
pub struct Block {
    pub id: u64,
    pub kind: BlockKind,
    pub spans: Vec<Span>,
    /// Raw payload for code blocks and block math.
    pub text: String,
    /// Fence info string (code blocks).
    pub lang: String,
    /// Heading level, 1-6 (0 = not a heading).
    pub level: u8,
    /// List nesting depth (0 = top level).
    pub depth: u8,
    /// Blockquote nesting depth (0 = not quoted).
    pub quote: u8,
    pub marker: Option<ListMarker>,
    /// Task-list state; `None` for regular list items.
    pub checked: Option<bool>,
    /// Code block closed in the source (false = auto-closed while streaming).
    pub closed: bool,
    /// GFM alert kind: note / tip / important / warning / caution.
    pub alert: String,
    /// Footnote label for `FootnoteDef`.
    pub name: String,
    pub table: Option<Box<Table>>,
    /// Media source / preview URL.
    pub url: String,
    pub title: String,
    pub description: String,
    pub math: Option<Box<MathImage>>,
}

impl Block {
    pub fn new(kind: BlockKind) -> Self {
        Self {
            id: 0,
            kind,
            spans: Vec::new(),
            text: String::new(),
            lang: String::new(),
            level: 0,
            depth: 0,
            quote: 0,
            marker: None,
            checked: None,
            closed: true,
            alert: String::new(),
            name: String::new(),
            table: None,
            url: String::new(),
            title: String::new(),
            description: String::new(),
            math: None,
        }
    }
}

/// A single incremental update instruction.
#[derive(Clone, Debug)]
pub enum Op {
    Put(Block),
    Append { id: u64, text: String },
    Close { id: u64 },
    Remove { id: u64 },
}

/// The result of one `feed` / `flush` / `finish` call.
#[derive(Clone, Debug, Default)]
pub struct Delta {
    pub rev: u64,
    pub ops: Vec<Op>,
}

impl Delta {
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Serialize to the JSON wire format consumed by Kotlin / C hosts.
    pub fn to_json(&self) -> String {
        crate::json::encode(self)
    }
}
