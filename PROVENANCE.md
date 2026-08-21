# Provenance and implementation record

Status: **independent implementation**. No upstream source file from
L-1124/QQMusicApi is incorporated in this repository. The earlier wording that
described the crate as "ported from" L-1124 was inaccurate and has been
removed.

## Maintainer attestation

- Maintainers: Osilvfe, Mai-xiyu.
- Date: 2026-08-21.
- Attestation: every Rust source file in this crate was written by the YAQMC
  maintainers without incorporating upstream L-1124/QQMusicApi source code or
  file content. L-1124/QQMusicApi was used only to match public API shapes and
  documented protocol behavior.

## Reference inputs

- L-1124/QQMusicApi, protocol/API reference only:
  <https://github.com/L-1124/QQMusicApi>
- L-1124 documentation: <https://l-1124.github.io/QQMusicApi/>
- Observed QQ Music desktop client protocol behavior, independently
  implemented.

## Obligations

- QMCDecode (MIT) mappings, copyright, and notice: `THIRD_PARTY_NOTICES.md`.
- This crate is GPL-3.0-or-later by the maintainers' choice; the L-1124
  reference carries no code-copy obligation under this record.
- If later review finds a module that tracks the upstream L-1124
  implementation structure, that module must be replaced or mapped through the
  crate-level provenance gate before distribution.

## Evidence

- Immutable record: this file as committed on `origin/main`.
