//! Engine configuration.

use crate::sanitize;

/// Parser feature bits used by [`Config::parser_flags`].
pub mod parse_flags {
    pub const TABLES: u32 = 1 << 0;
    pub const STRIKETHROUGH: u32 = 1 << 1;
    pub const TASKLISTS: u32 = 1 << 2;
    pub const FOOTNOTES: u32 = 1 << 3;
    /// GFM alert blocks (`> [!NOTE]`), plus GFM tables / strike / task lists.
    pub const GFM: u32 = 1 << 4;
    pub const MATH: u32 = 1 << 5;
    pub const SMART_PUNCTUATION: u32 = 1 << 6;

    pub const DEFAULT: u32 = TABLES | STRIKETHROUGH | TASKLISTS | FOOTNOTES | GFM | MATH;
}

/// Tunables for incremental parsing, markdown features and link policy.
#[derive(Clone, Debug)]
pub struct Config {
    /// Minimum interval between parses, in milliseconds. `feed` calls that
    /// arrive faster are coalesced and parsed on the next flush. `0` disables
    /// throttling (render on arrival).
    pub throttle_ms: u32,
    /// Maximum number of bytes kept as "unstable tail" before the engine forces
    /// a commit. Bounds per-feed work so streaming stays O(new text).
    pub max_tail_bytes: usize,
    /// Hard cap on the total document size; further input is dropped.
    pub max_document_bytes: usize,
    /// Hard cap on committed blocks; `0` disables the cap.
    pub max_blocks: u64,
    /// Enable `link_preview` blocks for links whose host is in
    /// [`Config::preview_domains`].
    pub enable_previews: bool,
    /// Markdown extensions to enable (see [`parse_flags`]).
    pub parser_flags: u32,
    /// Render soft line breaks as a space instead of a `softbreak` span.
    pub soft_break_as_space: bool,
    /// Drop raw HTML instead of emitting it as literal text.
    pub drop_html: bool,
    /// Interpret image titles like `=300x200` as size hints.
    pub enable_size_hints: bool,
    /// URL schemes accepted for links/images. Anything else is rendered as
    /// plain text (no navigation, no fetch).
    pub allowed_schemes: Vec<String>,
    /// Hosts that turn a standalone link into a `link_preview` card.
    pub preview_domains: Vec<String>,
    /// File extensions treated as audio sources (lowercase, without the dot).
    pub audio_extensions: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            throttle_ms: 8,
            max_tail_bytes: 8 * 1024,
            max_document_bytes: 16 * 1024 * 1024,
            max_blocks: 1_000_000,
            enable_previews: true,
            parser_flags: parse_flags::DEFAULT,
            soft_break_as_space: false,
            drop_html: false,
            enable_size_hints: true,
            allowed_schemes: vec![
                "http".to_string(),
                "https".to_string(),
                "file".to_string(),
                "mailto".to_string(),
                "tel".to_string(),
            ],
            preview_domains: Vec::new(),
            audio_extensions: vec![
                "mp3".to_string(),
                "wav".to_string(),
                "m4a".to_string(),
                "aac".to_string(),
                "ogg".to_string(),
                "oga".to_string(),
                "opus".to_string(),
                "flac".to_string(),
                "weba".to_string(),
                "3gp".to_string(),
                "amr".to_string(),
            ],
        }
    }
}

impl Config {
    /// Enable or disable a parser feature bit.
    pub fn set_parser_flag(&mut self, flag: u32, enabled: bool) {
        if enabled {
            self.parser_flags |= flag;
        } else {
            self.parser_flags &= !flag;
        }
    }

    /// Allow an extra audio file extension (with or without a leading dot).
    pub fn allow_audio_extension(&mut self, extension: &str) {
        let ext = extension
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase();
        if ext.is_empty() {
            return;
        }
        if !self.audio_extensions.iter().any(|e| *e == ext) {
            self.audio_extensions.push(ext);
        }
    }

    /// Whether a URL points at an audio file per [`Config::audio_extensions`].
    pub fn is_audio_url(&self, url: &str) -> bool {
        match sanitize::extension_of(url) {
            Some(ext) => self.audio_extensions.iter().any(|e| *e == ext),
            None => false,
        }
    }

    /// Allow an extra URL scheme (e.g. `agent` for `agent://resource/1`).
    pub fn allow_scheme(&mut self, scheme: &str) {
        let scheme = scheme.trim().trim_end_matches(':').to_ascii_lowercase();
        if scheme.is_empty() {
            return;
        }
        if !self.allowed_schemes.iter().any(|s| *s == scheme) {
            self.allowed_schemes.push(scheme);
        }
    }

    /// Register a host whose links render as preview cards.
    pub fn add_preview_domain(&mut self, domain: &str) {
        let domain = domain
            .trim()
            .trim_start_matches("www.")
            .to_ascii_lowercase();
        if domain.is_empty() {
            return;
        }
        if !self.preview_domains.iter().any(|d| *d == domain) {
            self.preview_domains.push(domain);
        }
    }

    /// Scheme allow-list check. Relative URLs and fragments are accepted.
    pub fn scheme_allowed(&self, url: &str) -> bool {
        let url = url.trim();
        if url.is_empty() || sanitize::has_control_chars(url) {
            return false;
        }
        match sanitize::scheme_of(url) {
            None => true,
            Some(scheme) => self
                .allowed_schemes
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(&scheme)),
        }
    }

    /// Whether a standalone link should become a preview card.
    pub fn is_preview_url(&self, url: &str) -> bool {
        if !self.enable_previews || !self.scheme_allowed(url) {
            return false;
        }
        match sanitize::scheme_of(url) {
            Some(scheme) if scheme == "http" || scheme == "https" => {}
            _ => return false,
        }
        let Some(host) = sanitize::host_of(url) else {
            return false;
        };
        let host = host.as_str();
        self.preview_domains.iter().any(|domain| {
            host == domain.as_str()
                || (host.len() > domain.len()
                    && host.ends_with(domain.as_str())
                    && host.as_bytes()[host.len() - domain.len() - 1] == b'.')
        })
    }
}
