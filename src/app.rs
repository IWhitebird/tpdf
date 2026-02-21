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

#[derive(Clone, Copy, PartialEq)]
pub enum PageLayout {
    Single,
    Dual,
    Triple,
}

impl PageLayout {
    pub fn pages_across(self) -> usize {
        match self {
            Self::Single => 1,
            Self::Dual => 2,
            Self::Triple => 3,
        }
    }

    pub fn cycle(self) -> Self {
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

pub struct App {
    pub cache: PageCache,
    pub picker: Picker,
    pub current_page: usize,
    pub page_count: usize,
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
    pub layout: PageLayout,
    pub dark_mode: bool,
    pub term_cols: u16,
    pub term_rows: u16,
    pub fullscreen: bool,
    pub goto_mode: bool,
    pub goto_input: String,
    page_bounds: (f32, f32),
    render_tx: Sender<RenderRequest>,
    render_rx: Receiver<RenderResult>,
    pending: HashSet<usize>,
    should_quit: bool,
}

const PAN_STEP: f32 = 0.15;
const ZOOM_STEP: f32 = 0.10;

impl App {
    pub fn new(
        path: &str,
        picker: Picker,
        term_cols: u16,
        term_rows: u16,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Open PDF briefly to get metadata
        let pdf = PdfDocument::open(path)?;
        let page_count = pdf.page_count();
        if page_count == 0 {
            return Err("PDF has no pages".into());
        }
        let page_bounds = pdf.page_bounds(0).unwrap_or((612.0, 792.0));
        drop(pdf);

        // ---------- render thread pool with shared work queue ----------
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
                let pdf =
                    PdfDocument::open(&p).expect("render worker: failed to open PDF");
                loop {
                    // Lock only long enough to pull one request off the queue
                    let req = {
                        let guard = rx.lock().unwrap();
                        guard.recv()
                    };
                    match req {
                        Ok(r) => {
                            if let Ok(img) = pdf.render_page(r.idx, r.scale) {
                                if tx
                                    .send(RenderResult { idx: r.idx, scale: r.scale, img })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }
                        Err(_) => break, // sender dropped → clean exit
                    }
                }
            });
        }
        // Drop the original res_tx so the channel closes when all workers exit
        drop(res_tx);

        Ok(Self {
            cache: PageCache::new(),
            picker,
            current_page: 0,
            page_count,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            layout: PageLayout::Single,
            dark_mode: false,
            fullscreen: false,
            term_cols,
            term_rows,
            goto_mode: false,
            goto_input: String::new(),
            page_bounds,
            render_tx: req_tx,
            render_rx: res_rx,
            pending: HashSet::new(),
            should_quit: false,
        })
    }

    // ----------------------------------------------------------------
    //  Main event loop
    // ----------------------------------------------------------------
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        self.request_visible_pages();
        let mut dirty = true;

