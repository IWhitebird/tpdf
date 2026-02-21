use image::DynamicImage;

pub fn invert(img: &DynamicImage) -> DynamicImage {
    let mut inverted = img.clone();
    inverted.invert();
    inverted
}
