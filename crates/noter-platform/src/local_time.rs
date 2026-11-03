//! Locale short time for the quiet save receipt.
//!
//! The application crate does not format clocks. Windows asks the user locale
//! for the time without seconds. Unix reads `T_FMT` from the environment's
//! time locale and removes the seconds field. Neither path calls `setlocale`,
//! so the process locale stays as it was. `None` means the caller must not
//! invent a time.

/// Upper bound passed to the locale formatter. A short time fits well inside it.
const TIME_BUFFER_BYTES: usize = 128;

/// Returns the user locale's short local time, without seconds.
///
/// `None` means the operating system could not format a time, the text
/// contained a terminal control, or nothing remained after trimming. A
/// control is rejected before trimming, so a trailing newline does not
/// become a clock reading. Callers show the ordinary save status instead
/// of guessing a time.
#[must_use]
pub fn local_short_time() -> Option<String> {
    #[cfg(windows)]
    {
        // `windows_` is the Linux mutation exclude. This body is compiled
        // only on Windows, and the Windows exclude does not match that prefix.
        windows_local_short_time()
    }
    #[cfg(unix)]
    {
        // `unix_` is the Windows mutation exclude. cargo-mutants does not
        // evaluate cfg, so this body would otherwise be reported missed.
        unix_local_short_time()
    }
    #[cfg(not(any(unix, windows)))]
    {
        // `linux` and `windows` are the mutation excludes on the other host.
        // This body is compiled on neither CI platform.
        linux_windows_local_short_time()
    }
}

/// `written` includes the trailing NUL from `GetTimeFormatEx`.
///
/// A count of one is only that NUL. A count past the buffer is not a clock.
/// The live API does not return those values, so the bounds have their own tests.
#[cfg(any(windows, test))]
fn windows_clock_units(buffer: &[u16], written: i32) -> Option<&[u16]> {
    let count = usize::try_from(written).ok()?;
    if count <= 1 || count > buffer.len() {
        return None;
    }
    Some(&buffer[..count - 1])
}

#[cfg(windows)]
fn windows_local_short_time() -> Option<String> {
    use windows_sys::Win32::Globalization::{GetTimeFormatEx, TIME_NOSECONDS};

    let mut buffer = [0_u16; TIME_BUFFER_BYTES];
    let capacity = i32::try_from(buffer.len()).expect("the time buffer fits in a Win32 length");
    // SAFETY: `buffer` is writable for the length passed in. A null locale
    // selects the user locale, a null `SYSTEMTIME` selects the current local
    // time, and a null format selects that locale's default time pattern.
    // `TIME_NOSECONDS` is the documented flag for the short time.
    #[allow(unsafe_code)]
    let written = unsafe {
        GetTimeFormatEx(
            std::ptr::null(),
            TIME_NOSECONDS,
            std::ptr::null(),
            std::ptr::null(),
            buffer.as_mut_ptr(),
            capacity,
        )
    };
    let units = windows_clock_units(&buffer, written)?;
    let text = String::from_utf16(units).ok()?;
    clock_text_for_receipt(&text)
}

#[cfg(unix)]
fn unix_local_short_time() -> Option<String> {
    let locale = UnixTimeLocale::from_environment()?;
    let format = locale.short_format()?;
    let broken = unix_local_tm()?;
    locale.format(&broken, &format)
}

#[cfg(not(any(unix, windows)))]
fn linux_windows_local_short_time() -> Option<String> {
    None
}

/// How many leading bytes `accept` takes.
///
/// The slice is finite, so the count ends. Callers advance by this count
/// once. An increment inside the scan can be mutated into a step that never
/// reaches the end of the format.
#[cfg(any(unix, test))]
fn leading_accepted_len(bytes: &[u8], accept: impl Fn(u8) -> bool) -> usize {
    bytes
        .iter()
        .copied()
        .take_while(|byte| accept(*byte))
        .count()
}

#[cfg(any(unix, test))]
const fn is_strftime_flag(byte: u8) -> bool {
    matches!(byte, b'-' | b'_' | b'0' | b'^' | b'#' | b'+')
}

/// Removes seconds from a POSIX `T_FMT` string.
///
/// `%T` becomes `%H:%M` and `%r` becomes `%I:%M %p`. A seconds conversion,
/// including flags, a width, and an `E` or `O` modifier, is dropped with one
/// preceding separator. `%X` and `%c` include an opaque time, so they produce
/// no format rather than a guess. The result is trimmed and empty input
/// produces `None`.
///
/// Unix calls this before `strftime_l`. Windows uses `GetTimeFormatEx` and
/// does not. The function still compiles into every test build so the scanner
/// is checked on Windows.
#[cfg(any(unix, test))]
fn locale_short_time_format(format: &str) -> Option<String> {
    let bytes = format.as_bytes();
    let mut out = String::with_capacity(format.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            let character = format[index..].chars().next()?;
            out.push(character);
            index = index.checked_add(character.len_utf8())?;
            continue;
        }
        let start = index;
        index = index.checked_add(1)?;
        if index >= bytes.len() {
            return None;
        }
        if bytes[index] == b'%' {
            out.push('%');
            out.push('%');
            index = index.checked_add(1)?;
            continue;
        }
        index = index.checked_add(leading_accepted_len(&bytes[index..], is_strftime_flag))?;
        index = index.checked_add(leading_accepted_len(&bytes[index..], |byte| {
            byte.is_ascii_digit()
        }))?;
        if index < bytes.len() && matches!(bytes[index], b'E' | b'O') {
            index = index.checked_add(1)?;
        }
        if index >= bytes.len() || !bytes[index].is_ascii_alphabetic() {
            return None;
        }
        let conversion = bytes[index];
        index = index.checked_add(1)?;
        match conversion {
            b'S' => {
                if out.ends_with([':', '.', ',', '-']) {
                    out.pop();
                }
            }
            b'T' => out.push_str("%H:%M"),
            b'r' => out.push_str("%I:%M %p"),
            b'X' | b'c' => return None,
            _ => out.push_str(&format[start..index]),
        }
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(unix)]
struct UnixTimeLocale(libc::locale_t);

