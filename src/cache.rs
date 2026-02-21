use std::collections::HashMap;

use image::DynamicImage;
use ratatui::layout::Rect;
use ratatui_image::{picker::Picker, protocol::Protocol, FilterType, Resize};

pub struct PageCache {
    images: HashMap<usize, DynamicImage>,
    image_scales: HashMap<usize, f32>,
    inverted: HashMap<usize, DynamicImage>,
    protocols: HashMap<(usize, bool), Protocol>,
    current_zoom: f32,
    current_pan: (f32, f32),
}

impl PageCache {
    pub fn new() -> Self {
        Self {
            images: HashMap::new(),
            image_scales: HashMap::new(),
            inverted: HashMap::new(),
            protocols: HashMap::new(),
            current_zoom: 1.0,
            current_pan: (0.0, 0.0),
        }
    }

    pub fn clear(&mut self) {
        self.images.clear();
        self.image_scales.clear();
        self.inverted.clear();
        self.protocols.clear();
    }

    pub fn invalidate_protocols(&mut self) {
        self.protocols.clear();
    }

    pub fn has_image_at_scale(&self, page_idx: usize, scale: f32) -> bool {
        self.image_scales
            .get(&page_idx)
            .is_some_and(|s| (s - scale).abs() < 0.01)
    }

    pub fn insert_image(&mut self, page_idx: usize, scale: f32, img: DynamicImage) {
        self.protocols.remove(&(page_idx, false));
        self.protocols.remove(&(page_idx, true));
        self.inverted.remove(&page_idx);
        self.images.insert(page_idx, img);
        self.image_scales.insert(page_idx, scale);
    }

    pub fn image_dims(&self, page_idx: usize) -> Option<(u32, u32)> {
        self.images
            .get(&page_idx)
            .map(|img| (img.width(), img.height()))
    }

    pub fn get_protocol(
        &mut self,
        page_idx: usize,
        dark_mode: bool,
        zoom: f32,
        pan: (f32, f32),
        picker: &Picker,
        area: Rect,
    ) -> Option<&Protocol> {
        let zoom_changed = (self.current_zoom - zoom).abs() > f32::EPSILON;
        let pan_changed = (self.current_pan.0 - pan.0).abs() > f32::EPSILON
            || (self.current_pan.1 - pan.1).abs() > f32::EPSILON;

        if zoom_changed || pan_changed {
            self.protocols.clear();
            self.current_zoom = zoom;
            self.current_pan = pan;
        }

        let key = (page_idx, dark_mode);
        if !self.protocols.contains_key(&key) {
            let base_img = if dark_mode {
                if !self.inverted.contains_key(&page_idx) {
                    let normal = self.images.get(&page_idx)?;
                    let mut inv = normal.clone();
                    inv.invert();
                    self.inverted.insert(page_idx, inv);
                }
                self.inverted.get(&page_idx)?
            } else {
                self.images.get(&page_idx)?
            };

            let img = if zoom > 1.0 {
                crop_with_pan(base_img, zoom, pan.0, pan.1)
            } else {
                base_img.clone()
            };

            let protocol = picker
                .new_protocol(img, area, Resize::Fit(Some(FilterType::CatmullRom)))
                .ok()?;
            self.protocols.insert(key, protocol);
        }
        self.protocols.get(&key)
    }
}

/// Crop a viewport-sized portion of the image for zoom, offset by pan.
/// `pan_x`/`pan_y` range: `-1.0` (top/left) to `1.0` (bottom/right), `0.0` = center.
fn crop_with_pan(img: &DynamicImage, zoom: f32, pan_x: f32, pan_y: f32) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    let crop_w = (w as f32 / zoom).round().max(1.0) as u32;
    let crop_h = (h as f32 / zoom).round().max(1.0) as u32;

    let max_x = w.saturating_sub(crop_w);
    let max_y = h.saturating_sub(crop_h);

    let x = (pan_x.mul_add(0.5, 0.5) * max_x as f32).round() as u32;
    let y = (pan_y.mul_add(0.5, 0.5) * max_y as f32).round() as u32;

    img.crop_imm(x.min(max_x), y.min(max_y), crop_w.max(1), crop_h.max(1))
}
