//! What `prepare` makes of files. Every file is written by the test with ImageIO; the
//! bytes of a file that was encoded again are never compared (they differ between
//! versions of macOS), only its kind, its size and that it can be read.

use super::*;
use crate::imageio::{Picture, Source, Written, uti, write};

const RED: [u8; 3] = [200, 30, 30];

/// A flat picture written as a file of `kind`; `None` when this Mac cannot write it.
fn file(width: u32, height: u32, alpha: Option<u8>, kind: &str, with: &Written) -> Option<Vec<u8>> {
    write(&[&Picture::flat(width, height, RED, alpha)], kind, with)
}

/// Pixels that do not compress: every byte from a small generator.
fn noise(width: u32, height: u32) -> Vec<u8> {
    let mut state: u32 = 0x2545_F491;
    (0..width as usize * height as usize * 4)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect()
}

/// What the stored bytes say about themselves, read back with ImageIO: they are a
/// picture that can be decoded.
fn read_back(prepared: &Prepared) -> crate::imageio::Facts {
    let source = Source::new(&prepared.bytes).expect("the stored bytes are an image");
    let facts = source.facts().expect("the stored bytes hold a picture");
    assert!(source.decode(64).is_some(), "the stored bytes decode");
    assert_eq!(ImageFormat::sniff(&prepared.bytes), Some(prepared.format));
    assert_eq!(stored_format(&facts.kind), Some(prepared.format));
    assert_eq!(facts.stored_pixels, prepared.pixels);
    assert_eq!(facts.orientation, 1, "stored pictures are upright");
    assert!(!facts.has_location, "stored pictures carry no position");
    assert!(prepared.bytes.len() <= MAX_STORED_BYTES);
    assert!(prepared.pixels.0.max(prepared.pixels.1) <= MAX_LONG_EDGE);
    facts
}

#[test]
fn the_limits() {
    assert_eq!(MAX_LONG_EDGE, 4096);
    assert_eq!(MAX_STORED_BYTES, 16_777_216);
    assert_eq!(MAX_INPUT_BYTES, 104_857_600);
    assert!(MAX_INPUT_BYTES > MAX_STORED_BYTES as u64);
    let decoded = u64::from(MAX_LONG_EDGE) * u64::from(MAX_LONG_EDGE) * 4;
    assert_eq!(decoded, 64 * 1024 * 1024);
}

#[test]
fn a_small_png_jpeg_and_gif_are_stored_as_they_came() {
    for (bytes, format) in [
        (fixtures::png(30, 20, RED), ImageFormat::Png),
        (fixtures::jpeg(30, 20, RED), ImageFormat::Jpeg),
        (fixtures::gif(30, 20, RED), ImageFormat::Gif),
    ] {
        let prepared = prepare(&bytes).unwrap();
        assert!(prepared.untouched, "{format:?}");
        assert_eq!(prepared.bytes, bytes, "{format:?}");
        assert_eq!(prepared.format, format);
        assert_eq!(prepared.pixels, (30, 20));
        assert_eq!(prepared.density, 72);
        read_back(&prepared);
        // The same file again is the same bytes, so the library stores it once.
        assert_eq!(prepare(&bytes).unwrap(), prepared);
    }
}

