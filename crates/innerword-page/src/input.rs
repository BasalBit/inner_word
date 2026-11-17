//! What a page is made from: one `.html` file and the local files it names, or one
//! folder taken whole (docs/pages-design.md, section 3, "Inputs").

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::error::{PageError, Problem, count, megabytes};
use crate::limits::{MAX_FILE_BYTES, MAX_FILES, MAX_PAGE_BYTES};
use crate::media::{Kind, OCTET_STREAM, hex, media_type, sha256};
use crate::resolve::{Resolved, local_base, resolve};
use crate::scan::scan_file;

/// What a page is made from.
#[derive(Clone)]
pub struct Input {
    /// The file name without `.html`, or the folder's name: the last fallback of the title.
    pub name: String,
    /// The folder the page's paths are relative to (a single file's folder).
    pub root: PathBuf,
    /// The path of the entry, relative to the root.
    pub entry: String,
    /// Every file of the page, sorted by path.
    pub files: Vec<InputFile>,
    /// The SHA-256 of the input in hex (a single file is hashed as index.html).
    pub fingerprint: String,
    pub single_file: bool,
    /// Single file: literal paths beside it that were not taken. Folder: links left out.
    pub warnings: Vec<String>,
}

/// One file of the input, at its path relative to the root (`/` between folders).
#[derive(Clone)]
pub struct InputFile {
    pub path: String,
    pub bytes: Arc<[u8]>,
}

impl fmt::Debug for InputFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "InputFile {{ path: {:?}, bytes: {} }}",
            self.path,
            self.bytes.len()
        )
    }
}

impl fmt::Debug for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Input")
            .field("name", &self.name)
            .field("root", &self.root)
            .field("entry", &self.entry)
            .field("files", &self.files)
            .field("fingerprint", &self.fingerprint)
            .field("single_file", &self.single_file)
            .field("warnings", &self.warnings)
            .finish()
    }
}

const ADDRESS_AS_INPUT: &str = "a page is added from a file or a folder on this Mac; fetching a live site is not supported. Save the page first (the skill says how)";

/// Directories macOS shows as a file: never walked into.
const PACKAGES: &[&str] = &[
    "app",
    "appex",
    "bundle",
    "framework",
    "plugin",
    "kext",
    "xpc",
    "saver",
    "prefpane",
    "qlgenerator",
    "mdimporter",
    "component",
    "photoslibrary",
    "musiclibrary",
    "tvlibrary",
    "imovielibrary",
    "photolibrary",
    "aplibrary",
    "fcpbundle",
    "logicx",
    "band",
    "rtfd",
    "pages",
    "numbers",
    "key",
    "xcodeproj",
    "xcworkspace",
    "playground",
    "sparsebundle",
    "pkg",
    "mpkg",
    "dsym",
    "docarchive",
    "scptd",
    "wdgt",
    "lpdf",
    "nib",
    "xcassets",
    "sketch",
];

/// A file: that file and what its references name, never a walk of its folder.
/// A folder: every file under it (no dot names, no packages), at most MAX_FILES /
/// MAX_PAGE_BYTES.
pub fn read_input(path: &Path, entry: Option<&str>) -> Result<Input, PageError> {
    let shown = path.to_string_lossy();
    let lower = shown.trim().to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.") {
        return Err(PageError::input(ADDRESS_AS_INPUT));
    }
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(PageError::input(format!(
                "there is no file or folder at {shown}"
            )));
        }
        Err(error) => return Err(PageError::io(path, error)),
    };
    if meta.is_dir() {
        read_folder(path, entry)
    } else if meta.is_file() {
        read_single(path, entry)
    } else {
        Err(PageError::input(format!(
            "{shown} is neither a file nor a folder"
        )))
    }
}

fn is_html_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".html") || lower.ends_with(".htm")
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn stem(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".html") {
        name[..name.len() - 5].to_string()
    } else if lower.ends_with(".htm") {
        name[..name.len() - 4].to_string()
    } else {
        name.to_string()
    }
}

