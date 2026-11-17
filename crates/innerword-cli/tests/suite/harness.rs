//! A place for one test: a home directory and a library in a temporary folder, and the
//! `innerword` binary run with nothing else in its environment.
//!
//! The binary works on the user's real library unless it is told otherwise, and installs
//! skills into the user's real home. No test may reach either. So every process started
//! here gets an empty environment with only `HOME` (the temporary home) and
//! `INNER_WORD_DATA_DIR` (the temporary library) in it, and [`Sandbox::new`] refuses to
//! exist when its folders are not inside the temporary directory (see `guard.rs` for the
//! tests of the guard itself).

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::Value;

/// The binary under test.
pub const WRITER: &str = env!("CARGO_BIN_EXE_innerword");

/// One run of the binary.
#[derive(Debug)]
pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    /// The error a `--json` run wrote to standard error.
    pub fn error(&self) -> Value {
        let line = self.stderr.lines().last().unwrap_or_default();
        let value: Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!("standard error is not JSON ({error}): {}", self.stderr)
        });
        value["error"].clone()
    }
}

/// A temporary home and library.
pub struct Sandbox {
    root: tempfile::TempDir,
    /// What the environment of every process has besides `HOME`, `INNER_WORD_DATA_DIR`
    /// and `PATH` ([`Sandbox::fake_page_check`], [`Sandbox::real_page_check`]).
    env: Vec<(&'static str, PathBuf)>,
}

impl Sandbox {
    pub fn new() -> Sandbox {
        let root = tempfile::Builder::new()
            .prefix("innerword-cli-test-")
            .tempdir()
            .expect("a temporary directory");
        let sandbox = Sandbox {
            root,
            env: Vec::new(),
        };
        for dir in [
            sandbox.home(),
            sandbox.library(),
            sandbox.work(),
            sandbox.tmp(),
        ] {
            std::fs::create_dir_all(&dir).expect("a folder in the temporary directory");
        }
        sandbox.assert_isolated();
        sandbox
    }

    /// The same sandbox, where pages are checked by the fake check
    /// (`INNER_WORD_PAGE_CHECK=fake`, read by a debug build only): the text of the page
    /// with its tags taken out, nothing blocked, no WebKit. Its temporary directory is
    /// the sandbox's own.
    pub fn fake_page_check(mut self) -> Sandbox {
        self.env
            .push(("INNER_WORD_PAGE_CHECK", PathBuf::from("fake")));
        self.env.push(("TMPDIR", self.tmp()));
        self
    }

    /// The same sandbox, where pages are checked for real (WebKit in the tool's child,
    /// offscreen) and the tool may fetch from 127.0.0.1 (`INNER_WORD_FETCH_LOCAL=1`, read
    /// by a debug build only), where the test's own server is. Its temporary directory,
    /// where the checks and WebKit's rule list are, is the sandbox's own.
    pub fn real_page_check(mut self) -> Sandbox {
        self.env
            .push(("INNER_WORD_FETCH_LOCAL", PathBuf::from("1")));
        self.env.push(("TMPDIR", self.tmp()));
        self
    }

    /// The temporary directory of the processes of a sandbox for pages.
    pub fn tmp(&self) -> PathBuf {
        self.root.path().join("tmp")
    }

    /// The temporary directory everything of this test is in.
    pub fn root(&self) -> PathBuf {
        self.root
            .path()
            .canonicalize()
            .expect("the temporary directory exists")
    }

    /// The home directory the binary sees.
    pub fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    /// The data directory the binary works on.
    pub fn library(&self) -> PathBuf {
        self.root.path().join("library")
    }

    /// The directory the binary is started in; relative paths of exports land here.
    pub fn work(&self) -> PathBuf {
        self.root.path().join("work")
    }

    /// Fails loudly when this sandbox could reach the user's own library or home.
    pub fn assert_isolated(&self) {
        let temp = std::env::temp_dir()
            .canonicalize()
            .expect("the system's temporary directory");
        let root = self
            .root
            .path()
            .canonicalize()
            .expect("the temporary directory exists");
        assert!(
            root.starts_with(&temp),
            "{} is not a temporary directory",
            root.display()
        );
        for real in real_places() {
            for mine in [self.home(), self.library(), self.work()] {
                assert!(
                    !mine.starts_with(&real) && !real.starts_with(&mine),
                    "the test would touch {}: {}",
                    real.display(),
                    mine.display()
                );
            }
        }
    }

