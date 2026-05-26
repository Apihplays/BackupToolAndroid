use std::collections::HashMap;
use std::io::Read;

use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::adb::client::AdbClient;

/// Maximum bytes to read from device for thumbnail (512KB).
const MAX_THUMBNAIL_BYTES: usize = 512 * 1024;

/// Maximum cache entries to keep in memory.
const MAX_CACHE_ENTRIES: usize = 20;

/// Image extensions we can decode thumbnails for.
const THUMBNAIL_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "webp",
];

/// A decoded thumbnail as a grid of RGB colors.
#[derive(Clone)]
pub struct ThumbnailGrid {
    /// Pixel rows — each row is a Vec of Color::Rgb values.
    pub pixels: Vec<Vec<Color>>,
    pub width: u16,
    pub height: u16,
}

/// LRU-ish thumbnail cache to avoid refetching.
pub struct ThumbnailCache {
    cache: HashMap<String, Option<ThumbnailGrid>>,
    order: Vec<String>,
}

impl ThumbnailCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// Get a cached thumbnail (returns None if not cached).
    pub fn get(&self, path: &str) -> Option<&Option<ThumbnailGrid>> {
        self.cache.get(path)
    }

    /// Insert a thumbnail into the cache.
    pub fn insert(&mut self, path: String, grid: Option<ThumbnailGrid>) {
        if self.cache.contains_key(&path) {
            // Move to end of order
            self.order.retain(|p| p != &path);
        } else if self.order.len() >= MAX_CACHE_ENTRIES {
            // Evict oldest
            if let Some(oldest) = self.order.first().cloned() {
                self.cache.remove(&oldest);
                self.order.remove(0);
            }
        }
        self.order.push(path.clone());
        self.cache.insert(path, grid);
    }
}

/// Check if a file name has a thumbnail-supported extension.
pub fn is_thumbnail_supported(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    THUMBNAIL_EXTENSIONS.contains(&ext.as_str())
}

/// Fetch raw image bytes from device (limited to MAX_THUMBNAIL_BYTES).
pub fn fetch_thumbnail_bytes(client: &AdbClient, remote_path: &str) -> Option<Vec<u8>> {
    // Use dd to limit bytes read, avoiding loading huge RAW files
    let cmd = format!(
        "dd if='{}' bs=4096 count=128 2>/dev/null",
        remote_path
    );

    let mut child = client.shell_stream(&cmd).ok()?;
    let stdout = child.stdout.take()?;

    let mut reader = std::io::BufReader::new(stdout);
    let mut buf = Vec::with_capacity(MAX_THUMBNAIL_BYTES);
    let mut tmp = [0u8; 8192];

    loop {
        match reader.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.len() >= MAX_THUMBNAIL_BYTES {
                    buf.truncate(MAX_THUMBNAIL_BYTES);
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }

    let _ = child.kill();
    let _ = child.wait();

    if buf.len() > 100 {
        Some(buf)
    } else {
        None
    }
}

/// Decode image bytes into a pixel grid scaled to fit terminal dimensions.
pub fn decode_to_grid(image_bytes: &[u8], max_width: u16, max_height: u16) -> Option<ThumbnailGrid> {
    use image::ImageReader;
    use std::io::Cursor;

    let reader = ImageReader::new(Cursor::new(image_bytes))
        .with_guessed_format()
        .ok()?;
    let img = reader.decode().ok()?;

    // Each terminal cell = 1 char wide, 2 pixels tall (using half-block)
    let target_w = max_width as u32;
    let target_h = (max_height as u32) * 2; // 2 pixels per row of text

    let resized = img.resize(target_w, target_h, image::imageops::FilterType::Triangle);
    let rgb = resized.to_rgb8();
    let (w, h) = rgb.dimensions();

    let mut pixels = Vec::new();
    for y in 0..h {
        let mut row = Vec::new();
        for x in 0..w {
            let p = rgb.get_pixel(x, y);
            row.push(Color::Rgb(p[0], p[1], p[2]));
        }
        pixels.push(row);
    }

    Some(ThumbnailGrid {
        pixels,
        width: w as u16,
        height: h as u16,
    })
}

/// Render a thumbnail grid using Unicode half-block characters.
/// Each character cell represents 2 vertical pixels:
///   - Foreground color = top pixel
///   - Background color = bottom pixel
///   - Character = '▀' (upper half block)
pub fn render_thumbnail(frame: &mut Frame, area: Rect, grid: &ThumbnailGrid) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(" 🖼️ Preview ")
        .title_style(Style::default().fg(Color::Magenta))
        .style(Style::default().bg(Color::Rgb(15, 15, 25)));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if grid.pixels.is_empty() || inner.width == 0 || inner.height == 0 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Process 2 pixel rows at a time
    let mut y = 0;
    while y < grid.height && (lines.len() as u16) < inner.height {
        let mut spans: Vec<Span> = Vec::new();

        for x in 0..grid.width.min(inner.width) {
            let top = grid.pixels.get(y as usize)
                .and_then(|row| row.get(x as usize))
                .copied()
                .unwrap_or(Color::Rgb(15, 15, 25));

            let bottom = grid.pixels.get((y + 1) as usize)
                .and_then(|row| row.get(x as usize))
                .copied()
                .unwrap_or(Color::Rgb(15, 15, 25));

            spans.push(Span::styled("▀", Style::default().fg(top).bg(bottom)));
        }

        lines.push(Line::from(spans));
        y += 2;
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

/// Render a "no preview" placeholder.
pub fn render_no_preview(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(" 🖼️ Preview ")
        .title_style(Style::default().fg(Color::DarkGray))
        .style(Style::default().bg(Color::Rgb(15, 15, 25)));

    let msg = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Select an image to preview",
            Style::default().fg(Color::DarkGray),
        )),
    ])
    .block(block);

    frame.render_widget(msg, area);
}
