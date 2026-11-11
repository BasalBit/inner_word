//! Prints facts about the real macOS text system as seen through gpui 0.2.2:
//! installed font families, metrics, shaping, wrapping, caching cost.
//! Used to verify docs/gpui-notes.md. Opens a hidden window and quits.
//!
//! ```sh
//! cargo run -p writer-app --example gpui_probe
//! ```

use std::time::Instant;

use gpui::{
    App, Application, Bounds, Context, FontStyle, FontWeight, IntoElement, Render, SharedString,
    TextRun, Window, WindowBounds, WindowOptions, div, font, prelude::*, px, size,
};

struct Empty;

impl Render for Empty {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn run(font: gpui::Font, len: usize) -> TextRun {
    TextRun {
        len,
        font,
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

fn probe(window: &mut Window, cx: &mut App) {
    let names = cx.text_system().all_font_names();
    println!("font families: {}", names.len());
    for family in [
        "Charter",
        "Iowan Old Style",
        "Georgia",
        "SF Mono",
        "Menlo",
        "Monaco",
        "SF Pro",
        ".SystemUIFont",
        "New York",
        "Helvetica Neue",
        "Palatino",
        "Baskerville",
        "Avenir Next",
    ] {
        println!("  {family:<16} installed: {}", names.iter().any(|n| n == family));
    }
    println!("appearance: app {:?}, window {:?}", cx.window_appearance(), window.appearance());
    println!("scale factor: {}, rem size: {:?}", window.scale_factor(), window.rem_size());

    let ts = window.text_system().clone();
    let charter = font("Charter");
    let id = ts.resolve_font(&charter);
    println!(
        "Charter 17px: ascent {:?} descent {:?} cap {:?} x {:?} em width {:?}",
        ts.ascent(id, px(17.)),
        ts.descent(id, px(17.)),
        ts.cap_height(id, px(17.)),
        ts.x_height(id, px(17.)),
        ts.em_width(id, px(17.)).ok()
    );
    let missing = ts.resolve_font(&font("No Such Family"));
    println!(
        "missing family resolves to fallback: {:?}",
        ts.get_font_for_id(missing).map(|f| f.family)
    );

    let text: SharedString = "Hello world, naïve café".into();
    let line = ts.shape_line(text.clone(), px(17.), &[run(charter.clone(), text.len())], None);
    println!(
        "shape_line {:?}: width {:?}, ascent {:?}, descent {:?}, glyphs {}",
        text.as_ref(),
        line.width,
        line.ascent,
        line.descent,
        line.runs.iter().map(|r| r.glyphs.len()).sum::<usize>()
    );
    println!(
        "  x_for_index(5) {:?}, closest_index_for_x(40px) {}, index_for_x(40px) {:?}",
        line.x_for_index(5),
        line.closest_index_for_x(px(40.)),
        line.index_for_x(px(40.))
    );

    // Mixed fonts on one line (one font size only).
    let mixed = "regular bold italic code";
    let runs = vec![
        run(charter.clone(), 8),
        run(
            gpui::Font {
                weight: FontWeight::BOLD,
                ..charter.clone()
            },
            5,
        ),
        run(
            gpui::Font {
                style: FontStyle::Italic,
                ..charter.clone()
            },
            7,
        ),
        run(font("Menlo"), 4),
    ];
    let line = ts.shape_line(mixed.into(), px(17.), &runs, None);
    println!(
        "mixed runs: {} shaped runs, font ids {:?}",
        line.runs.len(),
        line.runs.iter().map(|r| r.font_id).collect::<Vec<_>>()
    );

    let lines = ts.shape_text(mixed.into(), px(17.), &runs, None, None).unwrap();
    println!(
        "mixed runs via shape_text: {} shaped runs, font ids {:?}",
        lines[0].runs().len(),
        lines[0].runs().iter().map(|r| r.font_id).collect::<Vec<_>>()
    );
    let similar: Vec<&String> = names
        .iter()
        .filter(|n| n.contains("Iowan") || n.starts_with("SF ") || n.starts_with(".SF"))
        .collect();
    println!("families like Iowan / SF: {similar:?}");

    // Ligatures and emoji: glyph count vs characters, glyph indices.
    for sample in ["office fit", "a😀b", "e\u{301}"] {
        let line = ts.shape_line(sample.into(), px(17.), &[run(charter.clone(), sample.len())], None);
        let glyphs: Vec<(usize, bool)> = line
            .runs
            .iter()
            .flat_map(|r| r.glyphs.iter().map(|g| (g.index, g.is_emoji)))
            .collect();
        println!(
            "glyphs for {sample:?} ({} chars): {:?}, x_for_index(1) {:?}",
            sample.chars().count(),
            glyphs,
            line.x_for_index(1)
        );
    }

    // Glyph fallback: characters Charter lacks come from other fonts.
    let sample = "a😀b日本";
    let line = ts.shape_line(sample.into(), px(17.), &[run(charter.clone(), sample.len())], None);
    println!(
        "fallback for {sample:?}: runs {:?}",
        line.runs
            .iter()
            .map(|r| (r.font_id, ts.get_font_for_id(r.font_id).map(|f| f.family), r.glyphs.len()))
            .collect::<Vec<_>>()
    );

    // Wrapping: per visual row ranges.
    let para = "It was a bright cold day in April, and the clocks were striking thirteen. \
                Winston Smith, his chin nuzzled into his breast in an effort to escape the vile wind.";
    let lines = ts
        .shape_text(para.into(), px(17.), &[run(charter.clone(), para.len())], Some(px(300.)), None)
        .unwrap();
    let wrapped = &lines[0];
    let mut starts = vec![0];
    starts.extend(
        wrapped
            .wrap_boundaries()
            .iter()
            .map(|b| wrapped.runs()[b.run_ix].glyphs[b.glyph_ix].index),
    );
    starts.push(para.len());
    println!("wrap at 300px: {} rows", starts.len() - 1);
    for pair in starts.windows(2) {
        println!("  {:>3}..{:<3} {:?}", pair[0], pair[1], &para[pair[0]..pair[1]]);
    }
    println!(
        "  size at line height 27.2px: {:?}",
        wrapped.size(px(17. * 1.6))
    );

    // Shaping cost and the per-frame cache.
    let t = Instant::now();
    for i in 0..1000 {
        let s = format!("{i} The quick brown fox jumps over the lazy dog, again and again.");
        let len = s.len();
        ts.shape_line(s.into(), px(17.), &[run(charter.clone(), len)], None);
    }
    println!("1000 distinct 60-char lines shaped in {:?}", t.elapsed());
    let t = Instant::now();
    for i in 0..1000 {
        let s = format!("{i} The quick brown fox jumps over the lazy dog, again and again.");
        let len = s.len();
        ts.shape_line(s.into(), px(17.), &[run(charter.clone(), len)], None);
    }
    println!("same 1000 lines again (cache hit) in {:?}", t.elapsed());
    let a = ts.shape_line(text.clone(), px(17.), &[run(charter.clone(), text.len())], None);
    let b = ts.shape_line(text.clone(), px(17.), &[run(charter.clone(), text.len())], None);
    println!(
        "same text shaped twice shares the layout Arc: {}",
        std::sync::Arc::ptr_eq(&*a, &*b)
    );
    let colored = TextRun {
        color: gpui::red(),
        ..run(charter.clone(), text.len())
    };
    let c = ts.shape_line(text.clone(), px(17.), &[colored], None);
    println!(
        "different colour, same font: layout shared: {}",
        std::sync::Arc::ptr_eq(&*a, &*c)
    );
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(400.), px(300.)),
                        cx,
                    ))),
                    show: false,
                    focus: false,
                    ..Default::default()
                },
                |_, cx| cx.new(|_| Empty),
            )
            .unwrap();
        window
            .update(cx, |_, window, cx| probe(window, cx))
            .unwrap();
        cx.quit();
    });
}
