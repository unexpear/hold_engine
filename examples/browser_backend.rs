//! Small same-origin backend for the browser-side Morphic prototype.
//!
//! Usage:
//!   cargo run --example browser_backend
//!   cargo run --example browser_backend -- --scene examples/demo.hsc --port 8787
//!
//! What it does:
//! - serves `morphic-web-prototype/` over HTTP
//! - loads a source `.hsc` scene through the Rust editor bridge
//! - exposes minimal endpoints for scene load/apply/save/reload
//! - exposes raw shared editor-protocol command execution for future clients

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};

use hold::{
    EditorBridgeServer, EditorProtocolError, EditorRuntimeHost, EditorSceneSession,
    EditorSessionError, Project, SceneBridgeError, SceneDocument, SceneError, Texture,
    TextureFormat, load_tga,
};
use hold::editor::{
    EditorBrowserEntryKind, EditorBrowserError, EditorBrowserModel, EditorBrowserRootKind,
};

const DEFAULT_SCENE: &str = "examples/demo.hsc";
const DEFAULT_WEB_ROOT: &str = "morphic-web-prototype";
const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 8787;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse()?;
    let bind_addr = format!("{}:{}", args.host, args.port);
    let listener = TcpListener::bind(&bind_addr)?;
    let mut app = BackendApp::open(
        args.scene_path.clone(),
        args.web_root.clone(),
        args.assets_root.clone(),
    )?;

    eprintln!(
        "[browser-backend] serving '{}' at http://{}",
        args.scene_path.display(),
        bind_addr
    );

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(err) = handle_client(stream, &mut app) {
                    eprintln!("[browser-backend] {err}");
                }
            }
            Err(err) => eprintln!("[browser-backend] accept error: {err}"),
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct Args {
    host: String,
    port: u16,
    scene_path: PathBuf,
    web_root: PathBuf,
    assets_root: Option<PathBuf>,
}

impl Args {
    fn parse() -> Result<Self, BackendError> {
        let mut host = DEFAULT_HOST.to_string();
        let mut port = DEFAULT_PORT;
        let mut scene_path = PathBuf::from(DEFAULT_SCENE);
        let mut web_root = PathBuf::from(DEFAULT_WEB_ROOT);
        let mut assets_root = None;

        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--host" => {
                    host = args.next().ok_or_else(|| {
                        BackendError::BadRequest("missing value for --host".into())
                    })?;
                }
                "--port" => {
                    let value = args.next().ok_or_else(|| {
                        BackendError::BadRequest("missing value for --port".into())
                    })?;
                    port = value.parse().map_err(|_| {
                        BackendError::BadRequest(format!("invalid --port value '{value}'"))
                    })?;
                }
                "--scene" => {
                    let value = args.next().ok_or_else(|| {
                        BackendError::BadRequest("missing value for --scene".into())
                    })?;
                    scene_path = PathBuf::from(value);
                }
                "--web-root" => {
                    let value = args.next().ok_or_else(|| {
                        BackendError::BadRequest("missing value for --web-root".into())
                    })?;
                    web_root = PathBuf::from(value);
                }
                "--assets-root" => {
                    let value = args.next().ok_or_else(|| {
                        BackendError::BadRequest("missing value for --assets-root".into())
                    })?;
                    assets_root = Some(PathBuf::from(value));
                }
                other => {
                    return Err(BackendError::BadRequest(format!(
                        "unknown argument '{other}'"
                    )));
                }
            }
        }

        Ok(Self {
            host,
            port,
            scene_path,
            web_root,
            assets_root,
        })
    }
}

struct BackendApp {
    scene_path: PathBuf,
    web_root: PathBuf,
    project: Option<Project>,
    server: EditorBridgeServer,
    dirty: bool,
}

impl BackendApp {
    fn open(
        scene_path: PathBuf,
        web_root: PathBuf,
        assets_root: Option<PathBuf>,
    ) -> Result<Self, BackendError> {
        let scene_path = absolutize_path(&scene_path)?;
        let web_root = absolutize_path(&web_root)?;
        if !web_root.join("index.html").is_file() {
            return Err(BackendError::BadRequest(format!(
                "web root '{}' does not contain index.html",
                web_root.display()
            )));
        }

        let project = infer_browser_project(&scene_path, assets_root.as_deref())?;
        let mut server = EditorBridgeServer::open_hsc(&scene_path)?;
        server.host_mut().session_mut().set_project(project.clone());
        Ok(Self {
            scene_path,
            web_root,
            project,
            server,
            dirty: false,
        })
    }

