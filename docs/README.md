# Noter Documentation

The root [README](../README.md) is the short project introduction. This index
routes users, contributors, and reviewers to the detailed source of truth for
each topic.

## Start here

| Goal | Document |
| --- | --- |
| Install, update, or remove Noter | [Installation and updates](INSTALLATION.md) |
| Understand Text Mode and Markdown Mode | [Native Markdown Mode](MARKDOWN.md) |
| See current work and release gates | [Roadmap](ROADMAP.md) |
| Build, test, or update screenshots | [Development guide](DEVELOPMENT.md) |
| Prepare or verify a release | [Release process](RELEASING.md) |
| Contribute a change | [Contributing guide](../CONTRIBUTING.md) |
| Play-test the current build | [Playtest brief](PLAYTEST.md) |
| Review local-data and network behavior | [Privacy contract](PRIVACY.md) |
| Report a vulnerability | [Security policy](../SECURITY.md) |

## Product contracts

- [Product requirements](REQUIREMENTS.md) define testable behavior for the
  first public-quality release.
- [UX direction](UX_VISION.md) defines the intended interaction and visual
  principles.
- [Native Markdown Mode](MARKDOWN.md) defines source authority, formatted
  editing, diagnostics, and completion evidence.
- [Theme contract](THEMES.md) defines the built-in palettes and safe extension
  boundary.
- [Privacy contract](PRIVACY.md) defines file, state, diagnostic, and network
  behavior.

## Engineering contracts

- [Technical design](DESIGN.md) defines architecture, failure modes, and
  verification layers.
- [Code quality standards](CODE-QUALITY-STANDARDS.md) define the merge gates.
- [Architecture decisions](adr/README.md) record accepted, costly-to-reverse
  choices.
- [Manual platform matrix](manual-test-matrix.md) defines release checks that
  automation cannot establish alone.
- [Release process](RELEASING.md) defines the dry-run, approval, artifact, and
  publication gates.

## Planning and evidence

- [Roadmap](ROADMAP.md) owns milestone order, status, and exit criteria.
- [Playtest brief](PLAYTEST.md) records what changed for the current round, what
  is deliberately still open, and which surfaces remain unexercised.
- [Research](RESEARCH.md) records dated primary-source findings behind product
  and engineering decisions.
- [Engineering baseline](BASELINE.md) records measured checkpoints and known
  gaps.
- [M1 reproducible baseline evidence](M1_BASELINE_EVIDENCE.md) records the
  canonical trust-kernel latency, memory, binary-size, and dependency reference.
- [M1 security review](M1_SECURITY_REVIEW.md) and
  [mutation evidence](M1_MUTATION_EVIDENCE.md) record durable-save review
  results.
- [M1 filesystem evidence](M1_FILESYSTEM_EVIDENCE.md) records native NTFS,
  native WSL2 ext4, and Windows-to-WSL boundary observations and remaining
  fixture gaps.
- [M3 editing evidence](M3_EDITING_EVIDENCE.md) records the exact-commit
  transaction, Undo, search, navigation, and lifecycle mutation campaign.
- [Architecture and product review](RIGOROUS_REVIEW.md) tracks the highest-risk
  remaining work and historical dispositions.
- The root [changelog](../CHANGELOG.md) records notable implementation changes.

Public claims are updated only when the cited command, platform, and commit
evidence exist. Future scope belongs in the roadmap rather than the root README.