    /// Makes the sandbox the whole world of a process: nothing of the environment of the
    /// test process is passed on.
    pub fn isolate(&self, command: &mut Command) {
        self.assert_isolated();
        command
            .env_clear()
            .env("HOME", self.home())
            .env("INNER_WORD_DATA_DIR", self.library())
            .env("PATH", "/usr/bin:/bin")
            .envs(self.env.iter().map(|(name, value)| (*name, value)))
            .current_dir(self.work())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    }

    /// The binary, isolated.
    pub fn command(&self) -> Command {
        let mut command = Command::new(WRITER);
        self.isolate(&mut command);
        command
    }

    fn finish(mut child: Child, input: Option<&str>) -> Run {
        if let Some(input) = input {
            let mut stdin = child.stdin.take().expect("standard input is a pipe");
            stdin
                .write_all(input.as_bytes())
                .expect("the binary reads its input");
        }
        let output = child.wait_with_output().expect("the binary ran");
        Run {
            code: output.status.code().expect("the binary exited by itself"),
            stdout: String::from_utf8(output.stdout).expect("standard output is UTF-8"),
            stderr: String::from_utf8(output.stderr).expect("standard error is UTF-8"),
        }
    }

    /// Runs the binary.
    pub fn run(&self, args: &[&str]) -> Run {
        let child = self
            .command()
            .args(args)
            .spawn()
            .expect("the binary starts");
        Sandbox::finish(child, None)
    }

    /// Runs the binary with `input` on its standard input.
    pub fn run_with_input(&self, args: &[&str], input: &str) -> Run {
        let child = self
            .command()
            .args(args)
            .stdin(Stdio::piped())
            .spawn()
            .expect("the binary starts");
        Sandbox::finish(child, Some(input))
    }

    /// Runs a command that must succeed and gives what it printed. A success prints
    /// nothing on standard error.
    #[track_caller]
    pub fn ok(&self, args: &[&str]) -> String {
        let run = self.run(args);
        assert_eq!(run.code, 0, "innerword {args:?} failed: {}", run.stderr);
        assert_eq!(run.stderr, "", "innerword {args:?} wrote to standard error");
        run.stdout
    }

    /// Runs a command with `--json` that must succeed and gives its result.
    #[track_caller]
    pub fn json(&self, args: &[&str]) -> Value {
        let mut with_json = args.to_vec();
        with_json.push("--json");
        let out = self.ok(&with_json);
        serde_json::from_str(&out).unwrap_or_else(|error| {
            panic!("innerword {args:?} --json is not JSON ({error}): {out}")
        })
    }

    /// Runs a command that must fail with this exit code and gives its message. A failure
    /// prints nothing on standard output.
    #[track_caller]
    pub fn fail(&self, args: &[&str], code: i32) -> String {
        let run = self.run(args);
        assert_eq!(
            run.code, code,
            "innerword {args:?}: {}{}",
            run.stdout, run.stderr
        );
        assert_eq!(
            run.stdout, "",
            "innerword {args:?} failed and still wrote to standard output"
        );
        assert!(
            run.stderr.starts_with("innerword: "),
            "innerword {args:?}: {}",
            run.stderr
        );
        run.stderr
    }

    /// Writes a small picture file (a PNG of `size` pixels in one colour) into the
    /// directory the binary is started in, under `name`, and says where it is.
    pub fn picture(&self, name: &str, size: (u32, u32), rgb: [u8; 3]) -> PathBuf {
        self.picture_at(&self.work().join(name), size, rgb)
    }

    /// The same at any path inside the sandbox.
    pub fn picture_at(&self, path: &Path, size: (u32, u32), rgb: [u8; 3]) -> PathBuf {
        assert!(
            inside(path.parent().expect("a file in a folder"), &self.root())
                || path.starts_with(self.root.path()),
            "a picture outside the sandbox: {}",
            path.display()
        );
        std::fs::create_dir_all(path.parent().expect("a file in a folder"))
            .expect("the folder of the picture");
        std::fs::write(path, png(size.0, size.1, rgb)).expect("the picture is written");
        path.to_path_buf()
    }