    fn current_source_text(&self) -> Result<String, BackendError> {
        self.server
            .host()
            .session()
            .scene()
            .to_hsc_string()
            .map_err(BackendError::Bridge)
    }

    fn session_json(&self) -> Result<String, BackendError> {
        let source_text = self.current_source_text()?;
        let source_path = self
            .server
            .host()
            .session()
            .source_path()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let startup_messages = self.server.startup_messages();
        let play_state = format!("{:?}", self.server.host().play_state());
        let scene_name = self.server.host().session().scene().name.clone();
        let mut json = String::new();
        write!(
            json,
            "{{\"backend\":{},\"sceneName\":{},\"sourcePath\":{},\"playState\":{},\"dirty\":{},\"startupMessages\":{},\"sourceText\":{}}}",
            json_string("rust"),
            json_string(&scene_name),
            json_string(&source_path),
            json_string(&play_state),
            if self.dirty || self.server.host().is_dirty() {
                "true"
            } else {
                "false"
            },
            json_string_array(&startup_messages),
            json_string(&source_text),
        )
        .expect("writing JSON into String should not fail");
        Ok(json)
    }

    fn browser_model(&self) -> Result<Option<EditorBrowserModel>, BackendError> {
        EditorBrowserModel::from_host(self.server.host()).map_err(BackendError::Browser)
    }

    fn browser_json(&self) -> Result<String, BackendError> {
        Ok(json_browser_model(self.browser_model()?.as_ref()))
    }

    fn apply_scene_text(&mut self, source_text: &str) -> Result<String, BackendError> {
        let scene = SceneDocument::from_hsc_str(source_text).map_err(BackendError::Parse)?;
        let mut session = EditorSceneSession::new(scene);
        session.set_source_path(Some(self.scene_path.clone()));
        session.set_project(self.project.clone());
        let host = EditorRuntimeHost::new(session)?;
        self.server = EditorBridgeServer::new(host);
        self.dirty = true;
        self.current_source_text()
    }

    fn save_scene_text(&mut self, source_text: &str) -> Result<String, BackendError> {
        self.apply_scene_text(source_text)?;
        self.server.host().save_source_hsc()?;
        self.dirty = false;
        self.current_source_text()
    }

    fn reload_scene_text(&mut self) -> Result<String, BackendError> {
        self.server = EditorBridgeServer::open_hsc(&self.scene_path)?;
        self.server
            .host_mut()
            .session_mut()
            .set_project(self.project.clone());
        self.dirty = false;
        self.current_source_text()
    }

    fn execute_protocol_message(&mut self, message: &str) -> Result<String, BackendError> {
        let trimmed = message.trim();
        if trimmed.is_empty() {
            return Err(BackendError::BadRequest(
                "protocol command body cannot be empty".into(),
            ));
        }

        let encoded = self.server.execute_message(trimmed)?;
        if trimmed == "cmd|Save" || trimmed == "cmd|ReloadSource" {
            self.dirty = false;
        }
        Ok(encoded.join("\n"))
    }

    fn serve_project_file(&self, project_path: &str) -> Result<HttpResponse, BackendError> {
        let path = self.resolve_project_file_path(project_path)?;
        if !path.is_file() {
            return Err(BackendError::NotFound(format!(
                "no file for '{}'",
                project_path
            )));
        }

        let content_type = guess_content_type(&path);
        let body = fs::read(&path).map_err(BackendError::Io)?;
        Ok(HttpResponse::binary(200, "OK", content_type, body))
    }

    fn serve_project_preview(&self, project_path: &str) -> Result<HttpResponse, BackendError> {
        let path = self.resolve_project_file_path(project_path)?;
        let preview = preview_image_asset(&path)?;
        Ok(HttpResponse::binary(
            200,
            "OK",
            preview.content_type,
            preview.body,
        ))
    }