/// "is 40.2 MB; a file can be 32 MB at most"
pub(crate) fn too_large(what: &str, bytes: u64) -> Problem {
    let size = megabytes(bytes);
    let size = if size == megabytes(MAX_FILE_BYTES) {
        format!("over {size}")
    } else {
        size
    };
    Problem::new(
        what,
        format!(
            "{what} is {size}; a file can be {} at most",
            megabytes(MAX_FILE_BYTES)
        ),
    )
}

pub(crate) fn too_heavy(bytes: u64) -> Problem {
    let size = megabytes(bytes);
    let size = if size == megabytes(MAX_PAGE_BYTES) {
        format!("over {size}")
    } else {
        size
    };
    Problem::new(
        "the page",
        format!(
            "the page weighs {size}; a page can be {} at most",
            megabytes(MAX_PAGE_BYTES)
        ),
    )
}

pub(crate) fn too_many_files(files: usize) -> Problem {
    Problem::new(
        "the page",
        format!(
            "the page has {} files; a page can have {} at most",
            count(files),
            count(MAX_FILES)
        ),
    )
}

fn read_single(path: &Path, entry: Option<&str>) -> Result<Input, PageError> {
    let entry_name = file_name(path);
    if !is_html_name(&entry_name) {
        return Err(PageError::input(format!(
            "{entry_name} is not an HTML file: a page is added from an .html file or a folder"
        )));
    }
    if let Some(given) = entry {
        let given = normal_entry(given);
        if given != entry_name {
            return Err(PageError::input(format!(
                "an entry is chosen only in a folder; {entry_name} is a single file"
            )));
        }
    }
    let canonical = fs::canonicalize(path).map_err(|error| PageError::io(path, error))?;
    let root = canonical
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    let name = stem(&entry_name);

    let mut taken: BTreeMap<String, Arc<[u8]>> = BTreeMap::new();
    let mut total: u64 = 0;
    let mut queue: Vec<String> = Vec::new();
    let mut problems: Vec<Problem> = Vec::new();
    // Files beside it that it names and that are not taken: said in a warning.
    let mut beside: BTreeSet<String> = BTreeSet::new();
    // Hidden files it names (a segment of the path begins with a dot): never taken,
    // as the walk of a folder never takes them (`.ssh/id_ed25519`, `.env`).
    let mut hidden: BTreeSet<String> = BTreeSet::new();
    take(&root, &entry_name, &mut taken, &mut total, &mut problems)?;
    queue.push(entry_name.clone());

    while let Some(current) = queue.pop() {
        let kind = Kind::of_path(&current);
        if !kind.is_text() {
            continue;
        }
        let bytes = taken[&current].clone();
        let scan = scan_file(kind, &bytes, &local_base(&current));
        for reference in scan.all_refs() {
            match resolve(&reference.raw, &reference.base) {
                Resolved::Local { path: local, .. } => {
                    let wanted = if local.is_empty() || local.ends_with('/') {
                        format!("{local}index.html")
                    } else {
                        local
                    };
                    if taken.contains_key(&wanted) || !is_file_inside(&root, &wanted) {
                        continue;
                    }
                    if is_hidden(&wanted) {
                        hidden.insert(wanted);
                        continue;
                    }
                    // A link the reader follows takes another page of it, never a
                    // document it only links to (`<a href="tax-2025.pdf">`).
                    if reference.navigation && Kind::of_path(&wanted) != Kind::Html {
                        beside.insert(wanted);
                        continue;
                    }
                    if take(&root, &wanted, &mut taken, &mut total, &mut problems)? {
                        queue.push(wanted);
                    }
                }
                Resolved::Outside => {
                    return Err(PageError::input(format!(
                        "{current} names {}, outside its folder: add the folder that holds both and say which page opens with --entry.",
                        reference.raw.trim()
                    )));
                }
                _ => {}
            }
        }
        if !problems.is_empty() {
            break;
        }
    }
    if !problems.is_empty() {
        return Err(PageError::Incomplete { name, problems });
    }

    // Literal paths of the page's files that name a file beside it: said, not taken.
    for (path, bytes) in &taken {
        let kind = Kind::of_path(path);
        if !kind.is_text() {
            continue;
        }
        let scan = scan_file(kind, bytes, &local_base(path));
        for literal in scan.all_literals() {
            if literal.substituted || !looks_like_path(&literal.value) {
                continue;
            }
            if let Resolved::Local { path: local, .. } =
                resolve(literal.value.trim(), &literal.base)
            {
                if !local.is_empty() && !taken.contains_key(&local) && is_file_inside(&root, &local)
                {
                    if is_hidden(&local) {
                        hidden.insert(local);
                    } else {
                        beside.insert(local);
                    }
                }
            }
        }
    }
    let mut warnings = Vec::new();
    if let Some(first) = beside.iter().next() {
        let more = beside.len() - 1;
        warnings.push(if more == 0 {
            format!(
                "{entry_name} names {first} beside it, which it may load later; it was not taken. To keep it, add the folder."
            )
        } else {
            format!(
                "{entry_name} names {first} and {} more {} beside it that it may load later; they were not taken. To keep them, add the folder.",
                count(more),
                if more == 1 { "file" } else { "files" }
            )
        });
    }
    if let Some(first) = hidden.iter().next() {
        let more = hidden.len() - 1;
        warnings.push(if more == 0 {
            format!(
                "{entry_name} names {first}, a hidden file beside it; it was not taken, as a file whose name begins with a dot never is."
            )
        } else {
            format!(
                "{entry_name} names {first} and {} more hidden {} beside it; they were not taken, as a file whose name begins with a dot never is.",
                count(more),
                if more == 1 { "file" } else { "files" }
            )
        });
    }

    let files: Vec<InputFile> = taken
        .into_iter()
        .map(|(path, bytes)| InputFile { path, bytes })
        .collect();
    let fingerprint = fingerprint_of(&files, Some(&entry_name), None);
    Ok(Input {
        name,
        root,
        entry: entry_name,
        files,
        fingerprint,
        single_file: true,
        warnings,
    })
}

