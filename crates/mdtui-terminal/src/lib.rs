use std::{
    collections::{HashMap, HashSet},
    env,
    hash::{DefaultHasher, Hash, Hasher},
    io::{self, ErrorKind, Stdout, Write},
    panic,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use anyhow::Result;
use arboard::Clipboard;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use image::{GenericImageView, imageops::FilterType};
use mdtui_render::Theme;
use ratatui::{Terminal, backend::CrosstermBackend};

pub const ST: &str = "\x1b\\";

pub type TuiBackend = CrosstermBackend<Stdout>;
pub type TuiTerminal = Terminal<TuiBackend>;

pub fn osc_set_background(hex: &str) -> String {
    format!("\x1b]11;{hex}{ST}")
}

pub fn osc_reset_background() -> &'static str {
    "\x1b]111\x1b\\"
}

pub fn osc52_copy(text: &str) -> String {
    format!("\x1b]52;c;{}{ST}", STANDARD.encode(text.as_bytes()))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalCapabilities {
    pub kitty_graphics: bool,
    pub terminal_program: Option<String>,
    pub term: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalRenderPath {
    KittyGraphics,
    StyledTextFallback,
}

impl TerminalCapabilities {
    pub fn detect() -> Self {
        Self::detect_from(env::vars())
    }

    pub fn detect_from<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        let mut terminal_program = None;
        let mut term = None;
        let mut kitty_window_id = false;
        let mut wezterm = false;
        let mut ghostty = false;
        for (key, value) in vars {
            let key = key.as_ref();
            let value = value.into();
            match key {
                "TERM_PROGRAM" => {
                    wezterm = value.eq_ignore_ascii_case("WezTerm");
                    ghostty = value.eq_ignore_ascii_case("ghostty");
                    terminal_program = Some(value);
                }
                "TERM" => term = Some(value),
                "KITTY_WINDOW_ID" => kitty_window_id = true,
                _ => {}
            }
        }
        let term_has_kitty = term
            .as_deref()
            .map(|term| term.to_ascii_lowercase().contains("kitty"))
            .unwrap_or(false);
        Self {
            kitty_graphics: kitty_window_id || ghostty || wezterm || term_has_kitty,
            terminal_program,
            term,
        }
    }

    pub fn render_path(&self) -> TerminalRenderPath {
        if self.kitty_graphics {
            TerminalRenderPath::KittyGraphics
        } else {
            TerminalRenderPath::StyledTextFallback
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GraphicsCacheKey {
    pub kind: String,
    pub text: String,
    pub width_cells: u16,
    pub theme_hash: u64,
    pub scale_milli: u16,
}

impl GraphicsCacheKey {
    pub fn heading(level: u8, text: &str, width_cells: u16, theme_hash: u64) -> Self {
        Self {
            kind: format!("h{level}"),
            text: text.to_string(),
            width_cells,
            theme_hash,
            scale_milli: heading_scale_milli(level),
        }
    }

    pub fn local_image(path: &str, width_cells: u16, theme_hash: u64) -> Self {
        Self {
            kind: "local-image".to_string(),
            text: path.to_string(),
            width_cells,
            theme_hash,
            scale_milli: 1000,
        }
    }

    pub fn preview(label: &str, source_hash: u64, width_cells: u16, theme_hash: u64) -> Self {
        Self {
            kind: "preview".to_string(),
            text: format!("{label}:{source_hash:x}"),
            width_cells,
            theme_hash,
            scale_milli: 1000,
        }
    }

    pub fn external_preview(
        label: &str,
        source_hash: u64,
        width_cells: u16,
        theme_hash: u64,
    ) -> Self {
        Self {
            kind: "external-preview".to_string(),
            text: format!("{label}:{source_hash:x}"),
            width_cells,
            theme_hash,
            scale_milli: 1000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KittyImage {
    pub id: u32,
    pub width_px: u32,
    pub height_px: u32,
    pub z_index: i32,
}

#[derive(Clone, Debug, Default)]
pub struct KittyGraphicsCache {
    next_id: u32,
    entries: HashMap<GraphicsCacheKey, KittyImage>,
}

impl KittyGraphicsCache {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            entries: HashMap::new(),
        }
    }

    pub fn get(&self, key: &GraphicsCacheKey) -> Option<KittyImage> {
        self.entries.get(key).copied()
    }

    pub fn get_or_insert(
        &mut self,
        key: GraphicsCacheKey,
        width_px: u32,
        height_px: u32,
        z_index: i32,
    ) -> KittyImage {
        self.get_or_insert_with_id(key, None, width_px, height_px, z_index)
    }

    pub fn get_or_insert_with_id(
        &mut self,
        key: GraphicsCacheKey,
        id: Option<u32>,
        width_px: u32,
        height_px: u32,
        z_index: i32,
    ) -> KittyImage {
        if let Some(image) = self.entries.get(&key).copied() {
            return image;
        }
        let id = id.unwrap_or(self.next_id).max(1);
        let image = KittyImage {
            id,
            width_px,
            height_px,
            z_index,
        };
        self.next_id = self.next_id.max(id).saturating_add(1).max(1);
        self.entries.insert(key, image);
        image
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

pub fn kitty_transmit_rgba(image: KittyImage, rgba: &[u8]) -> String {
    format!(
        "\x1b_Ga=t,f=32,s={},v={},i={},q=2,z={};{}\x1b\\",
        image.width_px,
        image.height_px,
        image.id,
        image.z_index,
        STANDARD.encode(rgba)
    )
}

pub fn kitty_transmit_png(image: KittyImage, png: &[u8]) -> String {
    format!(
        "\x1b_Ga=t,f=100,s={},v={},i={},q=2,z={};{}\x1b\\",
        image.width_px,
        image.height_px,
        image.id,
        image.z_index,
        STANDARD.encode(png)
    )
}

pub fn kitty_create_virtual_placement(image: KittyImage, columns: u16, rows: u16) -> String {
    format!(
        "\x1b_Ga=p,U=1,i={},c={},r={},q=2,z={};\x1b\\",
        image.id,
        columns.max(1),
        rows.max(1),
        image.z_index
    )
}

pub fn kitty_delete_image(image_id: u32) -> String {
    format!("\x1b_Ga=d,d=I,i={image_id},q=2;\x1b\\")
}

pub fn raster_heading_rgba(text: &str, level: u8, width_cells: u16, theme: &Theme) -> Vec<u8> {
    let width_px = heading_width_px(width_cells);
    let height_px = heading_height_px(level);
    let capacity = width_px.saturating_mul(height_px).saturating_mul(4) as usize;
    let mut pixels = Vec::with_capacity(capacity);
    let accent = [theme.accent.0, theme.accent.1, theme.accent.2, 255];
    let dim = [theme.fg_dim.0, theme.fg_dim.1, theme.fg_dim.2, 210];
    let bg = [theme.bg.0, theme.bg.1, theme.bg.2, 0];
    let text_hash = stable_hash(text);
    for y in 0..height_px {
        for x in 0..width_px {
            let underline = y + 3 >= height_px && x % 4 != 0;
            let stripe = ((x / 8) as u64 + (y / 8) as u64 + text_hash).is_multiple_of(7);
            let color = if underline {
                accent
            } else if stripe && y > 3 && y + 6 < height_px {
                dim
            } else {
                bg
            };
            pixels.extend_from_slice(&color);
        }
    }
    pixels
}

pub fn raster_preview_rgba(
    label: &str,
    source_hash: u64,
    width_cells: u16,
    height_cells: u16,
    theme: &Theme,
) -> Vec<u8> {
    let width_px = image_preview_width_px(width_cells);
    let height_px = image_preview_height_px(height_cells);
    let capacity = width_px.saturating_mul(height_px).saturating_mul(4) as usize;
    let mut pixels = Vec::with_capacity(capacity);
    let accent = [theme.accent.0, theme.accent.1, theme.accent.2, 230];
    let line = [theme.line.0, theme.line.1, theme.line.2, 180];
    let bg = [theme.bg.0, theme.bg.1, theme.bg.2, 0];
    let label_hash = stable_hash(label) ^ source_hash;
    for y in 0..height_px {
        for x in 0..width_px {
            let border = x < 2 || y < 2 || x + 3 > width_px || y + 3 > height_px;
            let trace = ((x / 10) as u64 + (y / 6) as u64 + label_hash).is_multiple_of(9);
            let color = if border {
                line
            } else if trace {
                accent
            } else {
                bg
            };
            pixels.extend_from_slice(&color);
        }
    }
    pixels
}

pub fn heading_width_px(width_cells: u16) -> u32 {
    u32::from(width_cells.max(1)).saturating_mul(8)
}

pub fn heading_height_px(level: u8) -> u32 {
    match level {
        1 => 24,
        2 => 20,
        _ => 16,
    }
}

pub fn heading_scale_milli(level: u8) -> u16 {
    match level {
        1 => 1500,
        2 => 1250,
        _ => 1000,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterImage {
    pub width_px: u32,
    pub height_px: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalPreviewCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl ExternalPreviewCommand {
    pub fn from_command_line(value: &str) -> Option<Self> {
        let mut parts = value.split_whitespace();
        Some(Self {
            program: PathBuf::from(parts.next()?),
            args: parts.map(str::to_string).collect(),
        })
    }
}

pub fn raster_local_image_rgba(
    bytes: &[u8],
    max_width_px: u32,
    max_height_px: u32,
) -> Option<RasterImage> {
    let decoded = image::load_from_memory(bytes).ok()?;
    let (source_width, source_height) = decoded.dimensions();
    if source_width == 0 || source_height == 0 {
        return None;
    }
    let max_width_px = max_width_px.max(1);
    let max_height_px = max_height_px.max(1);
    let scale = (max_width_px as f64 / source_width as f64)
        .min(max_height_px as f64 / source_height as f64)
        .min(1.0);
    let width_px = ((source_width as f64 * scale).round() as u32).max(1);
    let height_px = ((source_height as f64 * scale).round() as u32).max(1);
    let resized = decoded.resize_exact(width_px, height_px, FilterType::Triangle);
    Some(RasterImage {
        width_px,
        height_px,
        rgba: resized.to_rgba8().into_raw(),
    })
}

pub fn image_preview_width_px(width_cells: u16) -> u32 {
    u32::from(width_cells.max(1)).saturating_mul(8)
}

pub fn image_preview_height_px(height_cells: u16) -> u32 {
    u32::from(height_cells.max(1)).saturating_mul(16)
}

pub fn external_preview_command_for_label(label: &str) -> Option<ExternalPreviewCommand> {
    let specific = format!("MDTUI_PREVIEW_{}_RENDERER", preview_env_suffix(label));
    env_preview_command(&specific)
        .or_else(|| {
            if label.eq_ignore_ascii_case("math") {
                env_preview_command("MDTUI_MATH_RENDERER")
            } else {
                env_preview_command("MDTUI_DIAGRAM_RENDERER")
            }
        })
        .or_else(|| env_preview_command("MDTUI_PREVIEW_RENDERER"))
}

pub fn render_external_preview(
    command: &ExternalPreviewCommand,
    label: &str,
    source: &str,
    max_width_px: u32,
    max_height_px: u32,
) -> io::Result<Option<RasterImage>> {
    render_external_preview_with_theme(command, label, source, max_width_px, max_height_px, None)
}

pub fn render_external_preview_with_theme(
    command: &ExternalPreviewCommand,
    label: &str,
    source: &str,
    max_width_px: u32,
    max_height_px: u32,
    theme: Option<&Theme>,
) -> io::Result<Option<RasterImage>> {
    let args = command
        .args
        .iter()
        .map(|arg| expand_preview_arg(arg, label, max_width_px, max_height_px))
        .collect::<Vec<_>>();
    let mut process = Command::new(&command.program);
    process
        .args(&args)
        .env("MDTUI_PREVIEW_LABEL", label)
        .env("MDTUI_PREVIEW_WIDTH_PX", max_width_px.to_string())
        .env("MDTUI_PREVIEW_HEIGHT_PX", max_height_px.to_string());
    if let Some(theme) = theme {
        process
            .env("MDTUI_PREVIEW_THEME_BG", theme.bg_hex())
            .env("MDTUI_PREVIEW_THEME_FG", rgb_hex(theme.fg))
            .env("MDTUI_PREVIEW_THEME_ACCENT", rgb_hex(theme.accent));
    }
    let mut child = process
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take()
        && let Err(error) = stdin.write_all(source.as_bytes())
        && error.kind() != ErrorKind::BrokenPipe
    {
        return Err(error);
    }

    let output = child.wait_with_output()?;
    if !output.status.success() || output.stdout.is_empty() {
        return Ok(None);
    }
    Ok(raster_local_image_rgba(
        &output.stdout,
        max_width_px,
        max_height_px,
    ))
}

pub fn queue_external_preview_render(
    label: &str,
    source_hash: u64,
    source: &str,
    width_cells: u16,
    height_cells: u16,
    theme: &Theme,
) -> bool {
    let Some(command) = external_preview_command_for_label(label) else {
        return false;
    };
    let key = PreviewRasterKey::new(
        label,
        source_hash,
        width_cells,
        height_cells,
        graphics_theme_hash(theme),
    );
    if cached_external_preview_raster_by_key(&key).is_some() {
        return false;
    }
    let Ok(mut jobs) = external_preview_jobs().lock() else {
        return false;
    };
    if !jobs.insert(key.clone()) {
        return false;
    }
    drop(jobs);

    let label = label.to_string();
    let source = source.to_string();
    let theme = theme.clone();
    thread::spawn(move || {
        let max_width_px = image_preview_width_px(width_cells);
        let max_height_px = image_preview_height_px(height_cells);
        if let Ok(Some(raster)) = render_external_preview_with_theme(
            &command,
            &label,
            &source,
            max_width_px,
            max_height_px,
            Some(&theme),
        ) {
            store_external_preview_raster_by_key(key.clone(), raster);
        }
        if let Ok(mut jobs) = external_preview_jobs().lock() {
            jobs.remove(&key);
        }
    });
    true
}

pub fn cached_external_preview_raster(
    label: &str,
    source_hash: u64,
    width_cells: u16,
    height_cells: u16,
    theme_hash: u64,
) -> Option<RasterImage> {
    cached_external_preview_raster_by_key(&PreviewRasterKey::new(
        label,
        source_hash,
        width_cells,
        height_cells,
        theme_hash,
    ))
}

pub fn store_external_preview_raster(
    label: &str,
    source_hash: u64,
    width_cells: u16,
    height_cells: u16,
    theme_hash: u64,
    raster: RasterImage,
) -> bool {
    store_external_preview_raster_by_key(
        PreviewRasterKey::new(label, source_hash, width_cells, height_cells, theme_hash),
        raster,
    )
}

pub fn clear_external_preview_cache() {
    if let Ok(mut cache) = external_preview_cache().lock() {
        cache.clear();
    }
    EXTERNAL_PREVIEW_CACHE_GENERATION.fetch_add(1, Ordering::Relaxed);
}

pub fn external_preview_cache_generation() -> u64 {
    EXTERNAL_PREVIEW_CACHE_GENERATION.load(Ordering::Relaxed)
}

pub fn graphics_theme_hash(theme: &Theme) -> u64 {
    let mut hasher = DefaultHasher::new();
    for rgb in [
        theme.bg,
        theme.bg_soft,
        theme.bg_raised,
        theme.line,
        theme.line_soft,
        theme.fg,
        theme.fg_dim,
        theme.fg_mute,
        theme.fg_faint,
        theme.accent,
        theme.red,
        theme.yellow,
        theme.green,
        theme.teal,
        theme.blue,
        theme.purple,
        theme.pink,
    ] {
        rgb_hex(rgb).hash(&mut hasher);
    }
    hasher.finish()
}

pub fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let signature = bytes.get(..8)?;
    if signature != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    if bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
    if width == 0 || height == 0 {
        return None;
    }
    Some((width, height))
}

fn stable_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PreviewRasterKey {
    label: String,
    source_hash: u64,
    width_cells: u16,
    height_cells: u16,
    theme_hash: u64,
}

impl PreviewRasterKey {
    fn new(
        label: &str,
        source_hash: u64,
        width_cells: u16,
        height_cells: u16,
        theme_hash: u64,
    ) -> Self {
        Self {
            label: label.to_string(),
            source_hash,
            width_cells,
            height_cells,
            theme_hash,
        }
    }
}

fn cached_external_preview_raster_by_key(key: &PreviewRasterKey) -> Option<RasterImage> {
    external_preview_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(key).cloned())
}

fn store_external_preview_raster_by_key(key: PreviewRasterKey, raster: RasterImage) -> bool {
    let Ok(mut cache) = external_preview_cache().lock() else {
        return false;
    };
    if cache.get(&key) == Some(&raster) {
        return false;
    }
    cache.insert(key, raster);
    EXTERNAL_PREVIEW_CACHE_GENERATION.fetch_add(1, Ordering::Relaxed);
    true
}

fn external_preview_cache() -> &'static Mutex<HashMap<PreviewRasterKey, RasterImage>> {
    static CACHE: OnceLock<Mutex<HashMap<PreviewRasterKey, RasterImage>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn external_preview_jobs() -> &'static Mutex<HashSet<PreviewRasterKey>> {
    static JOBS: OnceLock<Mutex<HashSet<PreviewRasterKey>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn env_preview_command(name: &str) -> Option<ExternalPreviewCommand> {
    env::var(name)
        .ok()
        .and_then(|value| ExternalPreviewCommand::from_command_line(&value))
}

fn preview_env_suffix(label: &str) -> String {
    let suffix = label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    if suffix.is_empty() {
        "DEFAULT".to_string()
    } else {
        suffix
    }
}

fn expand_preview_arg(arg: &str, label: &str, width_px: u32, height_px: u32) -> String {
    arg.replace("{label}", label)
        .replace("{width}", &width_px.to_string())
        .replace("{height}", &height_px.to_string())
}

fn rgb_hex(rgb: mdtui_render::Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb.0, rgb.1, rgb.2)
}

static EXTERNAL_PREVIEW_CACHE_GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardMethod {
    Native,
    Osc52Fallback,
}

pub fn copy_to_clipboard_or_osc52<W: Write>(
    writer: &mut W,
    text: &str,
) -> io::Result<ClipboardMethod> {
    copy_with_native(writer, text, copy_native_clipboard)
}

fn copy_native_clipboard(text: &str) -> bool {
    Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text.to_string()))
        .is_ok()
}

fn copy_with_native<W: Write>(
    writer: &mut W,
    text: &str,
    native_copy: impl FnOnce(&str) -> bool,
) -> io::Result<ClipboardMethod> {
    if native_copy(text) {
        return Ok(ClipboardMethod::Native);
    }
    write!(writer, "{}", osc52_copy(text))?;
    writer.flush()?;
    Ok(ClipboardMethod::Osc52Fallback)
}

pub struct TerminalGuard {
    pub terminal: TuiTerminal,
    pub capabilities: TerminalCapabilities,
    active: bool,
}

impl TerminalGuard {
    pub fn enter(theme: &Theme) -> Result<Self> {
        install_panic_hook();
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        )?;
        write!(stdout, "{}", osc_set_background(&theme.bg_hex()))?;
        stdout.flush()?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self {
            terminal,
            capabilities: TerminalCapabilities::detect(),
            active: true,
        })
    }

    pub fn leave(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        write!(self.terminal.backend_mut(), "{}", osc_reset_background())?;
        execute!(
            self.terminal.backend_mut(),
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen
        )?;
        disable_raw_mode()?;
        self.terminal.show_cursor()?;
        self.active = false;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.leave();
    }
}

fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = write!(io::stdout(), "{}", osc_reset_background());
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use image::{ColorType, ImageEncoder, codecs::png::PngEncoder};

    use super::*;

    #[test]
    fn osc_background_protocol_uses_11_and_111_with_st() {
        assert_eq!(osc_set_background("#1d1b1a"), "\x1b]11;#1d1b1a\x1b\\");
        assert_eq!(osc_reset_background(), "\x1b]111\x1b\\");
    }

    #[test]
    fn osc52_copy_encodes_clipboard_payload() {
        assert_eq!(osc52_copy("hi"), "\x1b]52;c;aGk=\x1b\\");
    }

    #[test]
    fn clipboard_copy_uses_native_when_available() {
        let mut out = Vec::new();
        let method =
            copy_with_native(&mut out, "hi", |_| true).unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(method, ClipboardMethod::Native);
        assert!(out.is_empty());
    }

    #[test]
    fn clipboard_copy_falls_back_to_osc52() {
        let mut out = Vec::new();
        let method =
            copy_with_native(&mut out, "hi", |_| false).unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(method, ClipboardMethod::Osc52Fallback);
        assert_eq!(out, osc52_copy("hi").into_bytes());
    }

    #[test]
    fn detects_kitty_graphics_capability() {
        let capabilities =
            TerminalCapabilities::detect_from([("TERM", "xterm-kitty"), ("TERM_PROGRAM", "kitty")]);
        assert!(capabilities.kitty_graphics);

        let wezterm = TerminalCapabilities::detect_from([("TERM_PROGRAM", "WezTerm")]);
        assert!(wezterm.kitty_graphics);

        let plain = TerminalCapabilities::detect_from([("TERM", "xterm-256color")]);
        assert!(!plain.kitty_graphics);
    }

    #[test]
    fn terminal_matrix_selects_graphics_or_fallback_paths() {
        let cases = [
            (
                "Ghostty",
                vec![("TERM_PROGRAM", "ghostty"), ("TERM", "xterm-ghostty")],
                TerminalRenderPath::KittyGraphics,
            ),
            (
                "Kitty",
                vec![("KITTY_WINDOW_ID", "1"), ("TERM", "xterm-kitty")],
                TerminalRenderPath::KittyGraphics,
            ),
            (
                "WezTerm",
                vec![("TERM_PROGRAM", "WezTerm"), ("TERM", "xterm-256color")],
                TerminalRenderPath::KittyGraphics,
            ),
            (
                "iTerm2",
                vec![("TERM_PROGRAM", "iTerm.app"), ("TERM", "xterm-256color")],
                TerminalRenderPath::StyledTextFallback,
            ),
            (
                "basic ANSI",
                vec![("TERM", "linux")],
                TerminalRenderPath::StyledTextFallback,
            ),
        ];

        for (name, vars, expected) in cases {
            let capabilities = TerminalCapabilities::detect_from(vars);
            assert_eq!(capabilities.render_path(), expected, "{name}");
        }
    }

    #[test]
    fn kitty_graphics_commands_use_standard_st() {
        let image = KittyImage {
            id: 7,
            width_px: 2,
            height_px: 1,
            z_index: -10,
        };

        assert_eq!(
            kitty_transmit_rgba(image, &[255, 0, 0, 255]),
            "\x1b_Ga=t,f=32,s=2,v=1,i=7,q=2,z=-10;/wAA/w==\x1b\\"
        );
        assert_eq!(
            kitty_transmit_png(image, b"png"),
            "\x1b_Ga=t,f=100,s=2,v=1,i=7,q=2,z=-10;cG5n\x1b\\"
        );
        assert_eq!(
            kitty_create_virtual_placement(image, 4, 2),
            "\x1b_Ga=p,U=1,i=7,c=4,r=2,q=2,z=-10;\x1b\\"
        );
        assert_eq!(kitty_delete_image(7), "\x1b_Ga=d,d=I,i=7,q=2;\x1b\\");
    }

    #[test]
    fn png_dimensions_reads_ihdr_size() {
        let png = [
            0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 13, b'I', b'H', b'D', b'R',
            0, 0, 0, 16, 0, 0, 0, 8,
        ];

        assert_eq!(png_dimensions(&png), Some((16, 8)));
        assert_eq!(png_dimensions(b"not a png"), None);
    }

    #[test]
    fn local_image_raster_decodes_and_scales_to_bounds() {
        let rgba = vec![255_u8; 16 * 8 * 4];
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(&rgba, 16, 8, ColorType::Rgba8.into())
            .unwrap_or_else(|error| panic!("{error}"));

        let raster =
            raster_local_image_rgba(&png, 8, 8).unwrap_or_else(|| panic!("missing raster"));

        assert_eq!(raster.width_px, 8);
        assert_eq!(raster.height_px, 4);
        assert_eq!(raster.rgba.len(), 8 * 4 * 4);
    }

    #[test]
    fn external_preview_command_line_uses_program_and_args_without_shell() {
        let command = ExternalPreviewCommand::from_command_line(
            "/usr/bin/renderer --format png --width {width}",
        )
        .unwrap_or_else(|| panic!("missing command"));

        assert_eq!(command.program, PathBuf::from("/usr/bin/renderer"));
        assert_eq!(
            command.args,
            vec![
                "--format".to_string(),
                "png".to_string(),
                "--width".to_string(),
                "{width}".to_string(),
            ]
        );
    }

    #[test]
    fn external_preview_renderer_reads_stdin_and_decodes_stdout() {
        let test_id = format!("{}-external-preview", std::process::id());
        let temp = env::temp_dir();
        let png_path = temp.join(format!("mdtui-{test_id}.png"));
        let stdin_path = temp.join(format!("mdtui-{test_id}.stdin"));
        let script_path = temp.join(format!("mdtui-{test_id}.sh"));

        let rgba = vec![255_u8; 16 * 8 * 4];
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(&rgba, 16, 8, ColorType::Rgba8.into())
            .unwrap_or_else(|error| panic!("{error}"));
        fs::write(&png_path, &png).unwrap_or_else(|error| panic!("{error}"));
        fs::write(
            &script_path,
            format!(
                "cat > {}\ncat {}\n",
                stdin_path.display(),
                png_path.display()
            ),
        )
        .unwrap_or_else(|error| panic!("{error}"));

        let command = ExternalPreviewCommand {
            program: PathBuf::from("/bin/sh"),
            args: vec![script_path.to_string_lossy().to_string()],
        };
        let raster = render_external_preview(&command, "math", "x + y", 8, 8)
            .unwrap_or_else(|error| panic!("{error}"))
            .unwrap_or_else(|| panic!("missing external preview"));

        assert_eq!(raster.width_px, 8);
        assert_eq!(raster.height_px, 4);
        assert_eq!(
            fs::read_to_string(&stdin_path).unwrap_or_else(|error| panic!("{error}")),
            "x + y"
        );

        let _ = fs::remove_file(png_path);
        let _ = fs::remove_file(stdin_path);
        let _ = fs::remove_file(script_path);
    }

    #[test]
    fn external_preview_raster_cache_tracks_generation() {
        clear_external_preview_cache();
        let generation = external_preview_cache_generation();
        let theme_hash = graphics_theme_hash(&Theme::ghostty_default_dark());
        let changed = store_external_preview_raster(
            "math",
            42,
            12,
            4,
            theme_hash,
            RasterImage {
                width_px: 2,
                height_px: 1,
                rgba: vec![1, 2, 3, 4, 5, 6, 7, 8],
            },
        );

        assert!(changed);
        assert!(external_preview_cache_generation() > generation);
        assert_eq!(
            cached_external_preview_raster("math", 42, 12, 4, theme_hash)
                .map(|raster| (raster.width_px, raster.height_px)),
            Some((2, 1))
        );
        clear_external_preview_cache();
    }

    #[test]
    fn kitty_graphics_cache_reuses_ids_by_key() {
        let mut cache = KittyGraphicsCache::new();
        let key = GraphicsCacheKey::heading(1, "Title", 80, 42);
        let first = cache.get_or_insert(key.clone(), 100, 20, -10);
        let second = cache.get_or_insert(key.clone(), 100, 20, -10);
        let other = cache.get_or_insert(GraphicsCacheKey::heading(2, "Title", 80, 42), 80, 16, -10);

        assert_eq!(first.id, second.id);
        assert_ne!(first.id, other.id);
        assert_eq!(cache.get(&key), Some(first));
    }

    #[test]
    fn heading_raster_matches_expected_dimensions() {
        let theme = Theme::ghostty_default_dark();
        let rgba = raster_heading_rgba("Title", 1, 10, &theme);

        assert_eq!(
            rgba.len(),
            heading_width_px(10) as usize * heading_height_px(1) as usize * 4
        );
        assert!(rgba.chunks_exact(4).any(|pixel| pixel[3] > 0));
        assert_eq!(heading_scale_milli(1), 1500);
        assert_eq!(heading_scale_milli(2), 1250);
        assert_eq!(heading_height_px(1), 24);
        assert_eq!(heading_height_px(2), 20);
    }

    #[test]
    fn preview_raster_matches_placeholder_bounds() {
        let theme = Theme::ghostty_default_dark();
        let rgba = raster_preview_rgba("math", 42, 12, 4, &theme);

        assert_eq!(
            rgba.len(),
            image_preview_width_px(12) as usize * image_preview_height_px(4) as usize * 4
        );
        assert!(rgba.chunks_exact(4).any(|pixel| pixel[3] > 0));
    }
}