    fn resolve_project_file_path(&self, project_path: &str) -> Result<PathBuf, BackendError> {
        let model = self
            .browser_model()?
            .ok_or_else(|| BackendError::NotFound("browser model is unavailable".into()))?;
        let entry = model
            .entries
            .iter()
            .find(|entry| entry.project_path == project_path)
            .ok_or_else(|| {
                BackendError::NotFound(format!("no browser file for '{}'", project_path))
            })?;
        if entry.kind != EditorBrowserEntryKind::File {
            return Err(BackendError::BadRequest(format!(
                "'{}' is not a file entry",
                project_path
            )));
        }

        let project = self
            .project
            .as_ref()
            .ok_or_else(|| BackendError::NotFound("project context is unavailable".into()))?;
        resolve_project_path(Path::new(&project.root), project_path)
    }

    fn resolve_static_path(&self, request_path: &str) -> Result<PathBuf, BackendError> {
        let clean_path = request_path
            .split('?')
            .next()
            .unwrap_or("/")
            .trim_start_matches('/');
        let relative = if clean_path.is_empty() {
            Path::new("index.html")
        } else {
            Path::new(clean_path)
        };

        let mut resolved = self.web_root.clone();
        for component in relative.components() {
            match component {
                Component::CurDir => {}
                Component::Normal(part) => resolved.push(part),
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(BackendError::BadRequest(format!(
                        "refusing to serve path '{request_path}'"
                    )))
                }
            }
        }

        Ok(resolved)
    }
}

fn handle_client(mut stream: TcpStream, app: &mut BackendApp) -> Result<(), BackendError> {
    let request = read_request(&mut stream)?;
    let response = match dispatch_request(&request, app) {
        Ok(response) => response,
        Err(err) => HttpResponse::text(err.http_status(), err.to_string()),
    };
    write_response(&mut stream, response)
}

fn dispatch_request(
    request: &HttpRequest,
    app: &mut BackendApp,
) -> Result<HttpResponse, BackendError> {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/session") => Ok(HttpResponse::json(app.session_json()?)),
        ("GET", "/api/browser") => Ok(HttpResponse::json(app.browser_json()?)),
        ("GET", "/api/project-file") => {
            let project_path = request.query_param("path").ok_or_else(|| {
                BackendError::BadRequest("missing required query parameter 'path'".into())
            })?;
            app.serve_project_file(&project_path)
        }
        ("GET", "/api/project-preview") => {
            let project_path = request.query_param("path").ok_or_else(|| {
                BackendError::BadRequest("missing required query parameter 'path'".into())
            })?;
            app.serve_project_preview(&project_path)
        }
        ("GET", "/api/scene") => Ok(HttpResponse::plain(app.current_source_text()?)),
        ("POST", "/api/scene") => {
            let source_text = request.utf8_body()?;
            Ok(HttpResponse::plain(app.apply_scene_text(source_text)?))
        }
        ("POST", "/api/save") => {
            let source_text = request.utf8_body()?;
            Ok(HttpResponse::plain(app.save_scene_text(source_text)?))
        }
        ("POST", "/api/reload") => Ok(HttpResponse::plain(app.reload_scene_text()?)),
        ("POST", "/api/command") => {
            let message = request.utf8_body()?;
            Ok(HttpResponse::plain(app.execute_protocol_message(message)?))
        }
        ("GET", _) => serve_static(request, app),
        (method, _) => Err(BackendError::BadRequest(format!(
            "unsupported method '{method}'"
        ))),
    }
}

fn serve_static(request: &HttpRequest, app: &mut BackendApp) -> Result<HttpResponse, BackendError> {
    let path = app.resolve_static_path(&request.path)?;

    if !path.is_file() {
        return Err(BackendError::NotFound(format!(
            "no file for '{}'",
            request.path
        )));
    }

    let content_type = guess_content_type(&path);
    let body = fs::read(&path).map_err(BackendError::Io)?;
    Ok(HttpResponse::binary(200, "OK", content_type, body))
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    query: String,
    body: Vec<u8>,
}

impl HttpRequest {
    fn utf8_body(&self) -> Result<&str, BackendError> {
        std::str::from_utf8(&self.body)
            .map_err(|_| BackendError::BadRequest("request body must be valid UTF-8".into()))
    }

