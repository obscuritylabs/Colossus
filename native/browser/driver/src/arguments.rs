//! Owned POSIX argument storage, including the required argv[argc] sentinel.
use std::ffi::{CString, c_char};

use colossus_ports::BrowserDriverError;

pub(crate) struct NativeArguments {
    // The CString allocations must outlive every native call using argv.
    _bytes: Vec<CString>,
    pointers: Vec<*mut c_char>,
    count: i32,
}

impl NativeArguments {
    pub(crate) fn new<I, B>(arguments: I) -> Result<Self, BrowserDriverError>
    where
        I: IntoIterator<Item = B>,
        B: AsRef<[u8]>,
    {
        let bytes: Vec<_> = arguments
            .into_iter()
            .map(|argument| CString::new(argument.as_ref()).map_err(|_| BrowserDriverError::Denied))
            .collect::<Result<_, _>>()?;
        if bytes.is_empty() {
            return Err(BrowserDriverError::Denied);
        }
        let count = i32::try_from(bytes.len()).map_err(|_| BrowserDriverError::LimitExceeded)?;
        let pointers = bytes
            .iter()
            .map(|argument| argument.as_ptr().cast_mut())
            .chain(std::iter::once(std::ptr::null_mut()))
            .collect();
        Ok(Self {
            _bytes: bytes,
            pointers,
            count,
        })
    }

    pub(crate) fn as_ffi(&mut self) -> (i32, *mut *mut c_char) {
        (self.count, self.pointers.as_mut_ptr())
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::CStr;

    use super::*;

    #[test]
    fn helper_arguments_keep_exact_count_and_null_terminated_storage_after_move() {
        let original = NativeArguments::new([
            b"/opt/colossus-browser/colossus-native-browser-host".as_slice(),
            b"--type=zygote".as_slice(),
            b"--lang=C.UTF-8".as_slice(),
        ])
        .unwrap();
        let mut arguments = Box::new(original);
        let (count, pointers) = arguments.as_ffi();
        assert_eq!(count, 3);
        assert_eq!(arguments.pointers.len(), 4);
        for (index, expected) in arguments._bytes.iter().enumerate() {
            // SAFETY: this test retains all CString/vector allocations and reads
            // only an in-bounds argv element created from that same CString.
            let observed = unsafe { CStr::from_ptr(*pointers.add(index)) };
            assert_eq!(observed, expected.as_c_str());
        }
        // SAFETY: argv has exactly count+1 initialized slots; the final slot is
        // the POSIX sentinel and is checked while its vector remains alive.
        assert!(unsafe { *pointers.add(count as usize) }.is_null());
    }

    #[test]
    fn empty_argument_is_preserved_without_becoming_the_sentinel() {
        let mut arguments = NativeArguments::new([b"host".as_slice(), b"".as_slice()]).unwrap();
        assert_eq!(arguments.as_ffi().0, 2);
        assert!(!arguments.pointers[1].is_null());
        assert!(arguments.pointers[2].is_null());
        assert!(arguments._bytes[1].as_bytes().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn libc_spawn_observes_only_the_exact_owned_arguments() {
        let executable = CString::new("/bin/sh").unwrap();
        let mut arguments = NativeArguments::new([
            b"colossus-native-argv-fixture".as_slice(),
            b"-c".as_slice(),
            b"test \"$0\" = colossus-native-argv-fixture && test \"$#\" -eq 2 && test \"$1\" = 'with spaces' && test -z \"$2\"".as_slice(),
            b"colossus-native-argv-fixture".as_slice(),
            b"with spaces".as_slice(),
            b"".as_slice(),
        ])
        .unwrap();
        let (count, pointers) = arguments.as_ffi();
        assert_eq!(count, 6);
        let environment = [std::ptr::null_mut()];
        let mut process = 0;
        // SAFETY: the owned argv, its null sentinel and the empty terminated
        // environment remain alive through synchronous posix_spawn. Null
        // actions/attributes select defaults; pid storage has the correct type.
        let status = unsafe {
            libc::posix_spawn(
                &mut process,
                executable.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                pointers,
                environment.as_ptr(),
            )
        };
        assert_eq!(status, 0);
        let mut result = 0;
        loop {
            // SAFETY: posix_spawn succeeded and this test exclusively owns the
            // exact live child; waitpid writes only to valid exit-status storage.
            let waited = unsafe { libc::waitpid(process, &mut result, 0) };
            if waited == process {
                break;
            }
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EINTR)
            );
        }
        assert!(libc::WIFEXITED(result));
        assert_eq!(libc::WEXITSTATUS(result), 0);
    }

    #[test]
    fn rejects_absent_program_and_embedded_nul_before_native_entry() {
        assert!(matches!(
            NativeArguments::new(std::iter::empty::<&[u8]>()),
            Err(BrowserDriverError::Denied)
        ));
        assert!(matches!(
            NativeArguments::new([b"host\0unexpected".as_slice()]),
            Err(BrowserDriverError::Denied)
        ));
    }
}
