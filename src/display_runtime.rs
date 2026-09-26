//! Window runtime libraries on Linux and the BSDs.
//!
//! The windowing stack loads its keyboard, display-protocol, and OpenGL
//! libraries at run time instead of linking them. When one is missing it
//! panics deep inside that stack, and release builds abort, so the user sees
//! a crash rather than a cause. Checking the same libraries first turns that
//! into one line that names the package to install.

/// The display protocol the windowing stack will use.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DisplayBackend {
    X11,
    Wayland,
}

impl DisplayBackend {
    /// Chooses as winit 0.30 does: Wayland whenever `WAYLAND_DISPLAY` or
    /// `WAYLAND_SOCKET` names one, with no fallback to X11; otherwise X11.
    pub const fn from_environment(wayland_advertised: bool) -> Self {
        if wayland_advertised {
            Self::Wayland
        } else {
            Self::X11
        }
    }
}

/// One library the window needs, with the names it can load under and the
/// packages that provide it on common distributions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Requirement {
    pub sonames: &'static [&'static str],
    pub debian: &'static str,
    pub fedora: &'static str,
    pub arch: &'static str,
}

const XKBCOMMON: Requirement = Requirement {
    sonames: &["libxkbcommon.so.0", "libxkbcommon.so"],
    debian: "libxkbcommon0",
    fedora: "libxkbcommon",
    arch: "libxkbcommon",
};

const XKBCOMMON_X11: Requirement = Requirement {
    sonames: &["libxkbcommon-x11.so.0", "libxkbcommon-x11.so"],
    debian: "libxkbcommon-x11-0",
    fedora: "libxkbcommon-x11",
    arch: "libxkbcommon-x11",
};

const WAYLAND_CLIENT: Requirement = Requirement {
    sonames: &["libwayland-client.so.0", "libwayland-client.so"],
    debian: "libwayland-client0",
    fedora: "libwayland-client",
    arch: "wayland",
};

const WAYLAND_EGL: Requirement = Requirement {
    sonames: &["libwayland-egl.so.1", "libwayland-egl.so"],
    debian: "libwayland-egl1",
    fedora: "libwayland-egl",
    arch: "wayland",
};

/// OpenGL on Wayland, which only reaches it through EGL.
const EGL: Requirement = Requirement {
    sonames: &["libEGL.so.1", "libEGL.so"],
    debian: "libegl1",
    fedora: "mesa-libEGL",
    arch: "libglvnd",
};

/// OpenGL on X11 through EGL or GLX; either one lets the renderer start.
const OPENGL: Requirement = Requirement {
    sonames: &["libEGL.so.1", "libEGL.so", "libGL.so.1", "libGL.so"],
    debian: "libegl1",
    fedora: "mesa-libEGL",
    arch: "libglvnd",
};

/// Returns the libraries `backend` needs.
pub const fn requirements(backend: DisplayBackend) -> &'static [Requirement] {
    match backend {
        DisplayBackend::X11 => &[XKBCOMMON, XKBCOMMON_X11, OPENGL],
        DisplayBackend::Wayland => &[XKBCOMMON, WAYLAND_CLIENT, WAYLAND_EGL, EGL],
    }
}

/// Returns the requirements of `backend` that `loads` cannot satisfy.
pub fn missing(backend: DisplayBackend, loads: impl Fn(&[&str]) -> bool) -> Vec<Requirement> {
    requirements(backend)
        .iter()
        .copied()
        .filter(|requirement| !loads(requirement.sonames))
        .collect()
}

/// Explains which libraries are missing and how to get them.
pub fn describe(missing: &[Requirement]) -> String {
    let names = |package: fn(&Requirement) -> &'static str| {
        missing.iter().map(package).collect::<Vec<_>>().join(" ")
    };
    let libraries = missing
        .iter()
        .map(|requirement| requirement.sonames[0])
        .collect::<Vec<_>>()
        .join(", ");
    let pronoun = if missing.len() == 1 { "it" } else { "them" };
    format!(
        "the window needs {libraries}, which could not be loaded.\n\
         Install {}, for example:\n  \
         Debian or Ubuntu: sudo apt install {}\n  \
         Fedora: sudo dnf install {}\n  \
         Arch: sudo pacman -S {}\n\
         Or use the terminal interface: noter --tui",
        pronoun,
        names(|requirement| requirement.debian),
        names(|requirement| requirement.fedora),
        names(|requirement| requirement.arch),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_is_chosen_whenever_it_is_advertised() {
        assert_eq!(
            DisplayBackend::from_environment(true),
            DisplayBackend::Wayland
        );
        assert_eq!(DisplayBackend::from_environment(false), DisplayBackend::X11);
    }

    #[test]
    fn each_backend_needs_its_keyboard_protocol_and_opengl_libraries() {
        assert_eq!(
            requirements(DisplayBackend::X11),
            &[XKBCOMMON, XKBCOMMON_X11, OPENGL]
        );
        assert_eq!(
            requirements(DisplayBackend::Wayland),
            &[XKBCOMMON, WAYLAND_CLIENT, WAYLAND_EGL, EGL]
        );
        // GLX cannot stand in for EGL on Wayland.
        assert!(!EGL.sonames.contains(&"libGL.so.1"));
    }

    #[test]
    fn only_unloadable_requirements_are_reported() {
        let everything = missing(DisplayBackend::X11, |_| true);
        assert!(everything.is_empty());

        let without_x11_keyboard = missing(DisplayBackend::X11, |sonames| {
            !sonames.contains(&"libxkbcommon-x11.so.0")
        });
        assert_eq!(without_x11_keyboard, vec![XKBCOMMON_X11]);

        // Either OpenGL entry point satisfies the renderer.
        let glx_only = missing(DisplayBackend::X11, |sonames| {
            sonames != OPENGL.sonames || sonames.contains(&"libGL.so.1")
        });
        assert!(glx_only.is_empty());
    }

    #[test]
    fn the_description_names_the_library_and_each_package_manager() {
        let text = describe(&[XKBCOMMON_X11, OPENGL]);
        assert!(text.starts_with(
            "the window needs libxkbcommon-x11.so.0, libEGL.so.1, which could not be loaded."
        ));
        assert!(text.contains("Install them, for example:"));
        assert!(text.contains("sudo apt install libxkbcommon-x11-0 libegl1"));
        assert!(text.contains("sudo dnf install libxkbcommon-x11 mesa-libEGL"));
        assert!(text.contains("sudo pacman -S libxkbcommon-x11 libglvnd"));
        assert!(text.ends_with("noter --tui"));
    }
}
