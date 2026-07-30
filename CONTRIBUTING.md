# Contributing

Noter welcomes focused bug reports, documentation improvements, tests, and code
changes. Changes that touch document bytes, filesystem behavior, lifecycle
state, or release trust require correspondingly strong evidence.

## Before making a change

1. Read the [roadmap](docs/ROADMAP.md) and the contract relevant to the change.
2. Check existing issues and pull requests to avoid duplicating active work.
3. Open an issue before a wide architecture change, new dependency, new network
   behavior, or change to the documented product boundaries.
4. Keep each pull request narrow enough to review and revert independently.

## Quality requirements

Follow the [development guide](docs/DEVELOPMENT.md) and
[code quality standards](docs/CODE-QUALITY-STANDARDS.md). Every change should:

- state the user problem, invariant, or reproduced defect it addresses;
- include the smallest test that would have failed before the change;
- preserve or improve the enforced coverage thresholds;
- update user-facing documentation and the changelog when behavior changes;
- avoid unrelated formatting or refactoring; and
- contain no private documents, local paths, credentials, or sensitive logs.

The required Windows, macOS, Linux, documentation, coverage, dependency, and
mutation jobs must pass on the exact commit proposed for merge. A local pass is
necessary but is not release evidence.

## Security reports

Do not open a public issue for a suspected vulnerability or data-loss path.
Follow the private reporting process in [SECURITY.md](SECURITY.md).

## License

By submitting a contribution, you agree that it may be distributed under the
project's [Apache License 2.0](LICENSE).