        while !self.should_quit {
            // Drain every completed render and eagerly build protocols
            if self.process_render_results() {
                dirty = true;
            }

            if dirty {
                execute!(stdout(), BeginSynchronizedUpdate)?;
                terminal.draw(|frame| view::draw(frame, self))?;
                execute!(stdout(), EndSynchronizedUpdate)?;
                dirty = false;
            }

            // Short poll while visible pages are still pending so we can
            // show them the instant they arrive.  Long poll when idle.
            let timeout = if self.has_pending_visible() {
                Duration::from_millis(16) // ~60 fps
            } else {
                Duration::from_secs(60)
            };

            if event::poll(timeout)? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        let msg = if self.goto_mode {
                            input::key_to_goto_message(key)
                        } else {
                            input::key_to_message(key)
                        };
                        if let Some(msg) = msg {
                            self.update(msg);
                            self.request_visible_pages();
                            dirty = true;
                        }
                    }
                    Event::Resize(cols, rows) => {
                        self.term_cols = cols;
                        self.term_rows = rows;
                        self.cache.clear();
                        self.pending.clear();
                        self.request_visible_pages();
                        dirty = true;
                    }
                    _ => {}
                }
            }
        }

        // render_tx is dropped here → workers exit automatically
        Ok(())
    }

    // ----------------------------------------------------------------
    //  Render-result processing
    // ----------------------------------------------------------------
    fn process_render_results(&mut self) -> bool {
        let current_scale = self.render_scale();
        let mut received = false;

        while let Ok(r) = self.render_rx.try_recv() {
            self.pending.remove(&r.idx);
            // Discard results rendered at an outdated scale (e.g. after resize)
            if (r.scale - current_scale).abs() < 0.01 {
                self.cache.insert_image(r.idx, r.scale, r.img);
                received = true;
            }
        }

        // Eagerly encode terminal protocols for every visible page *now*,
        // so that draw() never has to block on encoding.
        if received {
            let n = self.layout.pages_across();
            let usable_rows = if self.fullscreen {
                self.term_rows
            } else {
                self.term_rows.saturating_sub(1)
            };
            let per_page_width = self.term_cols / n as u16;

            for i in 0..n {
                let idx = self.current_page + i;
                if idx < self.page_count {
                    let page_area = Rect::new(0, 0, per_page_width, usable_rows);
                    let render_area = if let Some((w, h)) = self.cache.image_dims(idx) {
                        view::aligned_image_area(
                            w, h, page_area, self.picker.font_size(), self.zoom,
                            view::HAlign::Center,
                        )
                    } else {
                        continue;
                    };
                    // get_protocol creates + caches the protocol if absent
                    self.cache.get_protocol(
                        idx,
                        self.dark_mode,
                        self.zoom,
                        (self.pan_x, self.pan_y),
                        &self.picker,
                        render_area,
                    );
                }
            }
        }
        received
    }

    /// Are any of the currently visible pages still waiting on the render pool?
    fn has_pending_visible(&self) -> bool {
        let scale = self.render_scale();
        let n = self.layout.pages_across();
        (0..n).any(|i| {
            let idx = self.current_page + i;
            idx < self.page_count && !self.cache.has_image_at_scale(idx, scale)
        })
    }

    // ----------------------------------------------------------------
    //  Adaptive render scale
    // ----------------------------------------------------------------
    pub fn render_scale(&self) -> f32 {
        let (fw, fh) = self.picker.font_size();
        let usable_rows = if self.fullscreen {
            self.term_rows
        } else {
            self.term_rows.saturating_sub(1)
        };
        let pages_across = self.layout.pages_across() as f64;
        let area_px_w = (self.term_cols as f64 / pages_across) * fw as f64;
        let area_px_h = usable_rows as f64 * fh as f64;

        let (page_w, page_h) = self.page_bounds;
        let fit = (area_px_w / page_w as f64).min(area_px_h / page_h as f64) as f32;
        // Render at higher resolution when zoomed in so cropping stays sharp
        fit * self.zoom.max(1.0)
    }

    // ----------------------------------------------------------------
    //  Page request scheduling
    // ----------------------------------------------------------------
    fn request_visible_pages(&mut self) {
        let scale = self.render_scale();
        let n = self.layout.pages_across();

        // 1. Visible pages — highest priority
        for i in 0..n {
            let idx = self.current_page + i;
            if idx < self.page_count {
                self.request_page(idx, scale);
            }
        }

        // 2. Pre-render ±5 pages in interleaved order around the visible range
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

    fn request_page(&mut self, idx: usize, scale: f32) {
        if !self.cache.has_image_at_scale(idx, scale) && !self.pending.contains(&idx) {
            if self.render_tx.send(RenderRequest { idx, scale }).is_ok() {
                self.pending.insert(idx);
            }
        }
    }

    // ----------------------------------------------------------------
    //  State helpers
    // ----------------------------------------------------------------
    fn reset_pan(&mut self) {
        self.pan_x = 0.0;
        self.pan_y = 0.0;
    }

    fn update(&mut self, msg: Message) {
        match msg {
            Message::Quit => self.should_quit = true,

            Message::NextPage => {
                let step = self.layout.pages_across();
                let max = self.page_count.saturating_sub(1);
                self.current_page = (self.current_page + step).min(max);
                self.reset_pan();
            }
            Message::PrevPage => {
                let step = self.layout.pages_across();
                self.current_page = self.current_page.saturating_sub(step);
                self.reset_pan();
            }
            Message::FirstPage => {
                self.current_page = 0;
                self.reset_pan();
            }
            Message::LastPage => {
                self.current_page = self.page_count.saturating_sub(1);
                self.reset_pan();
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
                if self.zoom > 1.0 {
                    self.pan_y = (self.pan_y - PAN_STEP).max(-1.0);
                }
            }
            Message::ScrollDown => {
                if self.zoom > 1.0 {
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
                        self.reset_pan();
                    }
                }
                self.goto_mode = false;
                self.goto_input.clear();
            }
            Message::GotoCancel => {
                self.goto_mode = false;
                self.goto_input.clear();
            }
        }
    }
}
