//! UI inspection and input control over `screencap`, `uiautomator dump` and
//! `input` (section 7 of the specification: screenshot, hierarchy dump,
//! element search, tap, swipe, text and key events).

use adb_engine::{classify_failure, AdbBackend, AdbError, AdbOutput};
use output::Coded;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Default dump file used when the direct `exec-out` dump fails.
const FALLBACK_DUMP_PATH: &str = "/sdcard/aca_window_dump.xml";

/// Errors produced by UI operations.
#[derive(Debug, Error)]
pub enum UiError {
    #[error("screenshot failed: {0}")]
    ScreenshotFailed(String),
    #[error("ui hierarchy dump failed: {0}")]
    DumpFailed(String),
    #[error("no UI element matches the selector")]
    NoMatch,
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Adb(#[from] AdbError),
}

impl Coded for UiError {
    fn code(&self) -> &'static str {
        match self {
            UiError::ScreenshotFailed(_) => "E_UI_SCREENSHOT_FAILED",
            UiError::DumpFailed(_) => "E_UI_DUMP_FAILED",
            UiError::NoMatch => "E_UI_NO_MATCH",
            UiError::InvalidInput(_) => "E_INVALID_ARG",
            UiError::Io(_) => "E_IO",
            UiError::Adb(e) => e.code(),
        }
    }
}

/// Screen rectangle in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }
}

/// One node of the uiautomator hierarchy (subset useful to agents).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UiNode {
    pub text: String,
    pub resource_id: String,
    pub class: String,
    pub package: String,
    pub content_desc: String,
    pub clickable: bool,
    pub enabled: bool,
    pub checked: bool,
    pub selected: bool,
    pub scrollable: bool,
    pub bounds: Rect,
    pub depth: usize,
}

/// Parsed UI hierarchy.
#[derive(Debug, Clone, Serialize)]
pub struct Hierarchy {
    pub node_count: usize,
    pub nodes: Vec<UiNode>,
}

/// Selector for [`UiEngine::find`] and selector-based taps. Every provided
/// field must match (AND semantics).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct NodeFilter {
    /// Exact text match.
    pub text: Option<String>,
    /// Substring match on `text` (mutually exclusive with `text` in the CLI).
    pub text_contains: Option<String>,
    pub resource_id: Option<String>,
    pub class: Option<String>,
    pub package: Option<String>,
    pub content_desc: Option<String>,
    pub clickable: Option<bool>,
}

impl NodeFilter {
    pub fn matches(&self, node: &UiNode) -> bool {
        if let Some(t) = &self.text {
            if node.text != *t {
                return false;
            }
        }
        if let Some(t) = &self.text_contains {
            if !node.text.contains(t.as_str()) {
                return false;
            }
        }
        if let Some(v) = &self.resource_id {
            if node.resource_id != *v {
                return false;
            }
        }
        if let Some(v) = &self.class {
            if node.class != *v {
                return false;
            }
        }
        if let Some(v) = &self.package {
            if node.package != *v {
                return false;
            }
        }
        if let Some(v) = &self.content_desc {
            if node.content_desc != *v {
                return false;
            }
        }
        if let Some(v) = self.clickable {
            if node.clickable != v {
                return false;
            }
        }
        true
    }
}

/// Where a tap should land.
#[derive(Debug, Clone, PartialEq)]
pub enum TapTarget {
    /// Absolute pixel coordinates.
    Point { x: i32, y: i32 },
    /// First node matching the selector; the tap hits its center.
    Selector(NodeFilter),
}

/// Result of a screenshot.
#[derive(Debug, Clone, Serialize)]
pub struct ScreenshotResult {
    pub path: String,
    pub bytes: u64,
}

/// Result of an input action.
#[derive(Debug, Clone, Serialize)]
pub struct InputResult {
    pub action: String,
    pub detail: String,
}

/// UI operations over any [`AdbBackend`].
pub struct UiEngine<B: AdbBackend> {
    backend: B,
}

