use std::collections::HashSet;
use std::io::{self, stdout};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use image::DynamicImage;
use ratatui::layout::Rect;
use ratatui::DefaultTerminal;
use ratatui_image::picker::Picker;

use crate::cache::PageCache;
use crate::input;
use crate::pdf::PdfDocument;
use crate::view;

pub struct AppConfig {
    pub dark_mode: bool,
    pub fullscreen: bool,
    pub start_page: usize,
    pub layout: PageLayout,
    pub text_mode: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PageLayout {
    Single,
    Dual,
    Triple,
}

impl PageLayout {
    pub const fn pages_across(self) -> usize {
        match self {
            Self::Single => 1,
            Self::Dual => 2,
            Self::Triple => 3,
        }
    }

    pub const fn cycle(self) -> Self {
        match self {
            Self::Single => Self::Dual,
            Self::Dual => Self::Triple,
            Self::Triple => Self::Single,
        }
    }
}

pub enum Message {
    Quit,
    NextPage,
    PrevPage,
    FirstPage,
    LastPage,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
    CycleLayout,
    ToggleDarkMode,
    ToggleFullscreen,
    ToggleTextMode,
    EnterGoto,
    GotoInput(char),
    GotoBackspace,
    GotoConfirm,
    GotoCancel,
}

struct RenderRequest {
    idx: usize,
    scale: f32,
}

struct RenderResult {
    idx: usize,
    scale: f32,
    img: DynamicImage,
}

#[allow(clippy::struct_excessive_bools)]
pub struct App {
    pub(crate) cache: PageCache,
    pub(crate) picker: Option<Picker>,
    pub(crate) current_page: usize,
    pub(crate) page_count: usize,
    pub(crate) zoom: f32,
    pub(crate) pan_x: f32,
    pub(crate) pan_y: f32,
    pub(crate) layout: PageLayout,
    pub(crate) dark_mode: bool,
    pub(crate) fullscreen: bool,
    pub(crate) goto_mode: bool,
    pub(crate) goto_input: String,
    pub(crate) text_mode: bool,
    pub(crate) text_scroll: usize,
    term_cols: u16,
    term_rows: u16,
    page_bounds: (f32, f32),
    pdf_path: String,
    text_pdf: Option<PdfDocument>,
    render_tx: Option<Sender<RenderRequest>>,
    render_rx: Option<Receiver<RenderResult>>,
    pending: HashSet<usize>,
    should_quit: bool,
}

const PAN_STEP: f32 = 0.15;
const ZOOM_STEP: f32 = 0.10;

impl App {
    pub fn new(
        path: &str,
        picker: Option<Picker>,
        term_cols: u16,
        term_rows: u16,
        config: &AppConfig,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let pdf = PdfDocument::open(path)?;
        let page_count = pdf.page_count();
        if page_count == 0 {
            return Err("PDF has no pages".into());
        }
        let page_bounds = pdf.page_bounds(0).unwrap_or((612.0, 792.0));
        drop(pdf);

        let (render_tx, render_rx) = if picker.is_some() {
            let (req_tx, req_rx) = mpsc::channel::<RenderRequest>();
            let (res_tx, res_rx) = mpsc::channel::<RenderResult>();
            let shared_rx = Arc::new(Mutex::new(req_rx));

            let num_threads = std::thread::available_parallelism()
                .map(|n| n.get().min(4))
                .unwrap_or(2);

            for _ in 0..num_threads {
                let rx = Arc::clone(&shared_rx);
                let tx = res_tx.clone();
                let p = path.to_string();
                std::thread::spawn(move || {
                    let pdf = PdfDocument::open(&p).expect("render worker: failed to open PDF");
                    loop {
                        let req = {
                            let guard = rx.lock().unwrap();
                            guard.recv()
                        };
                        match req {
                            Ok(r) => {
                                if let Ok(img) = pdf.render_page(r.idx, r.scale) {
                                    if tx
                                        .send(RenderResult {
                                            idx: r.idx,
                                            scale: r.scale,
                                            img,
                                        })
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }
                });
            }
            drop(res_tx);
            (Some(req_tx), Some(res_rx))
        } else {
            (None, None)
        };

        let start_page = config.start_page.min(page_count.saturating_sub(1));
        let text_mode = config.text_mode || picker.is_none();

        Ok(Self {
            cache: PageCache::new(),
            picker,
            current_page: start_page,
            page_count,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            layout: config.layout,
            dark_mode: config.dark_mode,
            fullscreen: config.fullscreen,
            term_cols,
            term_rows,
            goto_mode: false,
            goto_input: String::new(),
            text_mode,
            text_scroll: 0,
            page_bounds,
            pdf_path: path.to_string(),
            text_pdf: None,
            render_tx,
            render_rx,
            pending: HashSet::new(),
            should_quit: false,
        })
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        if !self.text_mode {
            self.request_visible_pages();
        }
        let mut dirty = true;

        while !self.should_quit {
            if !self.text_mode && self.process_render_results() {
                dirty = true;
            }

            if dirty {
                execute!(stdout(), BeginSynchronizedUpdate)?;
                terminal.draw(|frame| view::draw(frame, self))?;
                execute!(stdout(), EndSynchronizedUpdate)?;
                dirty = false;
            }

            let timeout = if self.text_mode {
                Duration::from_secs(60)
            } else {
                let has_pending = self.has_pending_visible();
                let needs_prewarm = !has_pending && self.has_nearby_unwarmed_protocol();
                if has_pending {
                    Duration::from_millis(16)
                } else if needs_prewarm {
                    Duration::from_millis(1)
                } else {
                    Duration::from_secs(60)
                }
            };

            if event::poll(timeout)? {
                // Drain ALL pending events before redrawing so held-key
                // repeats don't pile up behind slow frames.
                loop {
                    match event::read()? {
                        Event::Key(key) if key.kind == KeyEventKind::Press => {
                            let msg = if self.goto_mode {
                                input::key_to_goto_message(key)
                            } else {
                                input::key_to_message(key)
                            };
                            if let Some(msg) = msg {
                                self.update(msg);
                                dirty = true;
                            }
                        }
                        Event::Resize(cols, rows) => {
                            self.term_cols = cols;
                            self.term_rows = rows;
                            self.cache.clear();
                            self.pending.clear();
                            dirty = true;
                        }
                        _ => {}
                    }
                    // Keep draining while more events are buffered
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
                if dirty && !self.text_mode {
                    self.request_visible_pages();
                    self.cache.evict_distant(self.current_page, 15);
                }
            } else if !self.text_mode && self.has_nearby_unwarmed_protocol() {
                self.prewarm_one_nearby_protocol();
            }
        }

        Ok(())
    }

    /// Usable row count (subtracts 1 for the status bar unless fullscreen).
    fn usable_rows(&self) -> u16 {
        if self.fullscreen {
            self.term_rows
        } else {
            self.term_rows.saturating_sub(1)
        }
    }

    /// Ensure extracted text for `page_idx` is cached.
    pub(crate) fn ensure_page_text(&mut self, page_idx: usize) {
        if self.cache.has_text(page_idx) {
            return;
        }
        if self.text_pdf.is_none() {
            self.text_pdf = PdfDocument::open(&self.pdf_path).ok();
        }
        let text = self
            .text_pdf
            .as_ref()
            .and_then(|pdf| pdf.extract_text(page_idx).ok())
            .unwrap_or_default();
        self.cache.insert_text(page_idx, text);
    }

    fn process_render_results(&mut self) -> bool {
        let Some(ref render_rx) = self.render_rx else {
            return false;
        };
        let Some(ref picker) = self.picker else {
            return false;
        };

        let current_scale = self.render_scale();
        let mut received = false;

        while let Ok(r) = render_rx.try_recv() {
            self.pending.remove(&r.idx);
            if (r.scale - current_scale).abs() < 0.01 {
                self.cache.insert_image(r.idx, r.scale, r.img);
                received = true;
            }
        }

        if received {
            let n = self.layout.pages_across();
            let per_page_width = self.term_cols / n as u16;
            let usable = self.usable_rows();

            // Pre-warm protocols for visible pages + a few ahead for smooth navigation
            let prewarm_start = self.current_page;
            let prewarm_end = (self.current_page + n + 3).min(self.page_count);
            for idx in prewarm_start..prewarm_end {
                let Some((w, h)) = self.cache.image_dims(idx) else {
                    continue;
                };
                let page_area = Rect::new(0, 0, per_page_width, usable);
                let render_area = view::aligned_image_area(
                    w,
                    h,
                    page_area,
                    picker.font_size(),
                    self.zoom,
                    view::HAlign::Center,
                );
                self.cache.get_protocol(
                    idx,
                    self.dark_mode,
                    self.zoom,
                    (self.pan_x, self.pan_y),
                    picker,
                    render_area,
                );
            }
        }
        received
    }

    fn has_pending_visible(&self) -> bool {
        if self.picker.is_none() {
            return false;
        }
        let scale = self.render_scale();
        let n = self.layout.pages_across();
        (0..n).any(|i| {
            let idx = self.current_page + i;
            idx < self.page_count && !self.cache.has_image_at_scale(idx, scale)
        })
    }

    pub fn render_scale(&self) -> f32 {
        let Some(ref picker) = self.picker else {
            return 1.0;
        };
        let (fw, fh) = picker.font_size();
        let pages_across = self.layout.pages_across() as f64;
        let area_px_w = (f64::from(self.term_cols) / pages_across) * f64::from(fw);
        let area_px_h = f64::from(self.usable_rows()) * f64::from(fh);

        let (page_w, page_h) = self.page_bounds;
        let fit = (area_px_w / f64::from(page_w)).min(area_px_h / f64::from(page_h)) as f32;
        // Render at higher resolution when zoomed in so cropping stays sharp
        fit * self.zoom.max(1.0)
    }

    fn request_visible_pages(&mut self) {
        if self.render_tx.is_none() {
            return;
        }
        let scale = self.render_scale();
        let n = self.layout.pages_across();

        for i in 0..n {
            let idx = self.current_page + i;
            if idx < self.page_count {
                self.request_page(idx, scale);
            }
        }

        let visible_end = self.current_page + n;
        for offset in 0..5 {
            let ahead = visible_end + offset;
            if ahead < self.page_count {
                self.request_page(ahead, scale);
            }
            if let Some(behind) = self.current_page.checked_sub(offset + 1) {
                self.request_page(behind, scale);
            }
        }
    }

    /// Check if any nearby page has a cached image but no protocol yet.
    fn has_nearby_unwarmed_protocol(&self) -> bool {
        if self.picker.is_none() {
            return false;
        }
        let n = self.layout.pages_across();
        let start = self.current_page.saturating_sub(5);
        let end = (self.current_page + n + 5).min(self.page_count);
        (start..end).any(|idx| {
            self.cache.image_dims(idx).is_some() && !self.cache.has_protocol(idx, self.dark_mode)
        })
    }

    /// Generate one protocol for a nearby page during idle time.
    fn prewarm_one_nearby_protocol(&mut self) {
        let Some(ref picker) = self.picker else {
            return;
        };
        let n = self.layout.pages_across();
        let per_page_width = self.term_cols / n as u16;
        let usable = self.usable_rows();

        // Prioritise pages ahead, then behind
        let start = self.current_page;
        let end = (self.current_page + n + 5).min(self.page_count);
        let behind_start = self.current_page.saturating_sub(5);

        for idx in (start..end).chain(behind_start..self.current_page) {
            if self.cache.image_dims(idx).is_some() && !self.cache.has_protocol(idx, self.dark_mode)
            {
                let (w, h) = self.cache.image_dims(idx).unwrap();
                let page_area = Rect::new(0, 0, per_page_width, usable);
                let render_area = view::aligned_image_area(
                    w,
                    h,
                    page_area,
                    picker.font_size(),
                    self.zoom,
                    view::HAlign::Center,
                );
                self.cache.get_protocol(
                    idx,
                    self.dark_mode,
                    self.zoom,
                    (self.pan_x, self.pan_y),
                    picker,
                    render_area,
                );
                return;
            }
        }
    }

    fn request_page(&mut self, idx: usize, scale: f32) {
        let Some(ref render_tx) = self.render_tx else {
            return;
        };
        if !self.cache.has_image_at_scale(idx, scale)
            && !self.pending.contains(&idx)
            && render_tx.send(RenderRequest { idx, scale }).is_ok()
        {
            self.pending.insert(idx);
        }
    }

    fn reset_pan(&mut self) {
        self.pan_x = 0.0;
        self.pan_y = 0.0;
    }

    #[allow(clippy::too_many_lines)]
    fn update(&mut self, msg: Message) {
        match msg {
            Message::Quit => self.should_quit = true,

            Message::NextPage => {
                let max = self.page_count.saturating_sub(1);
                self.current_page = (self.current_page + 1).min(max);
                self.text_scroll = 0;
            }
            Message::PrevPage => {
                self.current_page = self.current_page.saturating_sub(1);
                self.text_scroll = 0;
            }
            Message::FirstPage => {
                self.current_page = 0;
                self.text_scroll = 0;
            }
            Message::LastPage => {
                self.current_page = self.page_count.saturating_sub(1);
                self.text_scroll = 0;
            }

            Message::ZoomIn => {
                self.zoom = (self.zoom + ZOOM_STEP).min(4.0);
                self.pending.clear();
                self.reset_pan();
            }
            Message::ZoomOut => {
                self.zoom = (self.zoom - ZOOM_STEP).max(0.25);
                self.pending.clear();
                self.reset_pan();
            }
            Message::ZoomReset => {
                self.zoom = 1.0;
                self.pending.clear();
                self.reset_pan();
            }

            Message::ScrollUp => {
                if self.text_mode {
                    self.text_scroll = self.text_scroll.saturating_sub(3);
                } else if self.zoom > 1.0 {
                    self.pan_y = (self.pan_y - PAN_STEP).max(-1.0);
                }
            }
            Message::ScrollDown => {
                if self.text_mode {
                    self.text_scroll = self.text_scroll.saturating_add(3);
                } else if self.zoom > 1.0 {
                    self.pan_y = (self.pan_y + PAN_STEP).min(1.0);
                }
            }
            Message::ScrollLeft => {
                if self.zoom > 1.0 {
                    self.pan_x = (self.pan_x - PAN_STEP).max(-1.0);
                }
            }
            Message::ScrollRight => {
                if self.zoom > 1.0 {
                    self.pan_x = (self.pan_x + PAN_STEP).min(1.0);
                }
            }

            Message::CycleLayout => {
                self.layout = self.layout.cycle();
                self.cache.invalidate_protocols();
            }
            Message::ToggleDarkMode => self.dark_mode = !self.dark_mode,
            Message::ToggleFullscreen => {
                self.fullscreen = !self.fullscreen;
                self.cache.clear();
                self.pending.clear();
            }
            Message::ToggleTextMode => {
                if self.picker.is_some() {
                    self.text_mode = !self.text_mode;
                    self.text_scroll = 0;
                    if !self.text_mode {
                        self.cache.clear();
                        self.pending.clear();
                        self.request_visible_pages();
                    }
                }
            }

            Message::EnterGoto => {
                self.goto_mode = true;
                self.goto_input.clear();
            }
            Message::GotoInput(c) => {
                if self.goto_input.len() < 10 {
                    self.goto_input.push(c);
                }
            }
            Message::GotoBackspace => {
                self.goto_input.pop();
            }
            Message::GotoConfirm => {
                if let Ok(page) = self.goto_input.parse::<usize>() {
                    if page >= 1 && page <= self.page_count {
                        self.current_page = page - 1;
                    }
                }
                self.goto_mode = false;
                self.goto_input.clear();
                self.text_scroll = 0;
            }
            Message::GotoCancel => {
                self.goto_mode = false;
                self.goto_input.clear();
            }
        }
    }
}