/// A string of a page that may be the path of a file it loads later: no scheme, no white
/// space at its ends, the ending of a file a page loads.
fn looks_like_path(text: &str) -> bool {
    if text.is_empty() || text.len() > 300 || text.trim() != text {
        return false;
    }
    if text.contains("://")
        || text.starts_with('#')
        || text.starts_with('?')
        || text.starts_with("data:")
    {
        return false;
    }
    if text.chars().any(|c| {
        matches!(
            c,
            '<' | '>' | '"' | '\n' | '\r' | '\t' | '*' | '|' | '{' | '}' | '\\'
        )
    }) {
        return false;
    }
    if text.contains(':') && !text.contains('/') {
        return false;
    }
    let path = text.split(['?', '#']).next().unwrap_or("");
    let name = path.rsplit('/').next().unwrap_or("");
    name.contains('.') && media_type(name) != OCTET_STREAM
}

/// Whether a path of the page has a segment that begins with a dot (`.ssh/id_ed25519`,
/// `img/.hidden.png`): such a file is never taken, from a folder or beside a single file.
fn is_hidden(path: &str) -> bool {
    path.split('/').any(|segment| segment.starts_with('.'))
}

/// Whether the page's path names a plain file inside the root (no link leads out of it).
fn is_file_inside(root: &Path, path: &str) -> bool {
    if path
        .split('/')
        .any(|segment| segment == ".." || segment.is_empty())
    {
        return false;
    }
    let full = root.join(path);
    let Ok(meta) = fs::symlink_metadata(&full) else {
        return false;
    };
    if !meta.file_type().is_file() {
        return false;
    }
    match fs::canonicalize(&full) {
        Ok(canonical) => canonical.starts_with(root),
        Err(_) => false,
    }
}

