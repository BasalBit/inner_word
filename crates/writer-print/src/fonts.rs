//! From a [`FontSpec`] to a `CTFont`: the font the layout meant.
//!
//! The layout names a font the way the app does (a family, a weight, a style, the
//! families to take a missing glyph from). GPUI turned that into a face before it
//! shaped the text; this module turns it into the same face again, by the same
//! steps (`gpui-0.2.2/src/platform/mac/text_system.rs`, `font_id` and
//! `load_family`, and `open_type.rs`, `apply_features_and_fallbacks`):
//!
//! 1. the family is looked up by name with font-kit (`select_family_by_name`);
//!    `.SystemUIFont` is asked for as `.AppleSystemUIFont`;
//! 2. every face of the family is loaded and given the cascade list (one descriptor
//!    per fallback family), and a face without an `m` or with unreadable traits is
//!    left out;
//! 3. `find_best_match` picks the face for the weight and the style;
//! 4. a family that cannot be had goes down GPUI's own stack of last resorts
//!    (Helvetica is the first of them a Mac has), in the regular face, and is
//!    remembered as missing.
//!
//! A font is never made from a PostScript name: asked for `.SFNS-Regular`, the
//! interface font, CoreText answers with Times New Roman.
//!
//! What GPUI can have and this cannot: a font the app registered from memory
//! (`TextSystem::add_fonts`). The Writer registers none.

use std::{
    collections::{BTreeSet, HashMap},
    ptr,
    sync::Arc,
};

use core_foundation::{
    array::CFArray,
    base::{CFType, TCFType},
    dictionary::{CFDictionary, CFDictionaryRef},
    number::CFNumber,
    string::CFString,
};
use core_graphics::base::CGFloat;
use core_text::{
    font::{CTFont, CTFontRef},
    font_descriptor::{
        CTFontDescriptor, CTFontDescriptorCreateWithAttributes,
        CTFontDescriptorCreateWithNameAndSize, CTFontDescriptorRef, kCTFontCascadeListAttribute,
        kCTFontFeatureSettingsAttribute, kCTFontSlantTrait, kCTFontSymbolicTrait,
        kCTFontWeightTrait, kCTFontWidthTrait,
    },
};
use font_kit::{
    font::Font,
    matching::find_best_match,
    properties::{Properties, Stretch, Style, Weight},
    source::SystemSource,
};

use crate::page::FontSpec;

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    fn CTFontCreateCopyWithAttributes(
        font: CTFontRef,
        size: CGFloat,
        matrix: *const core_graphics::geometry::CGAffineTransform,
        attributes: CTFontDescriptorRef,
    ) -> CTFontRef;
}

/// The families GPUI tries, in this order, when the one that was asked for cannot
/// be had (`TextSystem::fallback_font_stack`), under the names it looks them up
/// by. Each is asked for in its regular face, whatever the face that was wanted.
const LAST_RESORTS: [&str; 10] = [
    ".ZedMono",
    ".ZedSans",
    "Helvetica",
    "Segoe UI",
    "Ubuntu",
    "Adwaita Sans",
    "Cantarell",
    "Noto Sans",
    "DejaVu Sans",
    "Arial",
];

/// The name a family is looked up by (`gpui::text_system::font_name_with_fallbacks`).
fn lookup_name(family: &str) -> &str {
    match family {
        ".SystemUIFont" => ".AppleSystemUIFont",
        ".ZedSans" | "Zed Plex Sans" => "IBM Plex Sans",
        ".ZedMono" | "Zed Plex Mono" => "Lilex",
        other => other,
    }
}

/// One face of a family, with the cascade list of the font that asked for it.
struct Face {
    font: CTFont,
    properties: Properties,
}

/// A [`FontSpec`] as a key: the weight by its bits.
#[derive(Clone, PartialEq, Eq, Hash)]
struct SpecKey {
    family: String,
    weight: u32,
    italic: bool,
    fallbacks: Vec<String>,
}

impl SpecKey {
    fn of(spec: &FontSpec) -> Self {
        Self {
            family: spec.family.clone(),
            weight: spec.weight.to_bits(),
            italic: spec.italic,
            fallbacks: spec.fallbacks.clone(),
        }
    }
}

