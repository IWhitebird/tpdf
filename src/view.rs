use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};
use ratatui_image::Image as RatatuiImage;

use crate::app::{App, PageLayout};

#[derive(Clone, Copy)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let (content_area, status_area) = if app.fullscreen {
        (frame.area(), None)
    } else {
        let [ca, sa] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
        (ca, Some(sa))
    };

    let bg = if app.dark_mode {
        Color::Rgb(0, 0, 0)
    } else {
        Color::Rgb(255, 255, 255)
    };
    frame.render_widget(Block::default().style(Style::default().bg(bg)), content_area);

    match app.layout {
        PageLayout::Single => {
            render_page(frame, content_area, app, app.current_page, HAlign::Center);
        }
        PageLayout::Dual => draw_multi_page(frame, content_area, app, 2),
        PageLayout::Triple => draw_multi_page(frame, content_area, app, 3),
    }

    if let Some(sa) = status_area {
        draw_status_bar(frame, sa, app);
    }
}

fn draw_multi_page(frame: &mut Frame, area: Rect, app: &mut App, count: usize) {
    let constraints: Vec<Constraint> = (0..count).map(|_| Constraint::Fill(1)).collect();
    let areas = Layout::horizontal(constraints).spacing(0).split(area);

    for i in 0..count {
        let idx = app.current_page + i;
        if idx < app.page_count {
            let align = if i == 0 {
                HAlign::Right
            } else if i == count - 1 {
                HAlign::Left
            } else {
                HAlign::Center
            };
            render_page(frame, areas[i], app, idx, align);
        }
    }
}

fn render_page(frame: &mut Frame, area: Rect, app: &mut App, page_idx: usize, halign: HAlign) {
    if page_idx >= app.page_count {
        return;
    }

    let render_area = if let Some((w, h)) = app.cache.image_dims(page_idx) {
        aligned_image_area(w, h, area, app.picker.font_size(), app.zoom, halign)
    } else {
        area
    };

    if let Some(protocol) = app.cache.get_protocol(
        page_idx,
        app.dark_mode,
        app.zoom,
        (app.pan_x, app.pan_y),
        &app.picker,
        render_area,
    ) {
        let widget = RatatuiImage::new(protocol);
        frame.render_widget(widget, render_area);
    } else {
        let text = format!("Loading page {}...", page_idx + 1);
        let loading = Paragraph::new(text).alignment(Alignment::Center);
        let y = area.y + area.height / 2;
        frame.render_widget(loading, Rect::new(area.x, y, area.width, 1));
    }
}

/// Calculate a sub-rect for the image with the given horizontal alignment.
///
/// Uses the Picker's `font_size` and `ceil()` to match ratatui-image's internal
/// `round_pixel_size_to_cells`, so our area exactly matches the protocol footprint.
pub fn aligned_image_area(
    img_w: u32,
    img_h: u32,
    area: Rect,
    font_size: (u16, u16),
    zoom: f32,
    halign: HAlign,
) -> Rect {
    if area.width == 0 || area.height == 0 || img_w == 0 || img_h == 0 {
        return area;
    }

    let (fw, fh) = (f64::from(font_size.0), f64::from(font_size.1));

    let area_px_w = f64::from(area.width) * fw;
    let area_px_h = f64::from(area.height) * fh;

    let fit_scale = (area_px_w / f64::from(img_w)).min(area_px_h / f64::from(img_h));
    let display_scale = fit_scale * f64::from(zoom).min(1.0);

    let used_w = ((f64::from(img_w) * display_scale) / fw).ceil() as u16;
    let used_h = ((f64::from(img_h) * display_scale) / fh).ceil() as u16;

    let final_w = used_w.min(area.width).max(1);
    let final_h = used_h.min(area.height).max(1);

    let x_off = match halign {
        HAlign::Left => 0,
        HAlign::Center => (area.width.saturating_sub(final_w)) / 2,
        HAlign::Right => area.width.saturating_sub(final_w),
    };
    let y_off = (area.height.saturating_sub(final_h)) / 2;

    Rect::new(area.x + x_off, area.y + y_off, final_w, final_h)
}

fn draw_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    let bold = Style::default().add_modifier(Modifier::BOLD);

    if app.goto_mode {
        let left_parts = vec![
            Span::styled(" tpdf", bold),
            Span::raw(format!(" | goto: {}", app.goto_input)),
        ];
        let right = "Enter:go  Esc:cancel ";
        let left_len = 5 + 10 + app.goto_input.len();
        let gap = (area.width as usize).saturating_sub(left_len + right.len());

        let mut spans = left_parts;
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::raw(right));
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }

    let start = app.current_page + 1;
    let n = app.layout.pages_across();
    let end = (app.current_page + n).min(app.page_count);
    let pages = if end > start {
        format!("{start}-{end}/{}", app.page_count)
    } else {
        format!("{start}/{}", app.page_count)
    };

    let zoom_pct = format!("{}%", (app.zoom * 100.0).round() as u32);

    let mut info_parts = vec![pages, zoom_pct];
    match app.layout {
        PageLayout::Dual => info_parts.push("2UP".into()),
        PageLayout::Triple => info_parts.push("3UP".into()),
        PageLayout::Single => {}
    }
    if app.dark_mode {
        info_parts.push("NIGHT".into());
    }

    let info = info_parts.join(" | ");
    let keys = "h/l:page  jk:pan  +/-:zoom  d:layout  f:full  p:goto  n:night  q:quit ";

    let left_parts = vec![
        Span::styled(" tpdf", bold),
        Span::raw(format!(" | {info}")),
    ];
    let left_len = 5 + 3 + info.len();
    let gap = (area.width as usize).saturating_sub(left_len + keys.len());

    let mut spans = left_parts;
    spans.push(Span::raw(" ".repeat(gap)));
    spans.push(Span::raw(keys));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
