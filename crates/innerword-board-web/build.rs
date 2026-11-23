//! `scripts/bundle.sh` builds the released binaries with `INNER_WORD_BUNDLED` set: they then
//! hold no path of the build machine and look for the whiteboard's files only in their own
//! app bundle (`Assets::find`, docs/whiteboard-design.md section 3).
fn main() {
    println!("cargo::rerun-if-env-changed=INNER_WORD_BUNDLED");
    println!("cargo::rustc-check-cfg=cfg(innerword_bundled)");
    if std::env::var_os("INNER_WORD_BUNDLED").is_some() {
        println!("cargo::rustc-cfg=innerword_bundled");
    }
}
