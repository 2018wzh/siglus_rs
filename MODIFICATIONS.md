# Astra modifications

## Native resource language selection

The embedded typed configuration exposes `resource_language = original | chs`, defaulting to `original`. Standalone hosts can select the same `ResourceLanguage` in `SiglusHostConfig`. The selection is immutable for a runtime and propagates to configuration tables, scene reloads/metadata, images, cursors, composed images, and movies.

`chs` requires both `Scene.chs` and `Gameexe.chs` in the active Select.ini append. It never substitutes either half from the original pair, another append, or Data/. Native readers decode the selected packages; invalid configuration and missing configured startup/menu scene references fail explicitly. CHS decoding uses explicit project keys without original-pair key recovery or input key persistence.

CHS image lookup prefers the matching G01 over G00 and decodes it with the existing complete G00 album reader, preserving frame numbering. Other image formats and original-mode extension order remain unchanged. Movie replacement uses `mov_chs` only for the exact filename/extension selected by the normal `mov` resolver in the same append. Unreplaced resources retain their original native paths.

Font discovery remains the existing `font`/`fonts` mechanism. No speculative font override or binary-offset patches are added. The Rewrite sample still requires an actual font with glyph coverage and established semantics for all five legacy machine-code patch sites; these native resource capabilities alone do not establish complete localization readiness. Anemoi separately lacks a confirmed translation source.

Public generated fixtures cover paired native Scene/Gameexe parsing and strings, strict missing/incompatible pair errors, append boundaries, complete two-frame G01 decoding and rejection of an invalid later frame, movie path precedence, and typed ABI configuration. Tests do not execute a game VM. Build/test execution is performed by the integration owner.