#[test]
fn the_fixtures_are_files_that_pass_as_they_are_with_the_size_and_density_asked_for() {
    type Case = (Vec<u8>, ImageFormat, (u32, u32), u32);
    let cases: [Case; 5] = [
        (fixtures::png(8, 6, [0, 0, 0]), ImageFormat::Png, (8, 6), 72),
        (
            fixtures::png_at(64, 48, [10, 200, 90], 144),
            ImageFormat::Png,
            (64, 48),
            144,
        ),
        (
            fixtures::png_at(64, 48, [10, 200, 90], 300),
            ImageFormat::Png,
            (64, 48),
            300,
        ),
        (
            fixtures::jpeg(33, 17, [250, 250, 250]),
            ImageFormat::Jpeg,
            (33, 17),
            72,
        ),
        (
            fixtures::gif(5, 9, [0, 0, 255]),
            ImageFormat::Gif,
            (5, 9),
            72,
        ),
    ];
    for (bytes, format, pixels, density) in cases {
        assert_eq!(ImageFormat::sniff(&bytes), Some(format));
        let prepared = prepare(&bytes).unwrap();
        assert!(prepared.untouched);
        assert_eq!(prepared.bytes, bytes);
        assert_eq!(
            (prepared.format, prepared.pixels, prepared.density),
            (format, pixels, density)
        );
        // Opaque: the file says so, or says nothing.
        assert_ne!(read_back(&prepared).has_alpha, Some(true));
    }
    // A fixture is the same file every time it is made: the same id in the library.
    assert_eq!(fixtures::png(8, 6, RED), fixtures::png(8, 6, RED));
    assert_ne!(fixtures::png(8, 6, RED), fixtures::png(8, 6, [0, 0, 0]));
}

#[test]
fn what_the_library_is_handed() {
    let prepared = prepare(&fixtures::png_at(40, 30, RED, 144)).unwrap();
    let new = prepared.as_new_image();
    assert_eq!(new.bytes, prepared.bytes.as_slice());
    assert_eq!(new.format, ImageFormat::Png);
    assert_eq!((new.width, new.height, new.density), (40, 30, 144));
}

/// The seam to the store: what `prepare` gives is what the library takes, and the id it
/// is stored under is the hash of the prepared bytes.
#[test]
fn what_is_prepared_goes_into_a_library_as_it_is() {
    use innerword_store::{ImageId, Store};

    let mut store = Store::open_in_memory().unwrap();
    let prepared = prepare(&fixtures::png_at(40, 30, RED, 144)).unwrap();
    let info = store.put_image(&prepared.as_new_image()).unwrap();
    assert_eq!(info.id, ImageId::of(&prepared.bytes));
    assert_eq!(info.format, prepared.format);
    assert_eq!((info.width, info.height), prepared.pixels);
    assert_eq!(info.density, prepared.density);
    assert_eq!(info.byte_len, prepared.bytes.len() as u64);
    assert_eq!(
        store.image_bytes(&info.id).unwrap(),
        Some(prepared.bytes.clone())
    );

    // The same file prepared again is the same image, and one that was converted is
    // another, stored under the hash of what it was converted to.
    let again = prepare(&fixtures::png_at(40, 30, RED, 144)).unwrap();
    assert_eq!(store.put_image(&again.as_new_image()).unwrap(), info);
    let tiff = file(40, 30, None, uti::TIFF, &Written::default()).unwrap();
    let converted = prepare(&tiff).unwrap();
    let other = store.put_image(&converted.as_new_image()).unwrap();
    assert_eq!(other.id, ImageId::of(&converted.bytes));
    assert_ne!(other.id, ImageId::of(&tiff));
    assert_eq!(other.format, ImageFormat::Jpeg);
    assert_eq!(store.image_count().unwrap(), 2);
}