    fn query_param(&self, name: &str) -> Option<String> {
        for pair in self.query.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (raw_name, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
            let decoded_name = percent_decode(raw_name).ok()?;
            if decoded_name == name {
                return percent_decode(raw_value).ok();
            }
        }
        None
    }
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, BackendError> {
    let mut buffer = Vec::new();
    let mut temp = [0u8; 4096];
    let header_end = loop {
        if buffer.len() > MAX_HEADER_BYTES {
            return Err(BackendError::BadRequest(
                "request headers exceed the size limit".into(),
            ));
        }

        let read = stream.read(&mut temp).map_err(BackendError::Io)?;
        if read == 0 {
            return Err(BackendError::BadRequest(
                "connection closed before request headers completed".into(),
            ));
        }
        buffer.extend_from_slice(&temp[..read]);

        if let Some(index) = find_bytes(&buffer, b"\r\n\r\n") {
            break index;
        }
    };

    let header_text = std::str::from_utf8(&buffer[..header_end])
        .map_err(|_| BackendError::BadRequest("request headers must be valid UTF-8".into()))?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| BackendError::BadRequest("request line is missing".into()))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| BackendError::BadRequest("request method is missing".into()))?
        .to_string();
    let raw_path = request_parts
        .next()
        .ok_or_else(|| BackendError::BadRequest("request path is missing".into()))?;
    let (path, query) = match raw_path.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (raw_path.to_string(), String::new()),
    };
    if request_parts.next().is_none() {
        return Err(BackendError::BadRequest(
            "request line must include an HTTP version".into(),
        ));
    }

    let mut content_length = 0usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().map_err(|_| {
                    BackendError::BadRequest("invalid Content-Length header".into())
                })?;
            }
        }
    }

    if content_length > MAX_BODY_BYTES {
        return Err(BackendError::BadRequest(
            "request body exceeds the size limit".into(),
        ));
    }

    let mut body = buffer[(header_end + 4)..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut temp).map_err(BackendError::Io)?;
        if read == 0 {
            return Err(BackendError::BadRequest(
                "connection closed before request body completed".into(),
            ));
        }
        body.extend_from_slice(&temp[..read]);
        if body.len() > MAX_BODY_BYTES {
            return Err(BackendError::BadRequest(
                "request body exceeds the size limit".into(),
            ));
        }
    }
    body.truncate(content_length);

    Ok(HttpRequest {
        method,
        path,
        query,
        body,
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

struct HttpResponse {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

impl HttpResponse {
    fn binary(
        status: u16,
        reason: &'static str,
        content_type: &'static str,
        body: Vec<u8>,
    ) -> Self {
        Self {
            status,
            reason,
            content_type,
            body,
        }
    }

    fn plain(body: impl Into<String>) -> Self {
        Self::binary(
            200,
            "OK",
            "text/plain; charset=utf-8",
            body.into().into_bytes(),
        )
    }

    fn json(body: impl Into<String>) -> Self {
        Self::binary(
            200,
            "OK",
            "application/json; charset=utf-8",
            body.into().into_bytes(),
        )
    }

    fn text(status: u16, body: impl Into<String>) -> Self {
        Self::binary(
            status,
            reason_phrase(status),
            "text/plain; charset=utf-8",
            body.into().into_bytes(),
        )
    }
}

fn write_response(stream: &mut TcpStream, response: HttpResponse) -> Result<(), BackendError> {
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        response.status,
        response.reason,
        response.content_type,
        response.body.len()
    )
    .map_err(BackendError::Io)?;
    stream.write_all(&response.body).map_err(BackendError::Io)?;
    stream.flush().map_err(BackendError::Io)
}

fn guess_content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("md") => "text/markdown; charset=utf-8",
        Some("txt") | Some("hmat") | Some("obj") | Some("rs") => "text/plain; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("bmp") => "image/bmp",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            control if control.is_control() => {
                write!(escaped, "\\u{:04x}", control as u32)
                    .expect("writing JSON escape into String should not fail");
            }
            _ => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

fn json_string_array(values: &[String]) -> String {
    let mut out = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&json_string(value));
    }
    out.push(']');
    out
}

fn json_optional_string(value: Option<&str>) -> String {
    match value {
        Some(value) => json_string(value),
        None => "null".into(),
    }
}

