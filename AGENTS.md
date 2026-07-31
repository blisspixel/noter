# Repository Working Agreement

Read the root README, `docs/ROADMAP.md`, the relevant product contract, and
`docs/CODE-QUALITY-STANDARDS.md` before changing behavior. Product requirements
belong in `docs/REQUIREMENTS.md`, planned work in `docs/ROADMAP.md`, architecture
in `docs/DESIGN.md`, and user-visible changes in `CHANGELOG.md`.

Keep Noter focused, private, defensive, and cross-platform. Do not add accounts,
telemetry, advertising, cloud document formats, background network access, or
bundled AI. Preserve ordinary text and Markdown source, and never weaken data
safety to simplify an interface.

Use the existing source layout. Avoid duplicated domain logic, placeholders,
TODOs, dead code, and commented-out implementations. Keep comments accurate and
limited to decisions the code cannot express clearly. Do not add emojis, em
dashes, or tool or model attribution to repository content.

Every change must pass formatting, lint, tests, documentation validation,
dependency policy, and at least 80 percent meaningful whole-workspace line
coverage. UI changes also require regenerated Light and Dark screenshots and
visual review. Security-sensitive and release changes require the additional
evidence defined by the quality standards and roadmap.

Keep one protected `main` branch as the integration branch. Stage only the
intended change, require exact-head CI before merge, and do not commit local
automation state, logs, build output, secrets, or release credentials.