#[test]
fn a_jpeg_that_lies_on_its_side_is_stored_upright() {
    let with = Written {
        quality: Some(0.9),
        orientation: Some(6),
        ..Written::default()
    };
    let bytes = file(40, 20, None, uti::JPEG, &with).unwrap();
    let before = Source::new(&bytes).unwrap().facts().unwrap();
    assert_eq!((before.stored_pixels, before.orientation), ((40, 20), 6));

    let prepared = prepare(&bytes).unwrap();
    assert!(!prepared.untouched);
    assert_eq!(prepared.format, ImageFormat::Jpeg);
    assert_eq!(prepared.pixels, (20, 40), "width and height are swapped");
    assert_eq!(prepared.density, 72);
    read_back(&prepared);

    // And it is really turned, not only measured the other way round. A dark picture
    // with a light square in its top left corner, declared as lying on its right side
    // (orientation 6: the stored top is the right edge of the picture as it is meant):
    // upright, the square is in the top right corner.
    let mut pixels = vec![0u8; 40 * 20 * 4];
    for y in 0..10 {
        for x in 0..10 {
            pixels[(y * 40 + x) * 4..(y * 40 + x) * 4 + 3].fill(255);
        }
    }
    let marked = Picture::from_pixels(40, 20, &pixels, false);
    let with = Written {
        quality: Some(1.0),
        orientation: Some(6),
        ..Written::default()
    };
    let prepared = prepare(&write(&[&marked], uti::JPEG, &with).unwrap()).unwrap();
    assert_eq!(prepared.pixels, (20, 40));
    let upright = Source::new(&prepared.bytes).unwrap().decode(40).unwrap();
    assert_eq!(upright.pixels(), (20, 40));
    let light = |x, y| upright.lightness_at(x, y).unwrap() > 128;
    assert!(light(15, 4), "the square is at the top right");
    assert!(!light(4, 4), "and not at the top left");
    assert!(!light(4, 35) && !light(15, 35), "nor at the bottom");

    // The other turned and mirrored ways too: 2 to 4 keep the shape, 5 to 8 swap it.
    for orientation in 2..=8u8 {
        let with = Written {
            quality: Some(0.9),
            orientation: Some(orientation),
            ..Written::default()
        };
        let prepared = prepare(&file(40, 20, None, uti::JPEG, &with).unwrap()).unwrap();
        let expected = if orientation >= 5 { (20, 40) } else { (40, 20) };
        assert_eq!(prepared.pixels, expected, "orientation {orientation}");
        assert!(!prepared.untouched);
        read_back(&prepared);
    }
}

/// The test that decides between the two ways of taking a position out: the copy that
/// leaves the picture as it is, or encoding again. Whichever this Mac does, what is
/// stored has no position and is the same picture size.
#[test]
fn a_jpeg_with_a_position_is_stored_without_it() {
    let with = Written {
        quality: Some(0.9),
        location: Some((41.3275, 19.8187)),
        dpi: Some(200),
        ..Written::default()
    };
    let bytes = file(64, 48, None, uti::JPEG, &with).unwrap();
    let source = Source::new(&bytes).unwrap();
    let before = source.facts().unwrap();
    assert!(before.has_location, "the test's file carries a position");
    assert_eq!(before.orientation, 1);

    let lossless = without_location(&source, &before, Some(ImageFormat::Jpeg));
    eprintln!(
        "position taken out of a JPEG: {}",
        if lossless.is_some() {
            "by the copy that leaves the picture as it is (CGImageDestinationCopyImageSource)"
        } else {
            "by encoding the picture again (the lossless copy did not do it on this macOS)"
        }
    );

    let prepared = prepare(&bytes).unwrap();
    assert!(!prepared.untouched, "the bytes are not the input's own");
    assert_ne!(prepared.bytes, bytes);
    assert_eq!(prepared.format, ImageFormat::Jpeg);
    assert_eq!(prepared.pixels, (64, 48));
    assert_eq!(prepared.density, 200);
    let after = read_back(&prepared);
    assert!(!after.has_location);
    if let Some(lossless) = lossless {
        assert_eq!(prepared.bytes, lossless);
        // The picture itself was not encoded again: from the start of its scan to the
        // end the two files are the same bytes. Only what the file says about itself
        // was written anew.
        let scan = |file: &[u8]| {
            let start = file
                .windows(2)
                .rposition(|marker| marker == [0xFF, 0xDA])
                .expect("a JPEG has a scan");
            file[start..].to_vec()
        };
        assert!(scan(&bytes).len() > 20);
        assert_eq!(scan(&prepared.bytes), scan(&bytes));
    }

    // The same for a PNG.
    let with = Written {
        location: Some((-33.9, -70.6)),
        ..Written::default()
    };
    let bytes = file(64, 48, None, uti::PNG, &with).unwrap();
    if Source::new(&bytes).unwrap().facts().unwrap().has_location {
        let source = Source::new(&bytes).unwrap();
        let facts = source.facts().unwrap();
        eprintln!(
            "position taken out of a PNG: {}",
            if without_location(&source, &facts, Some(ImageFormat::Png)).is_some() {
                "by the copy that leaves the picture as it is"
            } else {
                "by encoding the picture again"
            }
        );
        let prepared = prepare(&bytes).unwrap();
        assert!(!prepared.untouched);
        assert_eq!(prepared.pixels, (64, 48));
        // Either way it stays a PNG: nothing of the picture is lost.
        assert_eq!(prepared.format, ImageFormat::Png);
        read_back(&prepared);
    } else {
        eprintln!("this Mac's ImageIO writes no position into a PNG: nothing to take out");
    }
}