fn json_browser_model(model: Option<&EditorBrowserModel>) -> String {
    let Some(model) = model else {
        return "null".into();
    };

    let mut entries = String::from("[");
    for (index, entry) in model.entries.iter().enumerate() {
        if index > 0 {
            entries.push(',');
        }
        write!(
            entries,
            "{{\"root\":{},\"kind\":{},\"name\":{},\"projectPath\":{},\"depth\":{}}}",
            json_string(match entry.root {
                EditorBrowserRootKind::Scenes => "scenes",
                EditorBrowserRootKind::Assets => "assets",
            }),
            json_string(match entry.kind {
                EditorBrowserEntryKind::Directory => "directory",
                EditorBrowserEntryKind::File => "file",
            }),
            json_string(&entry.name),
            json_string(&entry.project_path),
            entry.depth,
        )
        .expect("writing JSON into String should not fail");
    }
    entries.push(']');

    let mut out = String::new();
    write!(
        out,
        "{{\"projectName\":{},\"selectedPath\":{},\"entries\":{}}}",
        json_string(&model.project_name),
        json_optional_string(model.selected_path.as_deref()),
        entries,
    )
    .expect("writing JSON into String should not fail");
    out
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Error",
    }
}

#[derive(Debug)]
enum BackendError {
    Io(std::io::Error),
    Parse(SceneError),
    Bridge(SceneBridgeError),
    Browser(EditorBrowserError),
    Preview(String),
    Session(EditorSessionError),
    Protocol(EditorProtocolError),
    BadRequest(String),
    NotFound(String),
}

impl BackendError {
    fn http_status(&self) -> u16 {
        match self {
            Self::BadRequest(_) | Self::Parse(_) | Self::Protocol(_) => 400,
            Self::NotFound(_) => 404,
            Self::Io(_)
            | Self::Bridge(_)
            | Self::Browser(_)
            | Self::Preview(_)
            | Self::Session(_) => 500,
        }
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Parse(err) => write!(f, "{err}"),
            Self::Bridge(err) => write!(f, "{err}"),
            Self::Browser(err) => write!(f, "{err}"),
            Self::Preview(message) => write!(f, "preview error: {message}"),
            Self::Session(err) => write!(f, "{err}"),
            Self::Protocol(err) => write!(f, "{err}"),
            Self::BadRequest(message) => write!(f, "bad request: {message}"),
            Self::NotFound(message) => write!(f, "not found: {message}"),
        }
    }
}

impl std::error::Error for BackendError {}

impl From<std::io::Error> for BackendError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<EditorSessionError> for BackendError {
    fn from(err: EditorSessionError) -> Self {
        Self::Session(err)
    }
}

impl From<EditorBrowserError> for BackendError {
    fn from(err: EditorBrowserError) -> Self {
        Self::Browser(err)
    }
}

impl From<EditorProtocolError> for BackendError {
    fn from(err: EditorProtocolError) -> Self {
        Self::Protocol(err)
    }
}

fn absolutize_path(path: &Path) -> Result<PathBuf, BackendError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(env::current_dir().map_err(BackendError::Io)?.join(path))
    }
}

fn infer_browser_project(
    scene_path: &Path,
    assets_root_override: Option<&Path>,
) -> Result<Option<Project>, BackendError> {
    let scene_dir = match scene_path.parent() {
        Some(dir) => dir,
        None => return Ok(None),
    };
    let scene_parent = scene_dir.parent().unwrap_or(scene_dir);
    let assets_dir = match assets_root_override {
        Some(path) => absolutize_path(path)?,
        None => infer_assets_dir(scene_dir, scene_parent),
    };
    let project_root = if assets_dir.starts_with(scene_parent) {
        scene_parent.to_path_buf()
    } else {
        common_ancestor(scene_dir, &assets_dir)
            .unwrap_or_else(|| scene_parent.to_path_buf())
    };
    let project_name = project_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("browser-backend")
        .to_string();

    Ok(Some(Project {
        name: project_name,
        root: project_root.display().to_string(),
        assets_dir: assets_dir.display().to_string(),
        scenes_dir: scene_dir.display().to_string(),
    }))
}

fn infer_assets_dir(scene_dir: &Path, project_root: &Path) -> PathBuf {
    let candidates = [
        scene_dir.join("source"),
        project_root.join("assets"),
        project_root.join("source"),
        scene_dir.join("assets"),
    ];
    candidates
        .iter()
        .find(|path| path.is_dir())
        .cloned()
        .unwrap_or_else(|| project_root.join("assets"))
}

fn common_ancestor(left: &Path, right: &Path) -> Option<PathBuf> {
    let left_components = left.components().collect::<Vec<_>>();
    let right_components = right.components().collect::<Vec<_>>();
    let shared = left_components
        .iter()
        .zip(right_components.iter())
        .take_while(|(left, right)| left == right)
        .count();
    if shared == 0 {
        return None;
    }

    let mut path = PathBuf::new();
    for component in left_components.into_iter().take(shared) {
        path.push(component.as_os_str());
    }
    Some(path)
}