    /// Creates a document and gives its id as the commands take it (`doc:12`).
    #[track_caller]
    pub fn create(&self, title: &str, folder: Option<&str>, markdown: &str) -> String {
        let mut args = vec!["create", title, "--content", markdown];
        if let Some(folder) = folder {
            args.extend(["--in", folder, "--create-folders"]);
        }
        let created = self.json(&args);
        format!(
            "doc:{}",
            created["document"]["id"]
                .as_i64()
                .expect("the new document has an id")
        )
    }

    /// Holds the lock of the library the way an app does that takes no changes from tools
    /// (every version before 0.6.0: it listens nowhere), until the returned value is
    /// dropped. The app has been running for a while: the tools give one that has only
    /// just taken the lock a few seconds to say where it listens.
    pub fn open_an_app_that_takes_no_changes(
        &self,
    ) -> innerword_store::instance_lock::InstanceLock {
        use innerword_store::instance_lock::{Acquired, HolderKind, LOCK_FILE_NAME, acquire_as};
        let lock = match acquire_as(&self.library(), HolderKind::App)
            .expect("the lock file can be made")
        {
            Acquired::Lock(lock) => lock,
            Acquired::Taken(copy) => panic!("the library of the test is locked already: {copy:?}"),
        };
        let an_hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(self.library().join(LOCK_FILE_NAME))
            .and_then(|file| file.set_modified(an_hour_ago))
            .expect("the lock file can be dated");
        lock
    }

    /// Opens the library the way the running app does, until the returned value is
    /// dropped: a thread of the test holds the lock, listens where the tools look for
    /// the app, and carries out what they ask.
    pub fn open_the_app(&self) -> LiveApp {
        LiveApp::open(&self.library())
    }

