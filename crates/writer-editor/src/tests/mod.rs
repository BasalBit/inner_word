//! Headless tests (`#[gpui::test]`). They run on GPUI's test platform: no
//! real fonts, every glyph is 0.6 × font size wide, so geometry is
//! deterministic but not real.

mod slice;