/// Reads one file of a single-file input within the limits. Ok(false) when a limit was
/// reached (the problem is noted).
fn take(
    root: &Path,
    path: &str,
    taken: &mut BTreeMap<String, Arc<[u8]>>,
    total: &mut u64,
    problems: &mut Vec<Problem>,
) -> Result<bool, PageError> {
    let full = root.join(path);
    let meta = fs::metadata(&full).map_err(|error| PageError::io(&full, error))?;
    if meta.len() > MAX_FILE_BYTES {
        problems.push(too_large(path, meta.len()));
        return Ok(false);
    }
    if taken.len() + 1 > MAX_FILES {
        problems.push(too_many_files(taken.len() + 1));
        return Ok(false);
    }
    if *total + meta.len() > MAX_PAGE_BYTES {
        problems.push(too_heavy(*total + meta.len()));
        return Ok(false);
    }
    let bytes = fs::read(&full).map_err(|error| PageError::io(&full, error))?;
    *total += bytes.len() as u64;
    taken.insert(path.to_string(), Arc::from(bytes));
    Ok(true)
}

fn normal_entry(entry: &str) -> String {
    let mut entry = entry.trim().replace('\\', "/");
    while let Some(rest) = entry.strip_prefix("./") {
        entry = rest.to_string();
    }
    entry.trim_start_matches('/').to_string()
}

fn is_package(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((stem, ending)) if !stem.is_empty() => {
            PACKAGES.contains(&ending.to_ascii_lowercase().as_str())
        }
        _ => false,
    }
}

fn skipped_name(name: &str) -> bool {
    name.starts_with('.')
        || name == "__MACOSX"
        || name.eq_ignore_ascii_case("Thumbs.db")
        || name == "Icon\r"
}

