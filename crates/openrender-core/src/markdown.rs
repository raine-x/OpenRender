//! Markdown event stream -> IR blocks.
//!
//! The whole module is allocation-light and, more importantly, *iterative*:
//! `pulldown-cmark` walks the document without recursion, the builder keeps a
//! flat block vector plus small stacks for containers, so deeply nested input
//! cannot blow the native stack.

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::config::{parse_flags, Config};
use crate::ir::{Block, BlockKind, ListMarker, Span, SpanKind, Table};
use crate::sanitize::parse_size_hint;

/// Inputs shared by every render pass.
pub struct RenderArgs<'a> {
    pub cfg: &'a Config,
    /// True when the source chunk ends inside an unterminated code fence.
    pub unclosed_fence: bool,
    /// Present only when the `latex` feature is enabled and the host attached a
    /// renderer.
    #[cfg(feature = "latex")]
    pub math: Option<&'a dyn openrender_latex::MathRenderer>,
}

/// Render a complete markdown chunk (one or more top-level blocks) to IR.
pub fn render(text: &str, args: &RenderArgs<'_>) -> Vec<Block> {
    let mut opts = Options::empty();
    let flags = args.cfg.parser_flags;
    if flags & parse_flags::TABLES != 0 {
        opts.insert(Options::ENABLE_TABLES);
    }
    if flags & parse_flags::STRIKETHROUGH != 0 {
        opts.insert(Options::ENABLE_STRIKETHROUGH);
    }
    if flags & parse_flags::TASKLISTS != 0 {
        opts.insert(Options::ENABLE_TASKLISTS);
    }
    if flags & parse_flags::FOOTNOTES != 0 {
        opts.insert(Options::ENABLE_FOOTNOTES);
    }
    if flags & parse_flags::GFM != 0 {
        opts.insert(Options::ENABLE_GFM);
    }
    if flags & parse_flags::MATH != 0 {
        opts.insert(Options::ENABLE_MATH);
    }
    if flags & parse_flags::SMART_PUNCTUATION != 0 {
        opts.insert(Options::ENABLE_SMART_PUNCTUATION);
    }

    let mut builder = Builder::new(args);
    for event in Parser::new_ext(text, opts) {
        builder.event(event);
    }
    builder.finish()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActiveTag {
    Paragraph,
    Heading,
    Item,
    Footnote,
    Table,
}

struct ListCtx {
    ordered: bool,
    next: u64,
}

struct TableCtx {
    align: Vec<String>,
    header: Vec<Vec<Span>>,
    rows: Vec<Vec<Vec<Span>>>,
    cur_row: Vec<Vec<Span>>,
    in_head: bool,
}

struct LinkCtx {
    url: String,
    title: String,
}

struct ImageCtx {
    src: String,
    title: String,
    alt: String,
}

struct CodeCtx {
    lang: String,
    text: String,
}

struct Builder<'a> {
    args: &'a RenderArgs<'a>,
    blocks: Vec<Block>,
    active: Vec<(usize, ActiveTag)>,
    quote: u8,
    quotes: Vec<Option<String>>,
    lists: Vec<ListCtx>,
    strong: u32,
    em: u32,
    strike: u32,
    links: Vec<LinkCtx>,
    image: Option<ImageCtx>,
    code: Option<CodeCtx>,
    cell: Option<Vec<Span>>,
    table: Option<TableCtx>,
    skip_html: bool,
}

impl<'a> Builder<'a> {
    fn new(args: &'a RenderArgs<'a>) -> Self {
        Self {
            args,
            blocks: Vec::new(),
            active: Vec::new(),
            quote: 0,
            quotes: Vec::new(),
            lists: Vec::new(),
            strong: 0,
            em: 0,
            strike: 0,
            links: Vec::new(),
            image: None,
            code: None,
            cell: None,
            table: None,
            skip_html: false,
        }
    }