impl<B: AdbBackend> UiEngine<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    async fn run_on(&self, serial: &str, args: &[&str]) -> Result<AdbOutput, UiError> {
        let mut full: Vec<&str> = Vec::with_capacity(args.len() + 2);
        full.push("-s");
        full.push(serial);
        full.extend_from_slice(args);
        let out = self.backend.run(&full).await?;
        if !out.success() {
            return Err(UiError::Adb(classify_failure(serial, &out)));
        }
        Ok(out)
    }

    // ------------------------------------------------------------ screenshot

    /// Capture the screen (`exec-out screencap -p`) and write a PNG file.
    /// Default destination: `screenshot-<epoch>.png` in the current directory.
    pub async fn screenshot(
        &self,
        serial: &str,
        dest: Option<&Path>,
    ) -> Result<ScreenshotResult, UiError> {
        let out = self
            .run_on(serial, &["exec-out", "screencap", "-p"])
            .await?;
        let png_magic = [0x89u8, b'P', b'N', b'G'];
        if !out.stdout.starts_with(&png_magic) {
            return Err(UiError::ScreenshotFailed(
                out.error_text().trim().to_string(),
            ));
        }
        let path: PathBuf = match dest {
            Some(p) => p.to_path_buf(),
            None => {
                let epoch = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                PathBuf::from(format!("screenshot-{epoch}.png"))
            }
        };
        std::fs::write(&path, &out.stdout)?;
        Ok(ScreenshotResult {
            path: path.display().to_string(),
            bytes: out.stdout.len() as u64,
        })
    }

    // ----------------------------------------------------------------- dump

    /// Dump the UI hierarchy: direct `exec-out uiautomator dump /dev/tty`,
    /// with a file-based fallback (`uiautomator dump` + `cat`) for devices
    /// that do not stream to stdout.
    pub async fn dump(&self, serial: &str) -> Result<Hierarchy, UiError> {
        let direct = self
            .run_on(serial, &["exec-out", "uiautomator", "dump", "/dev/tty"])
            .await;
        let xml = match direct {
            Ok(out) => extract_xml(&out.stdout_str()),
            Err(_) => None,
        };
        let xml = match xml {
            Some(x) => x,
            None => {
                let dumped = self
                    .run_on(
                        serial,
                        &["shell", "uiautomator", "dump", FALLBACK_DUMP_PATH],
                    )
                    .await;
                if let Err(e) = dumped {
                    return Err(UiError::DumpFailed(e.to_string()));
                }
                let cat = self
                    .run_on(serial, &["exec-out", "cat", FALLBACK_DUMP_PATH])
                    .await?;
                extract_xml(&cat.stdout_str()).ok_or_else(|| {
                    UiError::DumpFailed(format!(
                        "no XML hierarchy in output ({} bytes)",
                        cat.stdout.len()
                    ))
                })?
            }
        };
        parse_hierarchy(&xml)
    }

    /// Find nodes matching a selector (dumps the hierarchy first).
    pub async fn find(&self, serial: &str, filter: &NodeFilter) -> Result<Vec<UiNode>, UiError> {
        let hierarchy = self.dump(serial).await?;
        Ok(hierarchy
            .nodes
            .into_iter()
            .filter(|n| filter.matches(n))
            .collect())
    }

    // ---------------------------------------------------------------- input

    /// Tap at coordinates, or on the center of the first node matching the
    /// selector.
    pub async fn tap(&self, serial: &str, target: TapTarget) -> Result<InputResult, UiError> {
        let (x, y) = match target {
            TapTarget::Point { x, y } => (x, y),
            TapTarget::Selector(filter) => {
                let nodes = self.find(serial, &filter).await?;
                let node = nodes.first().ok_or(UiError::NoMatch)?;
                node.bounds.center()
            }
        };
        self.run_on(serial, &["input", "tap", &x.to_string(), &y.to_string()])
            .await?;
        Ok(InputResult {
            action: "tap".into(),
            detail: format!("{x},{y}"),
        })
    }

    /// Swipe from (x1,y1) to (x2,y2) over `duration_ms` (default 300ms).
    pub async fn swipe(
        &self,
        serial: &str,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        duration_ms: Option<u32>,
    ) -> Result<InputResult, UiError> {
        let duration = duration_ms.unwrap_or(300).to_string();
        self.run_on(
            serial,
            &[
                "input",
                "swipe",
                &x1.to_string(),
                &y1.to_string(),
                &x2.to_string(),
                &y2.to_string(),
                &duration,
            ],
        )
        .await?;
        Ok(InputResult {
            action: "swipe".into(),
            detail: format!("{x1},{y1} -> {x2},{y2} in {duration}ms"),
        })
    }

    /// Type text. Spaces are supported (shell-quoted); newlines are rejected —
    /// use [`UiEngine::key`] for ENTER.
    pub async fn text(&self, serial: &str, text: &str) -> Result<InputResult, UiError> {
        let quoted = quote_input_text(text)?;
        self.run_on(serial, &["input", "text", &quoted]).await?;
        Ok(InputResult {
            action: "text".into(),
            detail: text.chars().count().to_string(),
        })
    }

    /// Send a key event (`KEYCODE_BACK` or a numeric keycode).
    pub async fn key(&self, serial: &str, keycode: &str) -> Result<InputResult, UiError> {
        let valid = keycode
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            && !keycode.is_empty();
        if !valid {
            return Err(UiError::InvalidInput(format!(
                "invalid keycode '{keycode}': expected KEYCODE_* or digits"
            )));
        }
        self.run_on(serial, &["input", "keyevent", keycode]).await?;
        Ok(InputResult {
            action: "key".into(),
            detail: keycode.to_string(),
        })
    }
}

