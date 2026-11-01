//! Bounded validation and late resolution for encrypted inputs and released tool images.

mod document;
mod import;
mod preview;
mod resolver;
mod validation;

pub use document::{
    MAX_COMBINED_DOCUMENT_BYTES, MAX_DOCUMENT_BYTES, MAX_DOCUMENT_COUNT, ValidatedDocument,
    document_media_type_for_file_name, supported_document_media_type, validate_document_bytes,
    validate_document_references,
};
pub use import::import_released_images;
pub use preview::image_preview;

pub use resolver::JournalRunInputMediaResolver;
pub use validation::{
    MAX_COMBINED_IMAGE_BYTES, MAX_IMAGE_BYTES, MAX_IMAGE_COUNT, MAX_IMAGE_PIXELS,
    MAX_IMAGE_SIDE_PIXELS, ValidatedImage, validate_image_bytes, validate_image_references,
};

#[cfg(test)]
mod tests;
