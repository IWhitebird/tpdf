use image::{DynamicImage, ImageBuffer, RgbImage};
use mupdf::{Colorspace, Document, Matrix};

pub struct PdfDocument {
    doc: Document,
}

impl PdfDocument {
    pub fn open(path: &str) -> Result<Self, mupdf::Error> {
        let doc = Document::open(path)?;
        Ok(Self { doc })
    }

    pub fn page_count(&self) -> usize {
        self.doc.page_count().unwrap_or(0) as usize
    }

    pub fn page_bounds(&self, page_idx: usize) -> Result<(f32, f32), mupdf::Error> {
        let page = self.doc.load_page(page_idx as i32)?;
        let bounds = page.bounds()?;
        Ok((bounds.x1 - bounds.x0, bounds.y1 - bounds.y0))
    }

    pub fn render_page(&self, page_idx: usize, scale: f32) -> Result<DynamicImage, mupdf::Error> {
        let page = self.doc.load_page(page_idx as i32)?;
        let matrix = Matrix::new_scale(scale, scale);
        let pixmap = page.to_pixmap(&matrix, &Colorspace::device_rgb(), false, true)?;

        let width = pixmap.width() as u32;
        let height = pixmap.height() as u32;
        let samples = pixmap.samples();
        let stride = pixmap.stride() as usize;
        let n = pixmap.n() as usize;
        let expected_stride = width as usize * n;

        let rgb_data = if stride == expected_stride {
            samples.to_vec()
        } else {
            let mut data = Vec::with_capacity(height as usize * expected_stride);
            for row in 0..height as usize {
                let start = row * stride;
                let end = start + expected_stride;
                data.extend_from_slice(&samples[start..end]);
            }
            data
        };

        let img: RgbImage = ImageBuffer::from_raw(width, height, rgb_data)
            .expect("pixmap dimensions should match buffer size");

        Ok(DynamicImage::ImageRgb8(img))
    }

}