#[test]
fn other_kinds_of_file_become_a_jpeg_or_with_transparency_a_png() {
    // A TIFF and a BMP: opaque, so a JPEG.
    for kind in [uti::TIFF, uti::BMP] {
        let bytes = file(50, 40, None, kind, &Written::default()).unwrap();
        assert_eq!(ImageFormat::sniff(&bytes), None);
        let prepared = prepare(&bytes).unwrap();
        assert!(!prepared.untouched, "{kind}");
        assert_eq!(prepared.format, ImageFormat::Jpeg, "{kind}");
        assert_eq!(prepared.pixels, (50, 40), "{kind}");
        assert_eq!(prepared.density, 72, "{kind}");
        read_back(&prepared);
    }

    // A TIFF with transparency: a PNG, and still transparent.
    let bytes = file(50, 40, Some(128), uti::TIFF, &Written::default()).unwrap();
    let prepared = prepare(&bytes).unwrap();
    assert!(!prepared.untouched);
    assert_eq!(prepared.format, ImageFormat::Png);
    assert_eq!(prepared.pixels, (50, 40));
    let facts = read_back(&prepared);
    assert_eq!(facts.has_alpha, Some(true));

    // A TIFF that declares a density keeps it.
    let with = Written {
        dpi: Some(300),
        ..Written::default()
    };
    let prepared = prepare(&file(50, 40, None, uti::TIFF, &with).unwrap()).unwrap();
    assert_eq!(prepared.density, 300);
    assert_eq!(read_back(&prepared).dpi.map(f64::round), Some(300.0));

    // A HEIC, as an iPhone takes them, where this Mac can write one for the test.
    match file(64, 48, None, uti::HEIC, &Written::default()) {
        Some(bytes) => {
            let prepared = prepare(&bytes).unwrap();
            assert!(!prepared.untouched);
            assert_eq!(prepared.format, ImageFormat::Jpeg);
            assert_eq!(prepared.pixels, (64, 48));
            read_back(&prepared);
            eprintln!("HEIC: written by this Mac and stored as a JPEG");
            // ImageIO decodes a HEIC into pixels with an alpha channel although the
            // file has none: the pixels are looked at, and an opaque photograph does
            // not become a PNG.
            let source = Source::new(&bytes).unwrap();
            assert_eq!(source.facts().unwrap().has_alpha, None);
            assert!(!source.decode(64).unwrap().has_transparency());
        }
        None => eprintln!("HEIC: skipped, this Mac's ImageIO cannot write one"),
    }
}

