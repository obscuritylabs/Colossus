use crate::validation::{InputError, validate_units};
use objc2::{
    DefinedClass, MainThreadOnly, Message, define_class, msg_send, rc::Retained, runtime::AnyObject,
};
use objc2_app_kit::NSTextField;
use objc2_foundation::{MainThreadMarker, NSFormatter, NSObjectProtocol, NSString};

pub(super) struct FormatterIvars {
    status: Retained<NSTextField>,
}

define_class!(
    // SAFETY: NSFormatter permits subclassing. These implementations preserve the
    // original string, reject invalid edits, and never parse or transform tokens.
    #[unsafe(super = NSFormatter)]
    #[name = "ColossusCredentialFormatterV1"]
    #[thread_kind = MainThreadOnly]
    #[ivars = FormatterIvars]
    pub(super) struct TokenFormatter;

    unsafe impl NSObjectProtocol for TokenFormatter {}

    impl TokenFormatter {
        #[unsafe(method_id(stringForObjectValue:))]
        fn formatted(&self, object: Option<&AnyObject>) -> Option<Retained<NSString>> {
            object.and_then(|object| object.downcast_ref::<NSString>()).map(Message::retain)
        }

        #[unsafe(method(getObjectValue:forString:errorDescription:))]
        fn parsed(&self, output: *mut *mut AnyObject, string: &NSString, _: *mut *mut NSString) -> bool {
            if validate_native(string).is_err() { false } else {
                if !output.is_null() {
                    // SAFETY: Cocoa supplies a writable autoreleasing object out
                    // parameter. Preserve the NSString with normal +0 semantics.
                    unsafe { *output = Retained::autorelease_ptr(string.retain()).cast(); }
                }
                true
            }
        }

        #[unsafe(method(isPartialStringValid:newEditingString:errorDescription:))]
        fn partial(&self, value: &NSString, _: *mut *mut NSString, _: *mut *mut NSString) -> bool {
            match validate_native(value) {
                Ok(()) => true,
                Err(error) => {
                    self.ivars().status.setStringValue(&NSString::from_str(error.message()));
                    false
                }
            }
        }
    }
);

impl TokenFormatter {
    pub(super) fn new(mtm: MainThreadMarker, status: Retained<NSTextField>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FormatterIvars { status });
        // SAFETY: NSFormatter's designated NSObject initializer is valid here.
        unsafe { msg_send![super(this), init] }
    }
}

/// Validate the Cocoa-owned candidate without allocating an unbounded Rust copy.
pub(super) fn validate_native(value: &NSString) -> Result<(), InputError> {
    validate_units(
        (0..value.length()).map(|index| value.characterAtIndex(index)),
        value.length(),
    )
}
