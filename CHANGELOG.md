# Changelog

All notable changes to this project will be documented in this file.

The format is based on Keep a Changelog, and this project adheres to Semantic Versioning.

## [0.1.1] - 2026-02-07

### Changed

- Faster fd-like pruning emulation when filtering large Spotlight result sets.

### Fixed

- Removed an internal micro-benchmark that accidentally shipped in the test suite.

## [0.1.0] - 2026-02-06

### Added

- Initial release: Spotlight-backed filename search with fd v10.3.0-like ignore behavior (high-ROI subset).
- Hidden handling (`--hidden`), ignore disabling (`--no-ignore`), and NUL-delimited output (`--print0`).
- Smart-case filename matching.