#[test]
fn a_picture_over_the_long_edge_is_made_smaller() {
    // 5000 wide: 4096 wide afterwards, the height in proportion. Opaque, so a JPEG.
    let prepared = prepare(&fixtures::png(5000, 100, RED)).unwrap();
    assert!(!prepared.untouched);
    assert_eq!(prepared.format, ImageFormat::Jpeg);
    assert_eq!(prepared.pixels, (4096, 82));
    read_back(&prepared);

    // Tall instead of wide.
    let prepared = prepare(&fixtures::png(100, 4100, RED)).unwrap();
    assert_eq!(prepared.pixels, (100, 4096));

    // With transparency it stays a PNG.
    let bytes = file(4200, 60, Some(100), uti::PNG, &Written::default()).unwrap();
    let prepared = prepare(&bytes).unwrap();
    assert_eq!(prepared.format, ImageFormat::Png);
    assert_eq!(prepared.pixels, (4096, 59));
    assert_eq!(read_back(&prepared).has_alpha, Some(true));

    // The density goes down with the pixels, so the picture is as large on the page as
    // it was: 5000 pixels at 300 to the inch are 4096 at 246.
    let prepared = prepare(&fixtures::png_at(5000, 100, RED, 300)).unwrap();
    assert_eq!(prepared.pixels, (4096, 82));
    assert_eq!(prepared.density, 246);
    assert_eq!(read_back(&prepared).dpi.map(f64::round), Some(246.0));

    // Exactly at the limit nothing is done.
    let bytes = fixtures::png(4096, 10, RED);
    let prepared = prepare(&bytes).unwrap();
    assert!(prepared.untouched);
    assert_eq!(prepared.pixels, (4096, 10));

    // A JPEG that lies on its side and is too long: upright and smaller in one step.
    let with = Written {
        quality: Some(0.9),
        orientation: Some(8),
        ..Written::default()
    };
    let prepared = prepare(&file(5000, 200, None, uti::JPEG, &with).unwrap()).unwrap();
    assert_eq!(prepared.pixels, (164, 4096));
    read_back(&prepared);
}

#[test]
fn a_density_is_believed_within_reason() {
    assert_eq!(
        prepare(&fixtures::png_at(20, 20, RED, 144))
            .unwrap()
            .density,
        144
    );
    assert_eq!(
        prepare(&fixtures::png_at(20, 20, RED, 36)).unwrap().density,
        36
    );
    assert_eq!(
        prepare(&fixtures::png_at(20, 20, RED, 1200))
            .unwrap()
            .density,
        1200
    );
    // What cannot be meant counts as nothing said.
    assert_eq!(
        prepare(&fixtures::png_at(20, 20, RED, 10)).unwrap().density,
        72
    );
    assert_eq!(
        prepare(&fixtures::png_at(20, 20, RED, 5000))
            .unwrap()
            .density,
        72
    );
}

#[test]
fn a_file_over_the_stored_size_is_encoded_again_until_it_fits() {
    // An opaque PNG of noise, 2400 pixels square: over 16 MB as a PNG, a few as a JPEG.
    let pixels = noise(2400, 2400);
    let picture = Picture::from_pixels(2400, 2400, &pixels, false);
    let bytes = write(&[&picture], uti::PNG, &Written::default()).unwrap();
    assert!(bytes.len() > MAX_STORED_BYTES, "{}", bytes.len());
    let prepared = prepare(&bytes).unwrap();
    assert!(!prepared.untouched);
    assert_eq!(prepared.format, ImageFormat::Jpeg);
    assert_eq!(prepared.pixels, (2400, 2400));
    read_back(&prepared);

    // The same with transparency has to stay a PNG, and a PNG of noise does not get
    // smaller by being written again: the picture is made smaller until its file fits.
    let picture = Picture::from_pixels(2400, 2400, &pixels, true);
    let bytes = write(&[&picture], uti::PNG, &Written::default()).unwrap();
    assert!(bytes.len() > MAX_STORED_BYTES, "{}", bytes.len());
    let prepared = prepare(&bytes).unwrap();
    assert!(!prepared.untouched);
    assert_eq!(prepared.format, ImageFormat::Png);
    assert!(
        prepared.pixels.0 < 2400 && prepared.pixels.0 >= 512,
        "{:?}",
        prepared.pixels
    );
    assert_eq!(prepared.pixels.0, prepared.pixels.1);
    assert_eq!(read_back(&prepared).has_alpha, Some(true));
}