    /// Starts `innerword mcp` and gives a client that talks to it over its pipes.
    pub fn mcp(&self) -> McpClient {
        let mut child = self
            .command()
            .arg("mcp")
            .stdin(Stdio::piped())
            .spawn()
            .expect("the server starts");
        let stdin = child.stdin.take().expect("standard input is a pipe");
        let stdout = BufReader::new(child.stdout.take().expect("standard output is a pipe"));
        McpClient {
            child,
            stdin: Some(stdin),
            stdout,
            next_id: 1,
        }
    }
}

/// The bytes of a PNG file: `width` × `height` pixels in one colour, 8 bits a channel,
/// its pixel data stored without compression. Written out here so that the tests of the
/// binary need nothing but the binary to have a real picture file (the tool reads it
/// with ImageIO, as it reads a user's).
pub fn png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    fn chunk(kind: &[u8; 4], data: &[u8], out: &mut Vec<u8>) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend(data);
        out.extend(&body);
        out.extend(crc32(&body).to_be_bytes());
    }
    // Every row: the filter "none", then its pixels.
    let mut row = vec![0u8];
    for _ in 0..width {
        row.extend(rgb);
    }
    let rows = row.repeat(height as usize);
    // A zlib stream of stored blocks, and the checksum of what they hold.
    let mut packed = vec![0x78, 0x01];
    let mut blocks = rows.chunks(65_535).peekable();
    while let Some(block) = blocks.next() {
        packed.push(u8::from(blocks.peek().is_none()));
        packed.extend((block.len() as u16).to_le_bytes());
        packed.extend((!(block.len() as u16)).to_le_bytes());
        packed.extend(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in &rows {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    packed.extend(((b << 16) | a).to_be_bytes());

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    // 8 bits a channel, colour type 2 (RGB), and the three methods there is one of each.
    header.extend([8, 2, 0, 0, 0]);
    chunk(b"IHDR", &header, &mut out);
    chunk(b"IDAT", &packed, &mut out);
    chunk(b"IEND", &[], &mut out);
    out
}

/// What stands in for the running app: it holds the lock of the library, listens on the
/// socket the lock file names, and carries every request out on a connection of its own,
/// one after the other, with the code the app runs on its main thread
/// (`innerword_agent::live::execute`). What the real app adds (saving the open document
/// first, its window, the note to the writer) is tested with the app.
pub struct LiveApp {
    closer: innerword_agent::live::Closer,
    thread: Option<std::thread::JoinHandle<()>>,
    served: std::sync::mpsc::Receiver<Served>,
    database: PathBuf,
    _lock: innerword_store::instance_lock::InstanceLock,
}

/// One request the stand-in for the app carried out.
#[derive(Debug)]
pub struct Served {
    /// The name of the operation (`"edit"`, `"flush"`).
    pub name: &'static str,
    /// Who asked (`--author`).
    pub author: Option<String>,
    /// How many things it changed in the library.
    pub changes: usize,
    /// Whether the answer was the result of the operation (and not a failure or a
    /// refusal).
    pub done: bool,
}

impl LiveApp {
    fn open(data_dir: &Path) -> LiveApp {
        use innerword_agent::live::{self, Listener};
        let lock = innerword_store::instance_lock::acquire(data_dir)
            .expect("the lock file can be made")
            .lock()
            .expect("nobody else has the library of the test");
        let database = data_dir.join(innerword_store::DATABASE_FILE_NAME);
        let database_for_thread = database.clone();
        let mut store = innerword_store::Store::open(&database).expect("the library opens");
        let listener = Listener::bind(data_dir).expect("the app listens");
        let closer = listener.closer();
        let (tell, served) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            while let Some(mut incoming) = listener.accept() {
                if !incoming.begin() {
                    continue;
                }
                // The app's listener stores the files of a page before the request is
                // queued, and the main thread only writes the row.
                if live::carries_page_files(incoming.request()) {
                    let mut files = innerword_store::ImageFile::beside(&database_for_thread);
                    if let Err(error) = live::store_page_files(incoming.request_mut(), &mut files) {
                        incoming.answer(live::Answer::failed(&error)).ok();
                        continue;
                    }
                }
                let request = incoming.request();
                let executed = live::execute(&mut store, request);
                tell.send(Served {
                    name: request.name(),
                    author: request.author().map(str::to_string),
                    changes: executed.changes.len(),
                    done: executed.answer.is_done(),
                })
                .ok();
                incoming.answer(executed.answer).ok();
            }
        });
        LiveApp {
            closer,
            thread: Some(thread),
            served,
            database,
            _lock: lock,
        }
    }

    /// Everything served since the last look.
    pub fn served(&self) -> Vec<Served> {
        self.served.try_iter().collect()
    }

    /// The names of the changes served since the last look (without the requests to
    /// save that every read sends).
    pub fn changes(&self) -> Vec<&'static str> {
        self.served()
            .into_iter()
            .map(|served| served.name)
            .filter(|name| *name != "flush")
            .collect()
    }

    /// Presses the owner's switch "Agents can make changes while Inner Word is open".
    pub fn set_switch(&self, on: bool) {
        let mut store = innerword_store::Store::open(&self.database).expect("the library opens");
        store
            .set_setting(innerword_agent::live::CHANGES_SETTING, &on)
            .expect("the setting is written");
    }
}

impl Drop for LiveApp {
    fn drop(&mut self) {
        self.closer.close();
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

/// Where the user's own things are, as far as the test process can tell: the real home,
/// the real library, the directories agents read.
pub fn real_places() -> Vec<PathBuf> {
    let mut places = Vec::new();
    if let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) {
        let home = PathBuf::from(home);
        places.push(home.join("Library/Application Support/Inner Word"));
        // The folder of the library while the app was called The Writer (0.9.0 and
        // before), which the first default look moves to "Inner Word".
        places.push(home.join("Library/Application Support/The Writer"));
        for dir in [
            ".claude",
            ".codex",
            ".gemini",
            ".config/opencode",
            ".agents",
            ".copilot",
            ".cursor",
        ] {
            places.push(home.join(dir));
        }
        places.push(home.join("Library/Application Support/Claude"));
    }
    if let Some(dir) = std::env::var_os("INNER_WORD_DATA_DIR").filter(|dir| !dir.is_empty()) {
        places.push(PathBuf::from(dir));
    }
    places
}

/// Whether `path` is inside `root`.
pub fn inside(path: &Path, root: &Path) -> bool {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    path.starts_with(root)
}

/// A client of `innerword mcp`: one JSON-RPC message per line over real pipes.
pub struct McpClient {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl McpClient {
    /// Writes one line to the server.
    pub fn send_line(&mut self, line: &str) {
        let stdin = self
            .stdin
            .as_mut()
            .expect("the input of the server is open");
        stdin.write_all(line.as_bytes()).expect("the server reads");
        stdin.write_all(b"\n").expect("the server reads");
        stdin.flush().expect("the server reads");
    }