fn resolve_project_path(project_root: &Path, project_path: &str) -> Result<PathBuf, BackendError> {
    let mut resolved = project_root.to_path_buf();
    for component in Path::new(project_path).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => resolved.push(part),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(BackendError::BadRequest(format!(
                    "refusing to serve project path '{}'",
                    project_path
                )));
            }
        }
    }
    Ok(resolved)
}

fn percent_decode(input: &str) -> Result<String, BackendError> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                decoded.push(
                    (hex_value(bytes[index + 1])? << 4) | hex_value(bytes[index + 2])?,
                );
                index += 3;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8(decoded)
        .map_err(|_| BackendError::BadRequest("query parameter must be valid UTF-8".into()))
}

fn hex_value(byte: u8) -> Result<u8, BackendError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(BackendError::BadRequest(
            "query parameter contains an invalid percent escape".into(),
        )),
    }
}

struct PreviewAsset {
    content_type: &'static str,
    body: Vec<u8>,
}

fn preview_image_asset(path: &Path) -> Result<PreviewAsset, BackendError> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("bmp") | Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("webp") => {
            Ok(PreviewAsset {
                content_type: guess_content_type(path),
                body: fs::read(path).map_err(BackendError::Io)?,
            })
        }
        Some("tga") => {
            let texture = load_tga(path)
                .map_err(|err| BackendError::Preview(format!("failed to decode TGA: {err}")))?;
            Ok(PreviewAsset {
                content_type: "image/bmp",
                body: encode_texture_as_bmp(&texture)?,
            })
        }
        _ => Err(BackendError::BadRequest(format!(
            "'{}' does not support image preview",
            path.display()
        ))),
    }
}