#[cfg(unix)]
impl UnixTimeLocale {
    fn from_environment() -> Option<Self> {
        // SAFETY: `LC_TIME_MASK` limits the locale object to time formatting.
        // An empty locale name reads the environment for that category only.
        // A null base creates a new object. Failure returns a null pointer and
        // does not change the process locale.
        #[allow(unsafe_code)]
        let locale =
            unsafe { libc::newlocale(libc::LC_TIME_MASK, c"".as_ptr(), std::ptr::null_mut()) };
        if locale.is_null() {
            None
        } else {
            Some(Self(locale))
        }
    }

    fn short_format(&self) -> Option<String> {
        // SAFETY: `self.0` came from `newlocale` and this guard has not freed
        // it. The returned pointer is valid until the next `nl_langinfo_l` on
        // this object or until `freelocale`. The copy below finishes first.
        #[allow(unsafe_code)]
        let pointer = unsafe { nl_langinfo_l(libc::T_FMT, self.0) };
        if pointer.is_null() {
            return None;
        }
        // SAFETY: the pointer was non-null and points at a C string owned by
        // the locale object. `CStr::from_ptr` only reads until the terminator.
        #[allow(unsafe_code)]
        let format = unsafe { std::ffi::CStr::from_ptr(pointer) };
        locale_short_time_format(format.to_str().ok()?)
    }

    fn format(&self, broken: &libc::tm, format: &str) -> Option<String> {
        let format = std::ffi::CString::new(format).ok()?;
        let mut buffer = [0_u8; TIME_BUFFER_BYTES];
        // SAFETY: `buffer` is writable for its full length. `broken` is a
        // complete `tm` from `localtime_r`. `format` is a terminated C string
        // with no interior null. `self.0` is the live locale object.
        #[allow(unsafe_code)]
        let written = unsafe {
            libc::strftime_l(
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                format.as_ptr(),
                broken,
                self.0,
            )
        };
        if written == 0 || written >= buffer.len() {
            return None;
        }
        let text = std::str::from_utf8(&buffer[..written]).ok()?;
        clock_text_for_receipt(text)
    }
}

#[cfg(unix)]
impl Drop for UnixTimeLocale {
    fn drop(&mut self) {
        if self.0.is_null() {
            return;
        }
        // SAFETY: the pointer came from `newlocale` and this is the only free.
        #[allow(unsafe_code)]
        unsafe {
            libc::freelocale(self.0);
        }
    }
}

#[cfg(unix)]
fn unix_local_tm() -> Option<libc::tm> {
    // SAFETY: a null pointer asks for the current calendar time. `-1` is the
    // documented failure value and leaves no output to read.
    #[allow(unsafe_code)]
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    if now == -1 {
        return None;
    }
    // Zero first. `tm` has padding `localtime_r` does not document as written,
    // and `assume_init` would otherwise read those bytes.
    let mut broken = std::mem::MaybeUninit::<libc::tm>::zeroed();
    // SAFETY: `now` is a live `time_t`. `broken` is writable and already
    // initialized to zero. On success the function writes every field.
    #[allow(unsafe_code)]
    let filled = unsafe { libc::localtime_r(&raw const now, broken.as_mut_ptr()) };
    if filled.is_null() {
        return None;
    }
    // SAFETY: `localtime_r` returned the same pointer it was given, so every
    // field is written. Padding left untouched is still the zero written above.
    #[allow(unsafe_code)]
    Some(unsafe { broken.assume_init() })
}