/// Quote a string for `input text` (adb joins argv with spaces, the device
/// shell then splits it): wrap in single quotes, escape embedded quotes.
/// Newlines cannot be represented and are rejected.
fn quote_input_text(text: &str) -> Result<String, UiError> {
    if text.contains('\n') || text.contains('\r') {
        return Err(UiError::InvalidInput(
            "newlines are not supported by input text; use ui key ENTER".into(),
        ));
    }
    if text.is_empty() {
        return Err(UiError::InvalidInput("empty text".into()));
    }
    Ok(format!("'{}'", text.replace('\'', "'\\''")))
}

/// Keep the XML fragment between the first `<hierarchy`/`<?xml` and the last
/// `</hierarchy>` (the dump command appends a status line).
fn extract_xml(text: &str) -> Option<String> {
    let start = text.find("<?xml").or_else(|| text.find("<hierarchy"))?;
    let end = text.rfind("</hierarchy>")? + "</hierarchy>".len();
    (end > start).then(|| text[start..end].to_string())
}

/// Parse `[x1,y1][x2,y2]` into a [`Rect`].
fn parse_bounds(raw: &str) -> Option<Rect> {
    let inner = raw.trim_start_matches('[');
    let (first, rest) = inner.split_once("][")?;
    let (x1, y1) = first.split_once(',')?;
    let (x2, y2) = rest.trim_end_matches(']').split_once(',')?;
    let x1: i32 = x1.trim().parse().ok()?;
    let y1: i32 = y1.trim().parse().ok()?;
    let x2: i32 = x2.trim().parse().ok()?;
    let y2: i32 = y2.trim().parse().ok()?;
    Some(Rect {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    })
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Parse one `<node ...>` element (start or empty tag) into a [`UiNode`].
fn node_from_event(e: &quick_xml::events::BytesStart, depth: usize) -> UiNode {
    let mut attrs: HashMap<String, String> = HashMap::new();
    for attr in e.attributes().flatten() {
        let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
        let value = xml_unescape(&String::from_utf8_lossy(attr.value.as_ref()));
        attrs.insert(key, value);
    }
    let get = |k: &str| attrs.get(k).cloned().unwrap_or_default();
    let bool_of = |k: &str| attrs.get(k).map(|v| v == "true").unwrap_or(false);
    UiNode {
        text: get("text"),
        resource_id: get("resource-id"),
        class: get("class"),
        package: get("package"),
        content_desc: get("content-desc"),
        clickable: bool_of("clickable"),
        enabled: bool_of("enabled"),
        checked: bool_of("checked"),
        selected: bool_of("selected"),
        scrollable: bool_of("scrollable"),
        bounds: parse_bounds(&get("bounds")).unwrap_or(Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }),
        depth,
    }
}