/// A family under a cascade list: the name it is looked up by, and the fallback
/// families.
type FamilyKey = (String, Vec<String>);

/// The fonts of one PDF: every family, face and size that was asked for, found
/// once.
pub(crate) struct Fonts {
    source: SystemSource,
    /// The usable faces of a family under a cascade list; `None` for a family
    /// that cannot be had.
    families: HashMap<FamilyKey, Option<Arc<Vec<Face>>>>,
    /// The face of a spec, at font-kit's own size.
    faces: HashMap<SpecKey, CTFont>,
    /// The face of a spec at a size.
    sized: HashMap<(SpecKey, u32), CTFont>,
    /// The same by the address of a shared spec, which is what the rows of a
    /// document carry: a row's font is found without hashing its names. The
    /// entry holds the spec, so the address cannot come to mean another one.
    shared: HashMap<(usize, u32), (Arc<FontSpec>, CTFont)>,
    /// The families that were asked for and could not be had.
    missing: BTreeSet<String>,
}

impl Fonts {
    pub(crate) fn new() -> Self {
        Self {
            source: SystemSource::new(),
            families: HashMap::new(),
            faces: HashMap::new(),
            sized: HashMap::new(),
            shared: HashMap::new(),
            missing: BTreeSet::new(),
        }
    }

    /// The font of `spec` at `size` points.
    pub(crate) fn font(&mut self, spec: &Arc<FontSpec>, size: f32) -> CTFont {
        let shared = (Arc::as_ptr(spec) as usize, size.to_bits());
        if let Some((_, font)) = self.shared.get(&shared) {
            return font.clone();
        }
        let font = self.font_of(spec, size);
        self.shared.insert(shared, (spec.clone(), font.clone()));
        font
    }

    fn font_of(&mut self, spec: &FontSpec, size: f32) -> CTFont {
        let key = (SpecKey::of(spec), size.to_bits());
        if let Some(font) = self.sized.get(&key) {
            return font.clone();
        }
        let face = match self.faces.get(&key.0) {
            Some(face) => face.clone(),
            None => {
                let face = self.resolve(spec);
                self.faces.insert(key.0.clone(), face.clone());
                face
            }
        };
        // What GPUI does for every run it shapes.
        let font = face.clone_with_font_size(f64::from(size.max(0.01)));
        self.sized.insert(key, font.clone());
        font
    }

    /// The families that were asked for and are not installed, sorted.
    pub(crate) fn missing(&self) -> Vec<String> {
        self.missing.iter().cloned().collect()
    }

    /// The face of a spec: its own, or the first last resort that can be had.
    fn resolve(&mut self, spec: &FontSpec) -> CTFont {
        if let Some(font) = self.face(&spec.family, spec.weight, spec.italic, &spec.fallbacks) {
            return font;
        }
        self.missing.insert(spec.family.clone());
        for family in LAST_RESORTS {
            if let Some(font) = self.face(family, Weight::NORMAL.0, false, &[]) {
                return font;
            }
        }
        // GPUI gives up here. A page is still better printed in the system's
        // own font than not at all.
        core_text::font::new_ui_font_for_language(core_text::font::kCTFontSystemFontType, 16., None)
    }

    /// The face of a family for a weight and a style, if the family can be had.
    fn face(
        &mut self,
        family: &str,
        weight: f32,
        italic: bool,
        fallbacks: &[String],
    ) -> Option<CTFont> {
        let name = lookup_name(family);
        let key = (name.to_string(), fallbacks.to_vec());
        let faces = match self.families.get(&key) {
            Some(faces) => faces.clone(),
            None => {
                let faces = load_family(&self.source, name, fallbacks).map(Arc::new);
                self.families.insert(key, faces.clone());
                faces
            }
        }?;
        let candidates: Vec<Properties> = faces.iter().map(|face| face.properties).collect();
        let wanted = Properties {
            style: if italic { Style::Italic } else { Style::Normal },
            weight: Weight(weight),
            stretch: Stretch::NORMAL,
        };
        let index = find_best_match(&candidates, &wanted).ok()?;
        Some(faces[index].font.clone())
    }
}