    fn event(&mut self, event: Event<'_>) {
        if self.skip_html {
            if matches!(event, Event::End(TagEnd::HtmlBlock)) {
                self.skip_html = false;
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.push_text(&text),
            Event::Code(text) => {
                let mut span = Span::new(SpanKind::Code);
                span.text = text.to_string();
                self.push_span(self.decorate(span));
            }
            Event::InlineMath(tex) => {
                let mut span = Span::new(SpanKind::LatexInline);
                span.text = tex.to_string();
                self.attach_math(&mut span, &tex, false);
                self.push_span(span);
            }
            Event::DisplayMath(tex) => self.display_math(&tex),
            Event::Html(html) => {
                if !self.args.cfg.drop_html {
                    self.push_text(&html);
                }
            }
            Event::InlineHtml(html) => {
                if !self.args.cfg.drop_html {
                    self.push_text(&html);
                }
            }
            Event::FootnoteReference(name) => {
                let mut span = Span::new(SpanKind::FootnoteRef);
                span.name = name.to_string();
                self.push_span(span);
            }
            Event::SoftBreak => {
                if self.args.cfg.soft_break_as_space {
                    self.push_text(" ");
                } else {
                    self.push_span(Span::new(SpanKind::SoftBreak));
                }
            }
            Event::HardBreak => {
                let span = Span::new(SpanKind::HardBreak);
                self.push_span(span);
            }
            Event::Rule => {
                self.new_block(BlockKind::Rule);
            }
            Event::TaskListMarker(checked) => {
                if let Some(&(idx, ActiveTag::Item)) = self.active.last() {
                    self.blocks[idx].checked = Some(checked);
                }
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.start_paragraph(),
            Tag::Heading { level, .. } => {
                let idx = self.new_block(BlockKind::Heading);
                self.blocks[idx].level = heading_level(level);
                self.active.push((idx, ActiveTag::Heading));
            }
            Tag::BlockQuote(kind) => {
                self.quote = self.quote.saturating_add(1);
                self.quotes.push(kind.map(alert_name));
            }
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some(CodeCtx {
                    lang,
                    text: String::new(),
                });
            }
            Tag::List(start) => {
                self.lists.push(ListCtx {
                    ordered: start.is_some(),
                    next: start.unwrap_or(1),
                });
            }
            Tag::Item => self.start_item(),
            Tag::FootnoteDefinition(name) => {
                let idx = self.new_block(BlockKind::FootnoteDef);
                self.blocks[idx].name = name.to_string();
                self.active.push((idx, ActiveTag::Footnote));
            }
            Tag::Table(aligns) => {
                let idx = self.new_block(BlockKind::Table);
                self.table = Some(TableCtx {
                    align: aligns
                        .iter()
                        .map(|a| {
                            match a {
                                Alignment::Left => "left",
                                Alignment::Center => "center",
                                Alignment::Right => "right",
                                Alignment::None => "none",
                            }
                            .to_string()
                        })
                        .collect(),
                    header: Vec::new(),
                    rows: Vec::new(),
                    cur_row: Vec::new(),
                    in_head: false,
                });
                self.active.push((idx, ActiveTag::Table));
            }
            Tag::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.in_head = true;
                    table.cur_row.clear();
                }
            }
            Tag::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.cur_row.clear();
                }
            }
            Tag::TableCell => {
                self.cell = Some(Vec::new());
            }
            Tag::Link {
                dest_url, title, ..
            } => {
                let url = if self.args.cfg.scheme_allowed(&dest_url) {
                    dest_url.to_string()
                } else {
                    String::new()
                };
                self.links.push(LinkCtx {
                    url,
                    title: title.to_string(),
                });
            }
            Tag::Image {
                dest_url, title, ..
            } => {
                self.image = Some(ImageCtx {
                    src: dest_url.to_string(),
                    title: title.to_string(),
                    alt: String::new(),
                });
            }
            Tag::Emphasis => self.em += 1,
            Tag::Strong => self.strong += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::HtmlBlock => {
                if self.args.cfg.drop_html {
                    self.skip_html = true;
                } else {
                    // Keep raw HTML as literal text in its own paragraph; it is
                    // never parsed or executed.
                    let idx = self.new_block(BlockKind::Paragraph);
                    self.active.push((idx, ActiveTag::Paragraph));
                }
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.end_paragraph(),
            TagEnd::HtmlBlock => self.end_paragraph(),
            TagEnd::Heading(_) => {
                self.pop_active(ActiveTag::Heading);
            }
            TagEnd::BlockQuote(_) => {
                self.quote = self.quote.saturating_sub(1);
                self.quotes.pop();
            }
            TagEnd::CodeBlock => self.end_code_block(),
            TagEnd::List(_) => {
                self.lists.pop();
            }
            TagEnd::Item => {
                self.pop_active(ActiveTag::Item);
            }
            TagEnd::FootnoteDefinition => {
                self.pop_active(ActiveTag::Footnote);
            }
            TagEnd::Table => self.end_table(),
            TagEnd::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.header = std::mem::take(&mut table.cur_row);
                    table.in_head = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    let row = std::mem::take(&mut table.cur_row);
                    table.rows.push(row);
                }
            }
            TagEnd::TableCell => {
                let cell = self.cell.take().unwrap_or_default();
                if let Some(table) = self.table.as_mut() {
                    table.cur_row.push(cell);
                }
            }
            TagEnd::Link => {
                self.links.pop();
            }
            TagEnd::Image => self.end_image(),
            TagEnd::Emphasis => self.em = self.em.saturating_sub(1),
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            _ => {}
        }
    }

    fn start_paragraph(&mut self) {
        if let Some(&(idx, owner)) = self.active.last() {
            match owner {
                ActiveTag::Item | ActiveTag::Footnote => {
                    if !self.blocks[idx].spans.is_empty() {
                        let span = Span::new(SpanKind::SoftBreak);
                        self.push_span(span);
                    }
                    self.active.push((idx, ActiveTag::Paragraph));
                    return;
                }
                ActiveTag::Table => return,
                _ => {}
            }
        }
        let idx = self.new_block(BlockKind::Paragraph);
        self.active.push((idx, ActiveTag::Paragraph));
    }

    fn start_item(&mut self) {
        let (ordered, number) = match self.lists.last_mut() {
            Some(ctx) => {
                let number = ctx.next;
                ctx.next = ctx.next.saturating_add(1);
                (ctx.ordered, number)
            }
            None => (false, 1),
        };
        let idx = self.new_block(BlockKind::ListItem);
        self.blocks[idx].depth = self.lists.len().saturating_sub(1).min(u8::MAX as usize) as u8;
        self.blocks[idx].marker = Some(if ordered {
            ListMarker::Ordered(number)
        } else {
            ListMarker::Bullet
        });
        self.active.push((idx, ActiveTag::Item));
    }

    fn end_paragraph(&mut self) {
        if let Some(&(idx, ActiveTag::Paragraph)) = self.active.last() {
            self.active.pop();
            self.finalize_paragraph(idx);
        }
    }

    fn end_code_block(&mut self) {
        let Some(code) = self.code.take() else {
            return;
        };
        let idx = self.new_block(BlockKind::CodeBlock);
        self.blocks[idx].text = code.text;
        self.blocks[idx].lang = code.lang;
        self.blocks[idx].depth = self.lists.len().min(u8::MAX as usize) as u8;
        self.blocks[idx].closed = true;
    }

    fn end_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        if let Some(idx) = self.pop_active(ActiveTag::Table) {
            self.blocks[idx].table = Some(Box::new(Table {
                align: table.align,
                header: table.header,
                rows: table.rows,
            }));
        }
    }

    fn end_image(&mut self) {
        let Some(image) = self.image.take() else {
            return;
        };
        let (dims, caption) = if self.args.cfg.enable_size_hints {
            parse_size_hint(&image.title)
        } else {
            (None, image.title.clone())
        };
        let title = if caption.is_empty() {
            image.title
        } else {
            caption
        };

        if !self.args.cfg.scheme_allowed(&image.src) {
            if !image.alt.is_empty() {
                self.push_text(&image.alt);
            }
            return;
        }

        let is_audio = self.args.cfg.is_audio_url(&image.src);
        let mut span = Span::new(if is_audio {
            SpanKind::Audio
        } else {
            SpanKind::Image
        });
        span.text = image.alt;
        span.url = image.src;
        span.title = title;
        if let Some((w, h)) = dims {
            span.width = w;
            span.height = h;
        }
        self.push_span(span);
    }

    fn display_math(&mut self, tex: &str) {
        let converted = match self.active.last() {
            Some(&(idx, ActiveTag::Paragraph))
                if self.blocks[idx].kind == BlockKind::Paragraph
                    && self.blocks[idx].spans.is_empty() =>
            {
                Some(idx)
            }
            _ => None,
        };
        let idx = match converted {
            Some(idx) => {
                self.blocks[idx].kind = BlockKind::LatexBlock;
                idx
            }
            None => self.new_block(BlockKind::LatexBlock),
        };
        self.blocks[idx].text = tex.to_string();
        #[cfg(feature = "latex")]
        {
            if let Some(renderer) = self.args.math {
                if let Some(rendered) = renderer.render(tex, true) {
                    self.blocks[idx].math = Some(Box::new(crate::ir::MathImage {
                        mime: rendered.format.mime().to_string(),
                        data: rendered.data,
                        width: rendered.width,
                        height: rendered.height,
                    }));
                }
            }
        }
    }

    fn push_text(&mut self, text: &str) {
        if let Some(code) = self.code.as_mut() {
            code.text.push_str(text);
            return;
        }
        if let Some(image) = self.image.as_mut() {
            image.alt.push_str(text);
            return;
        }
        let bold = self.strong > 0;
        let italic = self.em > 0;
        let strike = self.strike > 0;

        if let Some(cell) = self.cell.as_mut() {
            push_text_into(cell, text, bold, italic, strike, self.links.last());
            return;
        }
        if let Some(&(idx, _)) = self.active.last() {
            push_text_into(
                &mut self.blocks[idx].spans,
                text,
                bold,
                italic,
                strike,
                self.links.last(),
            );
        }
    }

    fn decorate(&self, mut span: Span) -> Span {
        if self.strong > 0 {
            span.bold = true;
        }
        if self.em > 0 {
            span.italic = true;
        }
        if self.strike > 0 {
            span.strike = true;
        }
        if span.url.is_empty() {
            if let Some(link) = self.links.last() {
                span.url = link.url.clone();
                span.title = link.title.clone();
            }
        }
        span
    }

    fn push_span(&mut self, span: Span) {
        if let Some(image) = self.image.as_mut() {
            match span.kind {
                SpanKind::SoftBreak | SpanKind::HardBreak => image.alt.push(' '),
                _ => image.alt.push_str(&span.text),
            }
            return;
        }
        if let Some(cell) = self.cell.as_mut() {
            push_merged(cell, span);
            return;
        }
        let Some(&(idx, _)) = self.active.last() else {
            return;
        };
        push_merged(&mut self.blocks[idx].spans, span);
    }

    fn new_block(&mut self, kind: BlockKind) -> usize {
        let mut block = Block::new(kind);
        block.quote = self.quote;
        block.alert = self
            .quotes
            .iter()
            .rev()
            .find_map(|q| q.clone())
            .unwrap_or_default();
        self.blocks.push(block);
        self.blocks.len() - 1
    }

    fn pop_active(&mut self, owner: ActiveTag) -> Option<usize> {
        if let Some(&(idx, tag)) = self.active.last() {
            if tag == owner {
                self.active.pop();
                return Some(idx);
            }
        }
        None
    }

    fn finalize_paragraph(&mut self, idx: usize) {
        while matches!(
            self.blocks[idx].spans.last().map(|s| s.kind),
            Some(SpanKind::SoftBreak) | Some(SpanKind::HardBreak)
        ) {
            self.blocks[idx].spans.pop();
        }
        if self.blocks[idx].kind != BlockKind::Paragraph {
            return;
        }
        if self.blocks[idx].spans.is_empty() {
            self.drop_block(idx);
            return;
        }
        self.convert_standalone(idx);
    }

    fn convert_standalone(&mut self, idx: usize) {
        if self.blocks[idx].spans.len() != 1 {
            return;
        }
        let (kind, url_is_audio, url, text, title_attr) = {
            let span = &self.blocks[idx].spans[0];
            (
                span.kind,
                !span.url.is_empty() && self.args.cfg.is_audio_url(&span.url),
                span.url.as_str(),
                span.text.as_str(),
                span.title.as_str(),
            )
        };

        if (kind == SpanKind::Audio || (kind == SpanKind::Text && url_is_audio)) && !url.is_empty()
        {
            let url = url.to_string();
            let title = if text.is_empty() {
                title_attr.to_string()
            } else {
                text.to_string()
            };
            let block = &mut self.blocks[idx];
            block.kind = BlockKind::Audio;
            block.url = url;
            block.title = title;
            block.spans.clear();
            return;
        }

        if kind == SpanKind::Text && !url.is_empty() && self.args.cfg.is_preview_url(url) {
            let url = url.to_string();
            let title = text.to_string();
            let description = title_attr.to_string();
            let block = &mut self.blocks[idx];
            block.kind = BlockKind::LinkPreview;
            block.url = url;
            block.title = title;
            block.description = description;
            block.spans.clear();
        }
    }

    fn drop_block(&mut self, idx: usize) {
        self.blocks.remove(idx);
        for (i, _) in self.active.iter_mut() {
            if *i > idx {
                *i -= 1;
            }
        }
    }

    fn attach_math(&mut self, span: &mut Span, tex: &str, display: bool) {
        #[cfg(feature = "latex")]
        {
            let _ = display;
            if let Some(renderer) = self.args.math {
                if let Some(rendered) = renderer.render(tex, false) {
                    span.math = Some(Box::new(crate::ir::MathImage {
                        mime: rendered.format.mime().to_string(),
                        data: rendered.data,
                        width: rendered.width,
                        height: rendered.height,
                    }));
                }
            }
        }
        #[cfg(not(feature = "latex"))]
        {
            let _ = (span, tex, display);
        }
    }

    fn finish(mut self) -> Vec<Block> {
        if self.args.unclosed_fence {
            if let Some(block) = self
                .blocks
                .iter_mut()
                .rev()
                .find(|b| b.kind == BlockKind::CodeBlock)
            {
                block.closed = false;
            }
        }
        // Drop blocks that failed to produce content (e.g. an empty table).
        let mut i = 0;
        while i < self.blocks.len() {
            let empty_table = self.blocks[i].kind == BlockKind::Table
                && self.blocks[i]
                    .table
                    .as_ref()
                    .map(|t| t.header.is_empty() && t.rows.is_empty())
                    .unwrap_or(true);
            if empty_table {
                self.blocks.remove(i);
            } else {
                i += 1;
            }
        }
        self.blocks
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn alert_name(kind: pulldown_cmark::BlockQuoteKind) -> String {
    match kind {
        pulldown_cmark::BlockQuoteKind::Note => "note",
        pulldown_cmark::BlockQuoteKind::Tip => "tip",
        pulldown_cmark::BlockQuoteKind::Important => "important",
        pulldown_cmark::BlockQuoteKind::Warning => "warning",
        pulldown_cmark::BlockQuoteKind::Caution => "caution",
    }
    .to_string()
}

fn push_merged(spans: &mut Vec<Span>, span: Span) {
    if span.kind == SpanKind::Text {
        if let Some(last) = spans.last_mut() {
            if last.kind == SpanKind::Text
                && last.bold == span.bold
                && last.italic == span.italic
                && last.strike == span.strike
                && last.url == span.url
                && last.title == span.title
            {
                last.text.push_str(&span.text);
                return;
            }
        }
    }
    spans.push(span);
}

/// Append a text run, merging with the previous text span when styles match.
/// Avoids one `String` allocation per text event on the hot path.
fn push_text_into(
    spans: &mut Vec<Span>,
    text: &str,
    bold: bool,
    italic: bool,
    strike: bool,
    link: Option<&LinkCtx>,
) {
    let (url, title) = match link {
        Some(link) => (link.url.as_str(), link.title.as_str()),
        None => ("", ""),
    };
    if let Some(last) = spans.last_mut() {
        if last.kind == SpanKind::Text
            && last.bold == bold
            && last.italic == italic
            && last.strike == strike
            && last.url == url
            && last.title == title
        {
            last.text.push_str(text);
            return;
        }
    }
    let mut span = Span::new(SpanKind::Text);
    span.text = text.to_string();
    span.bold = bold;
    span.italic = italic;
    span.strike = strike;
    if !url.is_empty() {
        span.url = url.to_string();
    }
    if !title.is_empty() {
        span.title = title.to_string();
    }
    spans.push(span);
}
