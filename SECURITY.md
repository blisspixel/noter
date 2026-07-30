# Security Policy

Noter is under active development and does not yet publish a supported stable
release. Security and privacy defects are treated as product defects, and
security fixes currently target the latest `main` revision.

## Reporting a vulnerability

Use the repository Security tab and select **Report a vulnerability** when that
private reporting option is available. Do not disclose exploit details, private
documents, credentials, or personal information in a public issue.

If private reporting is unavailable, open a public issue that contains only a
request for a private reporting channel. A maintainer can then arrange a safe
way to receive the details.

Include the affected revision or version, operating system, impact, minimal
reproduction steps, and any conditions required to trigger the behavior. Use
synthetic test data rather than a real private document.

## Scope

Reports about local document confidentiality or integrity, durable saving,
filesystem race handling, installers and updates, dependency integrity, and
unexpected network access are especially useful. Reports about unsupported
future features should instead use the ordinary issue tracker without sensitive
details.

The project will validate reports, communicate confirmed impact as accurately as
possible, and coordinate remediation before public disclosure. Response timing
may vary until a supported stable release exists.