fn read_folder(path: &Path, entry: Option<&str>) -> Result<Input, PageError> {
    let root = fs::canonicalize(path).map_err(|error| PageError::io(path, error))?;
    let name = {
        let n = file_name(&root);
        if n.is_empty() { "page".to_string() } else { n }
    };

    // The walk: paths and sizes first, so that nothing is read past a limit.
    let mut found: Vec<(String, u64)> = Vec::new();
    let mut links = 0usize;
    let mut total: u64 = 0;
    let mut stack: Vec<(PathBuf, String)> = vec![(root.clone(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let mut entries: Vec<(String, fs::FileType, PathBuf)> = Vec::new();
        for item in fs::read_dir(&dir).map_err(|error| PageError::io(&dir, error))? {
            let item = item.map_err(|error| PageError::io(&dir, error))?;
            let Some(item_name) = item.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let file_type = item
                .file_type()
                .map_err(|error| PageError::io(item.path(), error))?;
            entries.push((item_name, file_type, item.path()));
        }
        entries.sort_by(|a, b| b.0.cmp(&a.0));
        for (item_name, file_type, full) in entries {
            if skipped_name(&item_name) {
                continue;
            }
            let relative = if prefix.is_empty() {
                item_name.clone()
            } else {
                format!("{prefix}/{item_name}")
            };
            if file_type.is_symlink() {
                links += 1;
                continue;
            }
            if file_type.is_dir() {
                if !is_package(&item_name) {
                    stack.push((full, relative));
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let size = fs::metadata(&full)
                .map_err(|error| PageError::io(&full, error))?
                .len();
            if size > MAX_FILE_BYTES {
                return Err(PageError::Incomplete {
                    name,
                    problems: vec![too_large(&relative, size)],
                });
            }
            found.push((relative, size));
            total += size;
            if found.len() > MAX_FILES {
                return Err(PageError::Incomplete {
                    name,
                    problems: vec![Problem::new(
                        "the folder",
                        format!(
                            "the folder has more than {} files; a page can have {} at most",
                            count(MAX_FILES),
                            count(MAX_FILES)
                        ),
                    )],
                });
            }
            if total > MAX_PAGE_BYTES {
                return Err(PageError::Incomplete {
                    name,
                    problems: vec![Problem::new(
                        "the folder",
                        format!(
                            "the folder holds more than {}; a page can be {} at most",
                            megabytes(MAX_PAGE_BYTES),
                            megabytes(MAX_PAGE_BYTES)
                        ),
                    )],
                });
            }
        }
    }
    found.sort();

    let entry = choose_entry(&found, entry, &name)?;
    let mut files = Vec::with_capacity(found.len());
    for (relative, _) in &found {
        let full = root.join(relative);
        let bytes = fs::read(&full).map_err(|error| PageError::io(&full, error))?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(PageError::Incomplete {
                name,
                problems: vec![too_large(relative, bytes.len() as u64)],
            });
        }
        files.push(InputFile {
            path: relative.clone(),
            bytes: Arc::from(bytes),
        });
    }
    let mut warnings = Vec::new();
    if links > 0 {
        warnings.push(if links == 1 {
            "the folder has a link to another file or folder, which was left out".to_string()
        } else {
            format!(
                "the folder has {} links to other files or folders, which were left out",
                count(links)
            )
        });
    }
    let fingerprint = fingerprint_of(&files, None, Some(&entry));
    Ok(Input {
        name,
        root,
        entry,
        files,
        fingerprint,
        single_file: false,
        warnings,
    })
}

fn choose_entry(
    found: &[(String, u64)],
    entry: Option<&str>,
    _name: &str,
) -> Result<String, PageError> {
    if let Some(given) = entry {
        let given = normal_entry(given);
        if given.is_empty() {
            return Err(PageError::input(
                "the entry is empty: name the HTML file the page opens with",
            ));
        }
        let exact = found.iter().find(|(path, _)| *path == given);
        let any_case = found
            .iter()
            .find(|(path, _)| path.eq_ignore_ascii_case(&given));
        let Some((path, _)) = exact.or(any_case) else {
            return Err(PageError::input(format!("the folder has no file {given}")));
        };
        if !is_html_name(path) {
            return Err(PageError::input(format!(
                "{path} is not an HTML file: the entry is the HTML file the page opens with"
            )));
        }
        return Ok(path.clone());
    }
    let top: Vec<&String> = found
        .iter()
        .map(|(path, _)| path)
        .filter(|path| !path.contains('/'))
        .collect();
    for wanted in ["index.html", "index.htm"] {
        if let Some(path) = top.iter().find(|path| path.as_str() == wanted) {
            return Ok((*path).clone());
        }
        if let Some(path) = top.iter().find(|path| path.eq_ignore_ascii_case(wanted)) {
            return Ok((*path).clone());
        }
    }
    let pages: Vec<&&String> = top.iter().filter(|path| is_html_name(path)).collect();
    match pages.len() {
        1 => Ok((*pages[0]).clone()),
        0 => {
            if found.iter().any(|(path, _)| is_html_name(path)) {
                Err(PageError::input(
                    "the folder has no HTML file at its top and no index.html: say which one opens with --entry",
                ))
            } else {
                Err(PageError::input(
                    "the folder has no HTML file, so it is no page",
                ))
            }
        }
        n => Err(PageError::input(format!(
            "the folder has {n} pages at its top and no index.html: say which one opens with --entry"
        ))),
    }
}

/// The SHA-256, in hex, of: for each file in path order, its path, a NUL byte, the
/// SHA-256 of its bytes. `single_entry`: the entry of a single-file input, hashed under
/// the path `index.html` whatever its name. `folder_entry`: the entry of a folder,
/// hashed after the files (two NUL bytes, `entry`, a NUL byte, its path) when it is not
/// `index.html`, so that the same folder opening with another page is another input
/// (and a folder that opens with its index.html keeps the fingerprint it had).
pub(crate) fn fingerprint_of(
    files: &[InputFile],
    single_entry: Option<&str>,
    folder_entry: Option<&str>,
) -> String {
    let mut named: Vec<(String, [u8; 32])> = files
        .iter()
        .map(|file| {
            let path = match single_entry {
                Some(entry) if entry == file.path => "index.html".to_string(),
                _ => file.path.clone(),
            };
            (path, sha256(&file.bytes))
        })
        .collect();
    named.sort();
    let mut hasher = Sha256::new();
    for (path, digest) in &named {
        hasher.update(path.as_bytes());
        hasher.update([0u8]);
        hasher.update(digest);
    }
    if let Some(entry) = folder_entry.filter(|entry| *entry != "index.html") {
        hasher.update(b"\0\0entry\0");
        hasher.update(entry.as_bytes());
    }
    hex(&hasher.finalize())
}