/// Keeps a clock reading only when every character is safe to draw.
///
/// The control check happens before `trim`. Newline, tab, the C1 next-line
/// character, and the line and paragraph separators are whitespace, so
/// trimming them first would present them as an ordinary time.
fn clock_text_for_receipt(raw: &str) -> Option<String> {
    if raw.chars().any(crate::is_terminal_unsafe) {
        return None;
    }
    let text = raw.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

// libc 0.2.189 binds this on Linux and FreeBSD, but not on macOS.
// libSystem, glibc, and the BSDs all provide the symbol. Rustdoc does not
// attach a doc comment to an extern block.
#[cfg(unix)]
#[allow(unsafe_code)]
unsafe extern "C" {
    fn nl_langinfo_l(item: libc::nl_item, locale: libc::locale_t) -> *mut libc::c_char;
}

#[cfg(test)]
mod tests {
    use super::locale_short_time_format;

    #[test]
    fn locale_short_time_formats_drop_seconds() {
        assert_eq!(
            locale_short_time_format("%H:%M:%S").as_deref(),
            Some("%H:%M")
        );
        assert_eq!(
            locale_short_time_format("%I:%M:%S %p").as_deref(),
            Some("%I:%M %p")
        );
        assert_eq!(locale_short_time_format("%T").as_deref(), Some("%H:%M"));
        assert_eq!(locale_short_time_format("%r").as_deref(), Some("%I:%M %p"));
        assert_eq!(locale_short_time_format("%H:%M").as_deref(), Some("%H:%M"));
        assert_eq!(
            locale_short_time_format("%OH:%OM:%OS").as_deref(),
            Some("%OH:%OM")
        );
        assert_eq!(
            locale_short_time_format("%H.%M.%S").as_deref(),
            Some("%H.%M")
        );
        assert_eq!(
            locale_short_time_format("%H:%M:%S %Z").as_deref(),
            Some("%H:%M %Z")
        );
        assert_eq!(
            locale_short_time_format("%H:%M:%-S").as_deref(),
            Some("%H:%M")
        );
        assert_eq!(
            locale_short_time_format("%H:%M:%9S").as_deref(),
            Some("%H:%M")
        );
        assert_eq!(
            locale_short_time_format("%H %M %S").as_deref(),
            Some("%H %M")
        );
        assert_eq!(locale_short_time_format("%%S").as_deref(), Some("%%S"));
        assert_eq!(locale_short_time_format("%EY").as_deref(), Some("%EY"));
        for preserved in ["%-H", "%_H", "%0H", "%^H", "%#H", "%+H", "%12H", "%EH"] {
            assert_eq!(
                locale_short_time_format(preserved).as_deref(),
                Some(preserved),
                "{preserved}"
            );
        }
        assert_eq!(locale_short_time_format("%X"), None);
        assert_eq!(locale_short_time_format("%c"), None);
        assert_eq!(locale_short_time_format("%EX"), None);
        assert_eq!(locale_short_time_format(""), None);
        assert_eq!(locale_short_time_format("%"), None);
        assert_eq!(locale_short_time_format("%-"), None);
        assert_eq!(locale_short_time_format("%3"), None);
        assert_eq!(locale_short_time_format("%E"), None);
        assert_eq!(locale_short_time_format("%S"), None);
        assert_eq!(
            locale_short_time_format("午後 %H:%M:%S").as_deref(),
            Some("午後 %H:%M")
        );
        // `%n` and `%t` stay in the format. The clock gate below refuses the
        // newline or tab that `strftime` would write. Trimming first would
        // turn either into an ordinary time.
        assert_eq!(
            locale_short_time_format("%H:%M:%S%n").as_deref(),
            Some("%H:%M%n")
        );
        assert_eq!(
            locale_short_time_format("%H:%M%t").as_deref(),
            Some("%H:%M%t")
        );
    }

    #[test]
    fn windows_clock_units_reject_a_count_the_buffer_does_not_hold() {
        use super::windows_clock_units;

        let buffer = [u16::from(b'1'), u16::from(b'4'), 0];
        assert_eq!(windows_clock_units(&buffer, 0), None);
        assert_eq!(windows_clock_units(&buffer, 1), None);
        assert_eq!(windows_clock_units(&buffer, -1), None);
        assert_eq!(windows_clock_units(&buffer, 4), None);
        assert_eq!(
            windows_clock_units(&buffer, 3),
            Some([u16::from(b'1'), u16::from(b'4')].as_slice())
        );
        assert_eq!(
            windows_clock_units(&buffer, 2),
            Some([u16::from(b'1')].as_slice())
        );
    }

    #[test]
    fn a_terminal_control_never_becomes_a_clock_reading() {
        use super::clock_text_for_receipt;

        assert_eq!(
            clock_text_for_receipt("  午後2:02  ").as_deref(),
            Some("午後2:02")
        );
        for time in [
            "",
            " ",
            "14:02\n",
            " \n14:02\n ",
            "14:02\t",
            "14:02\u{7f}",
            "14:02\u{85}",
            "14:02\u{1b}",
            "14:02\u{2028}",
            "14:02\u{2029}",
            "14:02\u{202e}",
        ] {
            assert_eq!(clock_text_for_receipt(time), None, "{time:?}");
        }
    }

    #[test]
    fn the_local_short_time_is_a_short_readable_clock() {
        let time = super::local_short_time().expect("the locale short time");
        assert!(
            time.chars().any(|character| character.is_ascii_digit()),
            "{time}"
        );
        assert!(time.chars().count() <= 64, "{time}");
        assert!(
            time.chars()
                .all(|character| !crate::is_terminal_unsafe(character)),
            "{time}"
        );
        assert!(!time.contains('\n'), "{time}");
    }
}
