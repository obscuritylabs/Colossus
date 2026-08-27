//! Bounded validation and late resolution for encrypted run-input images.

mod resolver;
mod validation;

pub use resolver::JournalRunInputMediaResolver;
pub use validation::{
    MAX_COMBINED_IMAGE_BYTES, MAX_IMAGE_BYTES, MAX_IMAGE_COUNT, MAX_IMAGE_PIXELS,
    MAX_IMAGE_SIDE_PIXELS, ValidatedImage, validate_image_bytes, validate_image_references,
};

#[cfg(test)]
mod tests;