/// Every usable face of a family, each with the cascade list. `None` when the
/// family is not installed or one of its faces cannot be loaded (GPUI fails the
/// whole family then, too).
fn load_family(source: &SystemSource, name: &str, fallbacks: &[String]) -> Option<Vec<Face>> {
    let family = source.select_family_by_name(name).ok()?;
    let mut faces = Vec::new();
    for handle in family.fonts() {
        let loaded = handle.load().ok()?;
        let native = with_cascade(&loaded.native_font(), fallbacks);
        // SAFETY: a valid CTFont; font-kit only wraps it.
        let font = unsafe { Font::from_native_font(&native) };
        // GPUI leaves out a face without an `m` (it measures with one).
        if font.glyph_for_char('m').is_none() && font.full_name() != "Segoe Fluent Icons" {
            continue;
        }
        // And a face whose traits are not numbers: reading its properties
        // would panic.
        if !traits_are_numbers(&native) {
            continue;
        }
        faces.push(Face {
            properties: font.properties(),
            font: native,
        });
    }
    Some(faces)
}

/// Whether the four traits font-kit reads the properties from are there.
fn traits_are_numbers(font: &CTFont) -> bool {
    let traits = font.all_traits();
    // SAFETY: the keys are CoreText's constant strings.
    let keys = unsafe {
        [
            kCTFontSymbolicTrait,
            kCTFontWidthTrait,
            kCTFontWeightTrait,
            kCTFontSlantTrait,
        ]
    };
    keys.into_iter().all(|key| {
        traits
            .find(key)
            .is_some_and(|value| value.downcast::<CFNumber>().is_some())
    })
}

/// A copy of `font` that takes the glyphs it does not have from `fallbacks`, in
/// that order, before the system's own choice: what
/// `apply_features_and_fallbacks` makes of a `gpui::Font` (with no font features,
/// which The Writer never sets).
fn with_cascade(font: &CTFont, fallbacks: &[String]) -> CTFont {
    let features: CFArray<CFType> = CFArray::from_CFTypes(&[]);
    // SAFETY: the keys are CoreText's constant strings, wrapped without taking
    // them over.
    let (features_key, cascade_key) = unsafe {
        (
            CFString::wrap_under_get_rule(kCTFontFeatureSettingsAttribute),
            CFString::wrap_under_get_rule(kCTFontCascadeListAttribute),
        )
    };
    let mut attributes: Vec<(CFString, CFType)> = vec![(features_key, features.as_CFType())];
    if !fallbacks.is_empty() {
        let descriptors: Vec<CTFontDescriptor> = fallbacks
            .iter()
            .map(|family| {
                let name = CFString::new(family);
                // SAFETY: a valid string; the call returns a descriptor we own.
                unsafe {
                    CTFontDescriptor::wrap_under_create_rule(CTFontDescriptorCreateWithNameAndSize(
                        name.as_concrete_TypeRef(),
                        0.,
                    ))
                }
            })
            .collect();
        let cascade = CFArray::from_CFTypes(&descriptors);
        attributes.push((cascade_key, cascade.as_CFType()));
    }
    let attributes: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&attributes);
    // SAFETY: a valid dictionary and a valid font; both calls return objects we
    // own, and a null font would mean CoreText could not copy it.
    unsafe {
        let descriptor = CTFontDescriptorCreateWithAttributes(
            attributes.as_concrete_TypeRef() as CFDictionaryRef
        );
        if descriptor.is_null() {
            return font.clone();
        }
        let descriptor = CTFontDescriptor::wrap_under_create_rule(descriptor);
        let copy = CTFontCreateCopyWithAttributes(
            font.as_concrete_TypeRef(),
            0.,
            ptr::null(),
            descriptor.as_concrete_TypeRef(),
        );
        if copy.is_null() {
            font.clone()
        } else {
            CTFont::wrap_under_create_rule(copy)
        }
    }
}

/// A copy of `font` at another size (the smaller text of inline code).
pub(crate) fn at_size(font: &CTFont, size: f64) -> CTFont {
    font.clone_with_font_size(size.max(0.01))
}