#[test]
fn an_animated_gif_is_kept_whole_or_refused() {
    let first = Picture::flat(40, 30, RED, None);
    let second = Picture::flat(40, 30, [20, 20, 220], None);
    let bytes = write(&[&first, &second], uti::GIF, &Written::default()).unwrap();
    let facts = Source::new(&bytes).unwrap().facts().unwrap();
    assert_eq!(facts.count, 2, "the test's file is an animation");

    let prepared = prepare(&bytes).unwrap();
    assert!(prepared.untouched);
    assert_eq!(prepared.bytes, bytes);
    assert_eq!(prepared.format, ImageFormat::Gif);
    assert_eq!(prepared.pixels, (40, 30));

    // Over the long edge it cannot be made smaller without losing the animation.
    let first = Picture::flat(4100, 4, RED, None);
    let second = Picture::flat(4100, 4, [20, 20, 220], None);
    let bytes = write(&[&first, &second], uti::GIF, &Written::default()).unwrap();
    let error = prepare(&bytes).unwrap_err();
    assert_eq!(error, PrepareError::TooLarge { animated: true });

    // A GIF with one frame that is too long is a picture like any other.
    let prepared = prepare(&fixtures::gif(4100, 4, RED)).unwrap();
    assert!(!prepared.untouched);
    assert_eq!(prepared.pixels, (4096, 4));
    assert!(matches!(
        prepared.format,
        ImageFormat::Jpeg | ImageFormat::Png
    ));
    read_back(&prepared);
}

#[test]
fn what_is_not_a_picture_is_refused_in_a_sentence_that_names_it() {
    let subject = "“harbour.jpg”";

    let error = prepare(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n1 0 obj\n<<>>\nendobj\n").unwrap_err();
    assert_eq!(error, PrepareError::Unsupported("PDF"));
    assert_eq!(
        error.sentence(subject),
        "“harbour.jpg” is a PDF file. Images can be PNG, JPEG, GIF, HEIC, TIFF, WebP or BMP."
    );

    for svg in [
        &br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>"#[..],
        &b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"x\">\n<svg width=\"1\"/>"[..],
        &b"\xEF\xBB\xBF\n  <!-- drawn by hand -->\n<svg/>"[..],
    ] {
        let error = prepare(svg).unwrap_err();
        assert_eq!(error, PrepareError::Unsupported("SVG"));
        assert_eq!(
            error.sentence(subject),
            "“harbour.jpg” is an SVG file. Images can be PNG, JPEG, GIF, HEIC, TIFF, WebP or BMP."
        );
    }

    for not_an_image in [
        &b""[..],
        &b"These are words and no picture."[..],
        &[
            0x13, 0x37, 0xC0, 0xFF, 0xEE, 0x00, 0x42, 0x99, 0x10, 0x20, 0x30, 0x40,
        ][..],
        // The first bytes of a PNG and nothing after them.
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A][..],
        // HTML is text that begins with a tag and is no SVG.
        &b"<html><body>no picture</body></html>"[..],
    ] {
        let error = prepare(not_an_image).unwrap_err();
        assert_eq!(error, PrepareError::NotAnImage, "{not_an_image:?}");
        assert_eq!(error.sentence(subject), "“harbour.jpg” is not an image.");
    }

    // A picture whose text holds the words is still a picture.
    let mut png = fixtures::png(4, 4, RED);
    png.extend_from_slice(b"<svg><?xml %PDF");
    assert!(prepare(&png).is_ok());
}

#[test]
fn every_error_has_its_sentence() {
    let subject = "The picture on the clipboard";
    assert_eq!(
        PrepareError::NotAnImage.sentence(subject),
        "The picture on the clipboard is not an image."
    );
    assert_eq!(
        PrepareError::Unsupported("SVG").sentence(subject),
        "The picture on the clipboard is an SVG file. Images can be PNG, JPEG, GIF, HEIC, TIFF, WebP or BMP."
    );
    assert_eq!(
        PrepareError::Unsupported("PDF").sentence(subject),
        "The picture on the clipboard is a PDF file. Images can be PNG, JPEG, GIF, HEIC, TIFF, WebP or BMP."
    );
    assert_eq!(
        PrepareError::TooLarge { animated: false }.sentence(subject),
        "The picture on the clipboard is larger than 100 MB."
    );
    assert_eq!(
        PrepareError::TooLarge { animated: true }.sentence(subject),
        "The picture on the clipboard is an animated GIF over 16 MB or 4096 pixels, more than a document can hold."
    );
    assert_eq!(
        PrepareError::Failed("the disk said no".to_owned()).sentence(subject),
        "The picture on the clipboard could not be read as an image."
    );
}