fn encode_texture_as_bmp(texture: &Texture) -> Result<Vec<u8>, BackendError> {
    let bytes_per_pixel = match texture.format {
        TextureFormat::Rgba8 => 4usize,
        TextureFormat::Rgb8 => 3usize,
        TextureFormat::R8 => 1usize,
    };
    let width = usize::try_from(texture.width)
        .map_err(|_| BackendError::Preview("texture width exceeds supported preview size".into()))?;
    let height = usize::try_from(texture.height).map_err(|_| {
        BackendError::Preview("texture height exceeds supported preview size".into())
    })?;
    let expected_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
        .ok_or_else(|| BackendError::Preview("texture preview size overflowed".into()))?;
    if texture.data.len() != expected_len {
        return Err(BackendError::Preview(format!(
            "texture payload length {} does not match {}x{} {:?}",
            texture.data.len(),
            texture.width,
            texture.height,
            texture.format
        )));
    }

    let mut pixels = Vec::with_capacity(width * height * 4);
    for row in (0..height).rev() {
        for col in 0..width {
            let pixel_index = (row * width + col) * bytes_per_pixel;
            let (r, g, b, a) = match texture.format {
                TextureFormat::Rgba8 => (
                    texture.data[pixel_index],
                    texture.data[pixel_index + 1],
                    texture.data[pixel_index + 2],
                    texture.data[pixel_index + 3],
                ),
                TextureFormat::Rgb8 => (
                    texture.data[pixel_index],
                    texture.data[pixel_index + 1],
                    texture.data[pixel_index + 2],
                    255,
                ),
                TextureFormat::R8 => {
                    let value = texture.data[pixel_index];
                    (value, value, value, 255)
                }
            };
            pixels.extend_from_slice(&[b, g, r, a]);
        }
    }

    let header_size = 14usize + 40usize;
    let file_size = header_size
        .checked_add(pixels.len())
        .ok_or_else(|| BackendError::Preview("bmp preview size overflowed".into()))?;
    let file_size_u32 = u32::try_from(file_size)
        .map_err(|_| BackendError::Preview("bmp preview exceeds file size limits".into()))?;
    let pixel_size_u32 = u32::try_from(pixels.len())
        .map_err(|_| BackendError::Preview("bmp preview exceeds pixel size limits".into()))?;
    let width_i32 = i32::try_from(texture.width)
        .map_err(|_| BackendError::Preview("texture width exceeds BMP preview limits".into()))?;
    let height_i32 = i32::try_from(texture.height)
        .map_err(|_| BackendError::Preview("texture height exceeds BMP preview limits".into()))?;

    let mut out = Vec::with_capacity(file_size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&file_size_u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(54u32).to_le_bytes());
    out.extend_from_slice(&(40u32).to_le_bytes());
    out.extend_from_slice(&width_i32.to_le_bytes());
    out.extend_from_slice(&height_i32.to_le_bytes());
    out.extend_from_slice(&(1u16).to_le_bytes());
    out.extend_from_slice(&(32u16).to_le_bytes());
    out.extend_from_slice(&(0u32).to_le_bytes());
    out.extend_from_slice(&pixel_size_u32.to_le_bytes());
    out.extend_from_slice(&(2835u32).to_le_bytes());
    out.extend_from_slice(&(2835u32).to_le_bytes());
    out.extend_from_slice(&(0u32).to_le_bytes());
    out.extend_from_slice(&(0u32).to_le_bytes());
    out.extend_from_slice(&pixels);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hold::assets::decode_bmp;

    fn unique_test_root(label: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "hold_browser_backend_{}_{}_{}",
            label,
            std::process::id(),
            nonce
        ))
    }

    #[test]
    fn inferred_project_uses_scene_local_source_assets() {
        let root = unique_test_root("scene_source");
        let scene_dir = root.join("examples");
        fs::create_dir_all(scene_dir.join("source")).unwrap();
        let scene_path = scene_dir.join("demo.hsc");
        fs::write(&scene_path, "scene \"demo\"").unwrap();

        let project = infer_browser_project(&scene_path, None)
            .unwrap()
            .expect("project should be inferred");

        assert_eq!(project.root, root.display().to_string());
        assert_eq!(project.scenes_dir, scene_dir.display().to_string());
        assert_eq!(
            project.assets_dir,
            scene_dir.join("source").display().to_string()
        );

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn inferred_project_prefers_parent_assets_for_scene_subdir_layout() {
        let root = unique_test_root("project_assets");
        let scene_dir = root.join("scenes");
        let assets_dir = root.join("assets");
        fs::create_dir_all(&scene_dir).unwrap();
        fs::create_dir_all(&assets_dir).unwrap();
        let scene_path = scene_dir.join("level01.hsc");
        fs::write(&scene_path, "scene \"demo\"").unwrap();

        let project = infer_browser_project(&scene_path, None)
            .unwrap()
            .expect("project should be inferred");

        assert_eq!(project.root, root.display().to_string());
        assert_eq!(project.scenes_dir, scene_dir.display().to_string());
        assert_eq!(project.assets_dir, assets_dir.display().to_string());

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn resolve_project_path_rejects_parent_traversal() {
        let err = resolve_project_path(Path::new("C:/demo"), "../secrets.txt").unwrap_err();
        assert!(matches!(err, BackendError::BadRequest(_)));
    }

    #[test]
    fn percent_decode_handles_url_encoded_paths() {
        let decoded = percent_decode("examples%2Fsource%2Ftextures%2Fcrate.bmp").unwrap();
        assert_eq!(decoded, "examples/source/textures/crate.bmp");
    }

    #[test]
    fn bmp_preview_encoder_roundtrips_rgba_textures() {
        use hold::ColorSpace;
        let encoded = encode_texture_as_bmp(&Texture {
            width: 2,
            height: 2,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 64,
            ],
        })
        .unwrap();
        let decoded = decode_bmp(&encoded).unwrap();

        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 2);
        assert_eq!(decoded.format, TextureFormat::Rgba8);
        assert_eq!(decoded.data.len(), 16);
        assert_eq!(&decoded.data[0..4], &[255, 0, 0, 255]);
        assert_eq!(&decoded.data[12..16], &[255, 255, 255, 64]);
    }

    #[test]
    fn tga_preview_transcodes_to_browser_readable_bmp() {
        let root = unique_test_root("tga_preview");
        fs::create_dir_all(&root).unwrap();
        let tga_path = root.join("preview.tga");
        fs::write(
            &tga_path,
            [
                0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 24, 32, 0, 0, 255, 0, 255,
                0, 255, 255, 0, 0, 255, 255,
            ],
        )
        .unwrap();

        let preview = preview_image_asset(&tga_path).unwrap();
        let decoded = decode_bmp(&preview.body).unwrap();

        assert_eq!(preview.content_type, "image/bmp");
        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 2);

        fs::remove_dir_all(root).ok();
    }
}