    /// Reads one line from the server: a single JSON message.
    pub fn read(&mut self) -> Value {
        let mut line = String::new();
        let read = self
            .stdout
            .read_line(&mut line)
            .expect("the server answers");
        assert!(
            read > 0,
            "the server closed its output instead of answering"
        );
        assert!(
            line.ends_with('\n'),
            "a message ends with a line break: {line:?}"
        );
        serde_json::from_str(&line).unwrap_or_else(|error| {
            panic!("the server wrote something that is not JSON ({error}): {line}")
        })
    }

    /// Sends a notification; there is no answer.
    pub fn notify(&mut self, method: &str, params: Value) {
        let message = serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.send_line(&message.to_string());
    }

    /// Sends a request and gives the whole response.
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let message =
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.send_line(&message.to_string());
        let response = self.read();
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(
            response["id"], id,
            "the response answers the request: {response}"
        );
        response
    }

    /// The handshake of the legacy revisions.
    pub fn initialize(&mut self, version: &str, client: &str) -> Value {
        let response = self.request(
            "initialize",
            serde_json::json!({
                "protocolVersion": version,
                "capabilities": {},
                "clientInfo": { "name": client, "version": "1.0" },
            }),
        );
        self.notify("notifications/initialized", serde_json::json!({}));
        response["result"].clone()
    }

    /// Calls a tool and gives the result object (`content`, `structuredContent`, `isError`).
    #[track_caller]
    pub fn call(&mut self, name: &str, arguments: Value) -> Value {
        let response = self.request(
            "tools/call",
            serde_json::json!({ "name": name, "arguments": arguments }),
        );
        assert!(
            response.get("error").is_none(),
            "tools/call {name} is a protocol error: {response}"
        );
        let result = response["result"].clone();
        assert_eq!(result["content"][0]["type"], "text", "{result}");
        assert!(result["content"][0]["text"].is_string(), "{result}");
        assert!(result["structuredContent"].is_object(), "{result}");
        assert!(result["isError"].is_boolean(), "{result}");
        result
    }

    /// Calls a tool that must succeed; gives its text and its data.
    #[track_caller]
    pub fn call_ok(&mut self, name: &str, arguments: Value) -> (String, Value) {
        let result = self.call(name, arguments);
        assert_eq!(
            result["isError"], false,
            "{name} failed: {}",
            result["content"][0]["text"]
        );
        (
            result["content"][0]["text"].as_str().unwrap().to_string(),
            result["structuredContent"].clone(),
        )
    }

    /// Calls a tool that must fail; gives its message and the error as data.
    #[track_caller]
    pub fn call_error(&mut self, name: &str, arguments: Value) -> (String, Value) {
        let result = self.call(name, arguments);
        assert_eq!(
            result["isError"], true,
            "{name} did not fail: {}",
            result["content"][0]["text"]
        );
        (
            result["content"][0]["text"].as_str().unwrap().to_string(),
            result["structuredContent"]["error"].clone(),
        )
    }

    /// Closes the input of the server and waits for it: it must exit by itself, with
    /// success, having written nothing to standard output but its answers and nothing to
    /// standard error.
    pub fn close(mut self) {
        drop(self.stdin.take());
        let mut rest = String::new();
        std::io::Read::read_to_string(&mut self.stdout, &mut rest)
            .expect("the output of the server ends");
        assert_eq!(rest, "", "the server wrote more than its answers");
        let status = self.child.wait().expect("the server exits");
        assert!(status.success(), "the server exited with {status}");
        let mut stderr = String::new();
        std::io::Read::read_to_string(
            self.child
                .stderr
                .as_mut()
                .expect("standard error is a pipe"),
            &mut stderr,
        )
        .expect("standard error ends");
        assert_eq!(stderr, "", "the server wrote to standard error");
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        drop(self.stdin.take());
        let _ = self.child.wait();
    }
}