#[test]
fn more_than_a_hundred_megabytes_is_refused_before_anything_is_read() {
    // Bytes: over the limit by one. (Zeroes cost nothing until they are touched.)
    let bytes = vec![0u8; MAX_INPUT_BYTES as usize + 1];
    assert_eq!(
        prepare(&bytes).unwrap_err(),
        PrepareError::TooLarge { animated: false }
    );
    drop(bytes);

    // A file: its length is asked, and the file is not read.
    let dir = std::env::temp_dir().join(format!("innerword-image-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let large = dir.join("large.png");
    std::fs::File::create(&large)
        .unwrap()
        .set_len(MAX_INPUT_BYTES + 1)
        .unwrap();
    assert_eq!(
        prepare_file(&large).unwrap_err(),
        PrepareError::TooLarge { animated: false }
    );

    // A file as it should be, a file that is no picture, and one that is not there.
    let good = dir.join("good.png");
    let bytes = fixtures::png_at(12, 8, RED, 144);
    std::fs::write(&good, &bytes).unwrap();
    let prepared = prepare_file(&good).unwrap();
    assert!(prepared.untouched);
    assert_eq!(prepared.bytes, bytes);
    assert_eq!((prepared.pixels, prepared.density), ((12, 8), 144));

    let text = dir.join("notes.png");
    std::fs::write(&text, "not a picture, whatever the name says").unwrap();
    assert_eq!(prepare_file(&text).unwrap_err(), PrepareError::NotAnImage);

    // A path that names no plain file is not read: a folder, a device whose length is 0
    // and whose bytes never end, a named pipe that nobody writes to (opening it would
    // wait for a writer for ever).
    assert_eq!(prepare_file(&dir).unwrap_err(), PrepareError::NotAnImage);
    assert_eq!(
        prepare_file(Path::new("/dev/zero")).unwrap_err(),
        PrepareError::NotAnImage
    );
    let pipe = dir.join("pipe.png");
    let made = std::process::Command::new("mkfifo")
        .arg(&pipe)
        .status()
        .unwrap();
    assert!(made.success());
    let (sender, answer) = std::sync::mpsc::channel();
    let reading = pipe.clone();
    std::thread::spawn(move || sender.send(prepare_file(&reading)).unwrap());
    let refused = answer
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("a named pipe was opened or read");
    assert_eq!(refused.unwrap_err(), PrepareError::NotAnImage);
    // Through a link the file it names counts.
    let link = dir.join("link.png");
    std::os::unix::fs::symlink(&good, &link).unwrap();
    assert_eq!(prepare_file(&link).unwrap().bytes, bytes);

    let missing = prepare_file(&dir.join("nowhere.png")).unwrap_err();
    assert!(matches!(missing, PrepareError::Failed(_)), "{missing:?}");
    assert_eq!(
        missing.sentence("“nowhere.png”"),
        "“nowhere.png” could not be read as an image."
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn preparing_works_on_several_threads_at_once() {
    let threads: Vec<_> = (0..4u8)
        .map(|n| {
            std::thread::spawn(move || {
                let bytes = fixtures::png(200 + u32::from(n), 100, [n * 40, 10, 10]);
                let prepared = prepare(&bytes).unwrap();
                assert!(prepared.untouched);
                let tiff = file(300, 200, None, uti::TIFF, &Written::default()).unwrap();
                let converted = prepare(&tiff).unwrap();
                (prepared.pixels, converted.format)
            })
        })
        .collect();
    for (n, thread) in threads.into_iter().enumerate() {
        let (pixels, format) = thread.join().unwrap();
        assert_eq!(pixels, (200 + n as u32, 100));
        assert_eq!(format, ImageFormat::Jpeg);
    }
}
