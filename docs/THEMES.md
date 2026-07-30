# Theme Contract

**Reviewed:** 2026-07-30

**Status:** Five built-in themes and the validated contributor extension path
are implemented. Loading user-authored theme files at runtime remains planned.

## Principles

Themes may change presentation, never document semantics or privacy behavior.
Every theme uses the same native text shaping, bundled document font, editing
model, accessibility tree, and local-only data path.

Noter currently provides System, Light, Dark, Green Screen, and Amber Screen.

## Adding a built-in theme

Built-in specialty themes are ordinary palette data in `src/theme.rs`. A new
theme requires all of the following:

1. add one stable `AppTheme` value, label, compact label, and storage value;
2. include it in `AppTheme::ALL` so the existing menu and round-trip tests own
   the choice;
3. define one complete opaque palette rather than mutating a previous theme;
4. pass the shared fail-closed validator and semantic menu tests; and
5. add native visual evidence on every supported platform before claiming the
   theme is verified.

The shared validator requires:

- primary text against the editor at 7:1 or better;
- secondary text, links, warnings, and errors against the editor at 4.5:1 or
  better;
- selected text against selection and active-control backgrounds at 4.5:1 or
  better;
- ordinary text against raised controls at 4.5:1 or better;
- control outlines against the panel at 3:1 or better; and
- opaque colors for every palette field.

An invalid palette falls back to the standard Dark palette. Validation is a
runtime boundary as well as a test assertion, so a future external loader
cannot bypass it accidentally.

## Safe custom-theme boundary

The planned custom-theme format is declarative data, not a plugin system. Its
loader must be versioned, size-bounded, strict about unknown fields, and routed
through the same palette validator. A theme may select colors. It may not
contain or reference:

- executable code, scripts, shaders, or dynamic libraries;
- local or remote images, fonts, sounds, HTML, or CSS;
- URLs, network requests, document paths, or environment variables;
- commands, key bindings, editor behavior, or accessibility overrides.

Theme files are not loaded by the current pre-alpha build. This keeps the
extension surface honest while the schema, path rules, persistence behavior,
error UX, and cross-platform evidence are completed under M2.
