# Changelog

All notable changes to Colossus will be documented in this file.

This project follows the spirit of [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and uses semantic versioning before 1.0 with the usual caveat that minor versions may
include breaking changes while the public API is still settling.

## [Unreleased]

### Added

- Release-readiness documentation for installation, configuration, offline and
  airgapped operation, bundle format, release process, and security policy.
- Continuous integration workflow covering tests, linting, type checking, and package
  builds.

## [0.1.0] - 2026-06-08

### Added

- Initial secure layered CLI harness with CLI, REPL, and TUI interfaces.
- Ports-and-adapters architecture with dependency-inward boundaries.
- Deterministic echo provider and OpenAI-compatible provider adapters.
- Brokered built-in tool metadata, policy decisions, local state, and audit logging.
- Bundled skills and offline bundle manifest verification.