/// Parse a uiautomator dump into a flat list of nodes with depth.
pub fn parse_hierarchy(xml: &str) -> Result<Hierarchy, UiError> {
    let mut reader = Reader::from_str(xml);
    let mut nodes = Vec::new();
    let mut depth: usize = 0;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let is_node = e.name().as_ref() == b"node";
                if is_node {
                    nodes.push(node_from_event(&e, depth));
                }
                depth += 1;
            }
            // Self-closing `<node ... />` arrives as Empty, not Start.
            Ok(Event::Empty(e)) => {
                if e.name().as_ref() == b"node" {
                    nodes.push(node_from_event(&e, depth));
                }
            }
            Ok(Event::End(e)) => {
                depth = depth.saturating_sub(1);
                if e.name().as_ref() == b"hierarchy" {
                    break;
                }
            }
            Ok(Event::Eof) => break,
            Err(err) => return Err(UiError::DumpFailed(format!("xml parse error: {err}"))),
            _ => {}
        }
    }
    if nodes.is_empty() && !xml.contains("<node") {
        return Err(UiError::DumpFailed("no <node> elements in dump".into()));
    }
    Ok(Hierarchy {
        node_count: nodes.len(),
        nodes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as Map;
    use std::sync::Mutex;

    const HIERARCHY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<hierarchy rotation="0">
  <node index="0" text="" resource-id="" class="android.widget.FrameLayout" package="com.example" content-desc="" checkable="false" checked="false" clickable="false" enabled="true" scrollable="false" bounds="[0,0][1080,1920]">
    <node index="0" text="Hello &amp; welcome" resource-id="com.example:id/title" class="android.widget.TextView" package="com.example" content-desc="title" checkable="false" checked="false" clickable="true" enabled="true" scrollable="false" bounds="[100,200][500,300]" />
    <node index="1" text="OK" resource-id="" class="android.widget.Button" package="com.example" content-desc="" checkable="false" checked="false" clickable="true" enabled="true" scrollable="false" bounds="[0,1800][1080,1920]" />
  </node>
</hierarchy>"#;

    struct MockAdb {
        replies: Map<String, (i32, Vec<u8>)>,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl MockAdb {
        fn new() -> Self {
            Self {
                replies: Map::new(),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn on(mut self, key: &str, reply: &str) -> Self {
            self.replies
                .insert(key.to_string(), (0, reply.as_bytes().to_vec()));
            self
        }

        fn on_bytes(mut self, key: &str, reply: Vec<u8>) -> Self {
            self.replies.insert(key.to_string(), (0, reply));
            self
        }

        fn on_status(mut self, key: &str, status: i32, reply: &str) -> Self {
            self.replies
                .insert(key.to_string(), (status, reply.as_bytes().to_vec()));
            self
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl AdbBackend for MockAdb {
        #[allow(clippy::manual_async_fn)]
        fn run(
            &self,
            args: &[&str],
        ) -> impl std::future::Future<Output = Result<AdbOutput, AdbError>> + Send {
            async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push(args.iter().map(|s| s.to_string()).collect());
                let key = match args {
                    ["-s", _, rest @ ..] => rest.join(" "),
                    _ => args.join(" "),
                };
                let (status, reply) = self.replies.get(&key).cloned().unwrap_or((0, Vec::new()));
                Ok(AdbOutput::new(status, reply, Vec::new()))
            }
        }
    }

    #[test]
    fn parses_bounds_and_center() {
        let r = parse_bounds("[100,200][500,300]").unwrap();
        assert_eq!(
            r,
            Rect {
                x: 100,
                y: 200,
                width: 400,
                height: 100
            }
        );
        assert_eq!(r.center(), (300, 250));
        assert_eq!(parse_bounds("garbage"), None);
    }

    #[test]
    fn parses_hierarchy_with_depth_and_entities() {
        let h = parse_hierarchy(HIERARCHY).unwrap();
        assert_eq!(h.node_count, 3);
        let title = &h.nodes[1];
        assert_eq!(title.text, "Hello & welcome");
        assert_eq!(title.resource_id, "com.example:id/title");
        assert!(title.clickable);
        assert_eq!(
            title.depth, 2,
            "root=1, frame=2, title=3? depth counts starts"
        );
        assert_eq!(h.nodes[2].text, "OK");
        assert_eq!(h.nodes[2].bounds.center(), (540, 1860));
    }

    #[test]
    fn extract_xml_strips_status_line() {
        let raw = "UI hierchary dumped to: /dev/tty\n<?xml version=\"1.0\"?>\n<hierarchy><node bounds=\"[0,0][1,1]\"/></hierarchy>\nUI hierchary dumped to: /dev/tty";
        let xml = extract_xml(raw).unwrap();
        assert!(xml.starts_with("<?xml"));
        assert!(xml.ends_with("</hierarchy>"));
        assert_eq!(extract_xml("no xml here"), None);
    }

    #[test]
    fn node_filter_matching() {
        let h = parse_hierarchy(HIERARCHY).unwrap();
        let ok_btn = &h.nodes[2];
        let mut f = NodeFilter {
            text: Some("OK".into()),
            ..Default::default()
        };
        assert!(f.matches(ok_btn));
        f.text = Some("CANCEL".into());
        assert!(!f.matches(ok_btn));

        let f = NodeFilter {
            text_contains: Some("welcome".into()),
            ..Default::default()
        };
        assert!(f.matches(&h.nodes[1]));
        assert!(!f.matches(ok_btn));

        let f = NodeFilter {
            clickable: Some(true),
            ..Default::default()
        };
        assert!(f.matches(&h.nodes[1]));
        assert!(!f.matches(&h.nodes[0]));

        let f = NodeFilter {
            resource_id: Some("com.example:id/title".into()),
            ..Default::default()
        };
        assert!(f.matches(&h.nodes[1]));
    }

    #[test]
    fn input_text_quoting() {
        assert_eq!(quote_input_text("hello world").unwrap(), "'hello world'");
        assert_eq!(quote_input_text("it's").unwrap(), "'it'\\''s'");
        assert!(quote_input_text("line1\nline2").is_err());
        assert!(quote_input_text("").is_err());
    }

    #[tokio::test]
    async fn screenshot_writes_png() {
        let png = b"\x89PNG\r\n\x1a\nfake-pixel-data".to_vec();
        let adb = MockAdb::new().on_bytes("exec-out screencap -p", png.clone());
        let engine = UiEngine::new(adb);
        let dest = std::env::temp_dir().join(format!("aca-shot-{}.png", std::process::id()));
        let res = engine.screenshot("SER", Some(&dest)).await.unwrap();
        assert_eq!(res.bytes, png.len() as u64);
        assert!(dest.is_file());
        assert_eq!(std::fs::read(&dest).unwrap(), png);
        let _ = std::fs::remove_file(&dest);
    }

    #[tokio::test]
    async fn screenshot_rejects_non_png() {
        let adb = MockAdb::new().on("exec-out screencap -p", "error: device offline");
        let engine = UiEngine::new(adb);
        let dest = std::env::temp_dir().join(format!("aca-bad-{}.png", std::process::id()));
        let err = engine.screenshot("SER", Some(&dest)).await.unwrap_err();
        assert_eq!(err.code(), "E_UI_SCREENSHOT_FAILED");
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn dump_uses_direct_stream_then_falls_back() {
        // Direct stream works.
        let adb = MockAdb::new().on("exec-out uiautomator dump /dev/tty", HIERARCHY);
        let engine = UiEngine::new(adb);
        let h = engine.dump("SER").await.unwrap();
        assert_eq!(h.node_count, 3);

        // Direct stream empty → file fallback.
        let adb = MockAdb::new()
            .on_status("exec-out uiautomator dump /dev/tty", 1, "error")
            .on(
                "shell uiautomator dump /sdcard/aca_window_dump.xml",
                "Dumped file: /sdcard/aca_window_dump.xml\n",
            )
            .on("exec-out cat /sdcard/aca_window_dump.xml", HIERARCHY);
        let engine = UiEngine::new(adb);
        let h = engine.dump("SER").await.unwrap();
        assert_eq!(h.node_count, 3);
    }

    #[tokio::test]
    async fn find_returns_matching_nodes() {
        let adb = MockAdb::new().on("exec-out uiautomator dump /dev/tty", HIERARCHY);
        let engine = UiEngine::new(adb);
        let f = NodeFilter {
            clickable: Some(true),
            ..Default::default()
        };
        let found = engine.find("SER", &f).await.unwrap();
        assert_eq!(found.len(), 2);
        let f = NodeFilter {
            text: Some("NOPE".into()),
            ..Default::default()
        };
        assert!(engine.find("SER", &f).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn tap_on_selector_hits_node_center() {
        let adb = MockAdb::new()
            .on("exec-out uiautomator dump /dev/tty", HIERARCHY)
            .on("input tap 540 1860", "");
        let engine = UiEngine::new(adb);
        let f = NodeFilter {
            text: Some("OK".into()),
            ..Default::default()
        };
        let res = engine.tap("SER", TapTarget::Selector(f)).await.unwrap();
        assert_eq!(res.detail, "540,1860");
    }

    #[tokio::test]
    async fn tap_without_match_is_typed_error() {
        let adb = MockAdb::new().on("exec-out uiautomator dump /dev/tty", HIERARCHY);
        let engine = UiEngine::new(adb);
        let f = NodeFilter {
            text: Some("GHOST".into()),
            ..Default::default()
        };
        let err = engine.tap("SER", TapTarget::Selector(f)).await.unwrap_err();
        assert_eq!(err.code(), "E_UI_NO_MATCH");
    }

    #[tokio::test]
    async fn tap_at_point_and_swipe_and_key() {
        let adb = MockAdb::new()
            .on("input tap 10 20", "")
            .on("input swipe 0 100 0 900 250", "")
            .on("input keyevent KEYCODE_BACK", "");
        let engine = UiEngine::new(adb);
        let res = engine
            .tap("SER", TapTarget::Point { x: 10, y: 20 })
            .await
            .unwrap();
        assert_eq!(res.detail, "10,20");
        let res = engine
            .swipe("SER", 0, 100, 0, 900, Some(250))
            .await
            .unwrap();
        assert_eq!(res.action, "swipe");
        let res = engine.key("SER", "KEYCODE_BACK").await.unwrap();
        assert_eq!(res.detail, "KEYCODE_BACK");
        // Invalid keycodes never reach adb.
        let err = engine.key("SER", "back; rm -rf /").await.unwrap_err();
        assert_eq!(err.code(), "E_INVALID_ARG");
    }

    #[tokio::test]
    async fn text_with_spaces_is_quoted_before_adb() {
        let adb = MockAdb::new().on("input text 'hello world'", "");
        let engine = UiEngine::new(adb);
        let res = engine.text("SER", "hello world").await.unwrap();
        assert_eq!(res.action, "text");
        let calls = engine.backend.calls();
        assert!(calls
            .iter()
            .any(|c| c.last().map(String::as_str) == Some("'hello world'")));
        // Newline rejected before adb.
        let err = engine.text("SER", "a\nb").await.unwrap_err();
        assert_eq!(err.code(), "E_INVALID_ARG");
    }
}
