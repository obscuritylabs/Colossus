use std::sync::Arc;
#[cfg(feature = "dictation")]
mod native;
#[cfg(feature = "dictation")]
mod settings;

pub(crate) fn local_port() -> Option<Arc<dyn colossus_tui::LocalDictation>> {
    #[cfg(feature = "dictation")]
    {
        Some(Arc::new(native::NativeDictation::default()))
    }
    #[cfg(not(feature = "dictation"))]
    {
        None
    }
}
