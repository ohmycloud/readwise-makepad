# Local Makepad draw patch

This crate is copied from Makepad's `draw` crate at commit
`4f9ce7a8bb3fd19e5c61dcf13edd2e6d4a04cefc` (the same revision used by
the rest of the workspace). Its relative workspace dependencies have been
changed to Git dependencies so Cargo can patch only `makepad-draw`.

ReadWise adds letter spacing, word spacing, and justified wrapped rows to the
text shaping and layout options. The reader's TextFlow and layout preview pass
those options through `TextStyle`, so selection geometry and line wrapping use
the same glyph positions as the rendered text.

The original Makepad license is preserved in `LICENSE`.
