//! The AI server behind "Align with AI": a local one (LM Studio, Ollama, llama.cpp,
//! KoboldCpp, vLLM …) or OpenRouter, both OpenAI-compatible (`normalize_ai_url`,
//! `_ai_open`, `list_ai_models_detailed`, `check_openrouter_key`, `AiCheckWorker` and
//! `AiAlignWorker` in `lrcplayer.py`).
//!
//! Requests go over a small HTTP/1.1 client of its own instead of ureq, for the same
//! reasons the Python app uses `http.client` instead of urllib: a connect timeout apart
//! from the read timeout, a read timeout that applies to every read of a streamed
//! answer, and a cancel that closes the socket, which also stops most servers
//! generating.
//!
//! The settings rules (`ai_settings_raw`, `ai_settings`, `save_ai_settings`), the key
//! format (`protect_secret`) and the texts of the AI settings window are here too.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::align::{self, Message};
use crate::py;
use crate::settings::Settings;

pub const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";
pub const OPENROUTER_KEYS_URL: &str = "https://openrouter.ai/keys";

/// `AiError`: a problem worded for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiError(pub String);

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AiError {}

// ---------------------------------------------------------------------------------------
// Addresses
// ---------------------------------------------------------------------------------------

/// `urllib.parse.urlsplit`, for the parts used here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SplitUrl {
    scheme: String,
    netloc: String,
    path: String,
}

fn urlsplit(url: &str) -> SplitUrl {
    // Python strips leading C0 controls and spaces, and removes tabs and line breaks.
    let url: String = url
        .trim_start_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    let mut rest = url.as_str();
    let mut scheme = String::new();
    if let Some(i) = rest.find(':')
        && i > 0
        && rest.as_bytes()[0].is_ascii_alphabetic()
        && rest[..i]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    {
        scheme = rest[..i].to_ascii_lowercase();
        rest = &rest[i + 1..];
    }
    let mut netloc = String::new();
    if let Some(after) = rest.strip_prefix("//") {
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        netloc = after[..end].to_owned();
        rest = &after[end..];
    }
    let rest = rest.split('#').next().unwrap_or("");
    let path = rest.split('?').next().unwrap_or("");
    SplitUrl {
        scheme,
        netloc,
        path: path.to_owned(),
    }
}

impl SplitUrl {
    /// `urlunsplit((scheme, netloc, path, "", ""))`.
    fn unsplit(&self) -> String {
        let mut url = self.path.clone();
        let uses_netloc = matches!(
            self.scheme.as_str(),
            "" | "ftp"
                | "http"
                | "gopher"
                | "nntp"
                | "telnet"
                | "imap"
                | "wais"
                | "file"
                | "mms"
                | "https"
                | "shttp"
                | "snews"
                | "prospero"
                | "rtsp"
                | "rtsps"
                | "rtspu"
                | "rsync"
                | "svn"
                | "svn+ssh"
                | "sftp"
                | "nfs"
                | "git"
                | "git+ssh"
                | "ws"
                | "wss"
                | "itms-services"
        );
        if !self.netloc.is_empty()
            || (!self.scheme.is_empty() && uses_netloc && !url.starts_with("//"))
        {
            if !url.is_empty() && !url.starts_with('/') {
                url = format!("/{url}");
            }
            url = format!("//{}{url}", self.netloc);
        }
        if !self.scheme.is_empty() {
            url = format!("{}:{url}", self.scheme);
        }
        url
    }

    /// `hostname` (lower case, without brackets) and `port`. The port is `Err` when it
    /// isn't a number from 0 to 65535, where Python raises.
    fn host_port(&self) -> (String, Result<Option<u16>, ()>) {
        let hostinfo = self.netloc.rsplit('@').next().unwrap_or("");
        let (host, port) = if let Some(rest) = hostinfo.strip_prefix('[') {
            match rest.split_once(']') {
                Some((h, after)) => (h, after.strip_prefix(':')),
                None => (rest, None),
            }
        } else {
            match hostinfo.split_once(':') {
                Some((h, p)) => (h, Some(p)),
                None => (hostinfo, None),
            }
        };
        let port = match port {
            None | Some("") => Ok(None),
            Some(p) if p.bytes().all(|b| b.is_ascii_digit()) => {
                p.parse::<u16>().map(Some).map_err(|_| ())
            }
            Some(_) => Err(()),
        };
        (host.to_lowercase(), port)
    }
}

/// `normalize_ai_url`: "localhost:1234", "…/v1/chat/completions", Ollama's "…:11434"
/// → "http://host:port/v1".
pub fn normalize_ai_url(url: &str) -> String {
    let mut u = py::strip(url).trim_end_matches('/').to_owned();
    if u.is_empty() {
        return String::new();
    }
    if !u.contains("://") {
        u = format!("http://{u}");
    }
    for suffix in ["/chat/completions", "/completions", "/models"] {
        if u.to_lowercase().ends_with(suffix) {
            // The suffix is ASCII, so it has the same length in lower case.
            let keep = u.len() - suffix.len();
            if u.is_char_boundary(keep) {
                u = u[..keep].trim_end_matches('/').to_owned();
            }
        }
    }
    let mut parts = urlsplit(&u);
    let path = parts.path.trim_end_matches('/');
    parts.path = if path.is_empty() || path == "/api" {
        "/v1".to_owned()
    } else {
        path.to_owned()
    };
    parts.unsplit()
}

// ---------------------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------------------

/// Stops a request from another thread (`AiAlignWorker.cancel`): marks it cancelled and
/// shuts the socket down, which also stops most servers generating.
#[derive(Default)]
pub struct Cancel {
    cancelled: AtomicBool,
    socket: Mutex<Option<TcpStream>>,
}

impl Cancel {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Ok(guard) = self.socket.lock()
            && let Some(sock) = guard.as_ref()
        {
            let _ = sock.shutdown(Shutdown::Both);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn watch(&self, sock: &TcpStream) {
        if let Ok(mut guard) = self.socket.lock() {
            *guard = sock.try_clone().ok();
            if self.is_cancelled()
                && let Some(s) = guard.as_ref()
            {
                let _ = s.shutdown(Shutdown::Both);
            }
        }
    }

    fn forget(&self) {
        if let Ok(mut guard) = self.socket.lock() {
            *guard = None;
        }
    }
}

enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            // A server that closes without saying goodbye has still finished (Python's
            // ssl module treats it the same way: `suppress_ragged_eofs`).
            Stream::Tls(s) => match s.read(buf) {
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(0),
                other => other,
            },
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(buf),
            Stream::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            Stream::Tls(s) => s.flush(),
        }
    }
}

/// Why a request failed, so each caller can word it as the Python code does.
#[derive(Debug)]
enum Fail {
    /// Already worded for the user (`AiError`).
    Ai(String),
    /// A read or write timed out (`socket.timeout`).
    Timeout,
    /// Any other socket problem (`OSError`).
    Io(String),
    /// The answer wasn't valid JSON (`ValueError`).
    Json(String),
    /// Anything else (the bare `Exception`).
    Other(String),
}

impl From<AiError> for Fail {
    fn from(e: AiError) -> Self {
        Fail::Ai(e.0)
    }
}

fn is_timeout(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

impl From<io::Error> for Fail {
    fn from(e: io::Error) -> Self {
        if is_timeout(&e) {
            Fail::Timeout
        } else {
            Fail::Io(e.to_string())
        }
    }
}

/// `_ai_connect_error`.
fn connect_error_text(e: &io::Error, where_: &str) -> String {
    match e.kind() {
        io::ErrorKind::ConnectionRefused => {
            format!("Nothing is answering at {where_}. Is the model server running?")
        }
        _ if is_timeout(e) => format!(
            "The server at {where_} didn't respond. Check the address and that the server is running."
        ),
        _ => format!("Couldn't connect to {where_}: {e}"),
    }
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    use std::sync::OnceLock;
    static CONFIG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let verifier = rustls_platform_verifier::Verifier::new(provider.clone())
                .map_err(|e| e.to_string())?;
            let config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| e.to_string())?
                .dangerous() // not dangerous: this is the system's own verifier
                .with_custom_certificate_verifier(Arc::new(verifier))
                .with_no_client_auth();
            Ok(Arc::new(config))
        })
        .clone()
}

/// An open connection to the server (`http.client.HTTPConnection`).
struct Conn {
    stream: Stream,
    host_header: String,
}

/// `_ai_open`: connects within `connect_timeout`; every later read may then wait
/// `read_timeout`. Returns the connection and the address's path without a final "/".
fn ai_open(
    base_url: &str,
    connect_timeout: Duration,
    read_timeout: Duration,
    cancel: Option<&Cancel>,
) -> Result<(Conn, String), AiError> {
    let parts = urlsplit(base_url);
    let (host, port) = parts.host_port();
    let https = parts.scheme == "https";
    if !matches!(parts.scheme.as_str(), "http" | "https") || host.is_empty() {
        return Err(AiError(format!(
            "“{base_url}” isn't a valid server address."
        )));
    }
    let Ok(port) = port else {
        return Err(AiError(format!(
            "“{base_url}” isn't a valid server address."
        )));
    };
    let port = port.unwrap_or(if https { 443 } else { 80 });
    let where_ = parts.netloc.as_str();

    let addrs: Vec<_> = match (host.as_str(), port).to_socket_addrs() {
        Ok(a) => a.collect(),
        Err(_) => Vec::new(),
    };
    if addrs.is_empty() {
        return Err(AiError(format!(
            "Couldn't find the server “{where_}”. Check the address."
        )));
    }
    // `socket.create_connection`: each address in turn, the last error if none works.
    let mut last_err = None;
    let mut sock = None;
    for addr in &addrs {
        match TcpStream::connect_timeout(addr, connect_timeout) {
            Ok(s) => {
                sock = Some(s);
                break;
            }
            Err(e) => last_err = Some(e),
        }
    }
    let Some(sock) = sock else {
        let e = last_err.unwrap_or_else(|| io::Error::other("no address"));
        return Err(AiError(connect_error_text(&e, where_)));
    };
    let _ = sock.set_nodelay(true);
    let set_timeouts = |s: &TcpStream, t: Duration| {
        let _ = s.set_read_timeout(Some(t));
        let _ = s.set_write_timeout(Some(t));
    };
    set_timeouts(&sock, connect_timeout);
    if let Some(cancel) = cancel {
        cancel.watch(&sock);
    }

    let stream = if https {
        let config =
            tls_config().map_err(|e| AiError(format!("Couldn't connect to {where_}: {e}")))?;
        let name = rustls::pki_types::ServerName::try_from(host.clone())
            .map_err(|e| AiError(format!("Couldn't connect to {where_}: {e}")))?;
        let conn = rustls::ClientConnection::new(config, name)
            .map_err(|e| AiError(format!("Couldn't connect to {where_}: {e}")))?;
        let mut tls = rustls::StreamOwned::new(conn, sock);
        // The handshake belongs to connecting, as in `HTTPSConnection.connect`.
        while tls.conn.is_handshaking() {
            if let Err(e) = tls.conn.complete_io(&mut tls.sock) {
                return Err(AiError(connect_error_text(&e, where_)));
            }
        }
        set_timeouts(&tls.sock, read_timeout);
        Stream::Tls(Box::new(tls))
    } else {
        set_timeouts(&sock, read_timeout);
        Stream::Plain(sock)
    };

    let default_port = if https { 443 } else { 80 };
    let shown_host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.clone()
    };
    let host_header = if port == default_port {
        shown_host
    } else {
        format!("{shown_host}:{port}")
    };
    let path = parts.path.trim_end_matches('/').to_owned();
    Ok((
        Conn {
            stream,
            host_header,
        },
        path,
    ))
}

/// `_ai_headers`.
fn ai_headers(key: &str, extra: &[(String, String)]) -> Vec<(String, String)> {
    let mut headers = vec![
        ("Content-Type".to_owned(), "application/json".to_owned()),
        (
            "Accept".to_owned(),
            "text/event-stream, application/json".to_owned(),
        ),
        (
            "User-Agent".to_owned(),
            "LRCOpenPlayer (translation aligner)".to_owned(),
        ),
    ];
    if !key.is_empty() {
        headers.push(("Authorization".to_owned(), format!("Bearer {key}")));
    }
    for (name, value) in extra {
        match headers
            .iter_mut()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
        {
            Some(h) => h.1 = value.clone(),
            None => headers.push((name.clone(), value.clone())),
        }
    }
    headers
}

/// How the body of an answer is framed.
enum Framing {
    Length(u64),
    Chunked { left: u64, done: bool },
    UntilClose,
}

/// The body of an answer, without its HTTP framing.
struct Body {
    raw: BufReader<Stream>,
    framing: Framing,
}

impl Read for Body {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        match &mut self.framing {
            Framing::Length(0) => Ok(0),
            Framing::Length(left) => {
                let max = (*left).min(buf.len() as u64) as usize;
                let n = self.raw.read(&mut buf[..max])?;
                if n == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "the server closed the connection before the end of its answer",
                    ));
                }
                *left -= n as u64;
                Ok(n)
            }
            Framing::UntilClose => self.raw.read(buf),
            Framing::Chunked { left, done } => {
                if *done {
                    return Ok(0);
                }
                if *left == 0 {
                    let mut line = String::new();
                    self.raw.read_line(&mut line)?;
                    let size = line.trim().split(';').next().unwrap_or("").trim();
                    let size = u64::from_str_radix(size, 16).map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidData, "bad chunk length")
                    })?;
                    if size == 0 {
                        // Trailers up to the empty line.
                        loop {
                            let mut t = String::new();
                            if self.raw.read_line(&mut t)? == 0 || t.trim().is_empty() {
                                break;
                            }
                        }
                        *done = true;
                        return Ok(0);
                    }
                    *left = size;
                }
                let max = (*left).min(buf.len() as u64) as usize;
                let n = self.raw.read(&mut buf[..max])?;
                if n == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "the server closed the connection in the middle of its answer",
                    ));
                }
                *left -= n as u64;
                if *left == 0 {
                    let mut crlf = String::new();
                    self.raw.read_line(&mut crlf)?;
                }
                Ok(n)
            }
        }
    }
}

/// An answer: status, headers and the body to read.
struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: BufReader<Body>,
}

impl Response {
    /// `getheader`: every value of a header, joined with ", ".
    fn header(&self, name: &str) -> Option<String> {
        let values: Vec<&str> = self
            .headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect();
        (!values.is_empty()).then(|| values.join(", "))
    }

    fn read_all(&mut self) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        self.body.read_to_end(&mut out)?;
        Ok(out)
    }

    /// `readline`: one line with its line break, or nothing at the end.
    fn read_line(&mut self) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        self.body.read_until(b'\n', &mut out)?;
        Ok(out)
    }
}

impl Conn {
    /// `request` + `getresponse`.
    fn request(
        self,
        method: &str,
        path: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
    ) -> Result<Response, Fail> {
        let Conn {
            mut stream,
            host_header,
        } = self;
        let path = if path.is_empty() { "/" } else { path };
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {host_header}\r\nAccept-Encoding: identity\r\n"
        );
        if let Some(body) = body {
            head.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes())?;
        if let Some(body) = body {
            stream.write_all(body)?;
        }
        stream.flush()?;

        let mut raw = BufReader::new(stream);
        loop {
            let mut line = Vec::new();
            raw.read_until(b'\n', &mut line)?;
            if line.is_empty() {
                // `RemoteDisconnected`, a `ConnectionResetError`.
                return Err(Fail::Io(
                    "Remote end closed connection without response".to_owned(),
                ));
            }
            let status_line = String::from_utf8_lossy(&line).trim().to_owned();
            let mut parts = status_line.splitn(3, ' ');
            let version = parts.next().unwrap_or("");
            let status: Option<u16> = parts.next().and_then(|s| s.parse().ok());
            let Some(status) = status.filter(|_| version.starts_with("HTTP/")) else {
                return Err(Fail::Other(format!("{status_line:?}")));
            };
            let mut headers = Vec::new();
            loop {
                let mut h = Vec::new();
                raw.read_until(b'\n', &mut h)?;
                let h = String::from_utf8_lossy(&h);
                let h = h.trim_end_matches(['\r', '\n']);
                if h.is_empty() {
                    break;
                }
                if let Some((n, v)) = h.split_once(':') {
                    headers.push((n.trim().to_owned(), v.trim().to_owned()));
                }
            }
            if (100..200).contains(&status) {
                continue; // "100 Continue" and friends come before the real answer
            }
            let find = |name: &str| {
                headers
                    .iter()
                    .find(|(n, _): &&(String, String)| n.eq_ignore_ascii_case(name))
                    .map(|(_, v)| v.clone())
            };
            let framing = if status == 204 || status == 304 || method == "HEAD" {
                Framing::Length(0)
            } else if find("Transfer-Encoding")
                .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"))
            {
                Framing::Chunked {
                    left: 0,
                    done: false,
                }
            } else if let Some(n) = find("Content-Length").and_then(|v| v.trim().parse().ok()) {
                Framing::Length(n)
            } else {
                Framing::UntilClose
            };
            return Ok(Response {
                status,
                headers,
                body: BufReader::new(Body { raw, framing }),
            });
        }
    }
}

/// Python's `bytes.decode("utf-8", "replace")`.
fn text_of(body: &[u8]) -> String {
    String::from_utf8_lossy(body).into_owned()
}

/// `_ai_http_error`: what an error status means for the user.
pub fn http_error_text(status: u16, body: &[u8], endpoint: &str, model: &str) -> String {
    let text = text_of(body);
    let detail = match serde_json::from_str::<Value>(&text) {
        Ok(data) => {
            let err = match &data {
                Value::Object(map) => map.get("error").cloned().unwrap_or(data.clone()),
                _ => data.clone(),
            };
            match &err {
                Value::Object(map) => match map.get("message") {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Null) | None => String::new(),
                    Some(other) => py_str(other),
                },
                other => py_str(other),
            }
        }
        Err(_) => py::strip(&text).chars().take(200).collect(),
    };
    let detail = py::strip(&detail).to_owned();
    if status == 401 || status == 403 {
        return format!("The server refused the request (HTTP {status}). Check the API key.");
    }
    if status == 402 {
        let mut s = "The account has run out of credits (HTTP 402).".to_owned();
        if !detail.is_empty() {
            s.push_str(&format!(" {detail}"));
        }
        return s;
    }
    if status == 429 {
        return "Too many requests right now (HTTP 429). Free models are often busy; wait a minute or pick another model.".to_owned();
    }
    if !model.is_empty()
        && detail.to_lowercase().contains("model")
        && (status == 400 || status == 404)
    {
        return format!("The server doesn't have the model “{model}” loaded: {detail}");
    }
    if status == 404 {
        return format!(
            "There's no {endpoint} at this address (HTTP 404). OpenAI-compatible servers usually end in /v1."
        );
    }
    if detail.is_empty() {
        format!("The server answered with an error (HTTP {status}).")
    } else {
        format!("The server answered with an error (HTTP {status}): {detail}")
    }
}

/// Python's `str()` of a JSON value, for the few places that print one.
fn py_str(v: &Value) -> String {
    match v {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        other => py_repr(other),
    }
}

/// Python's `repr()` of a JSON value inside a list or dict.
fn py_repr(v: &Value) -> String {
    match v {
        Value::String(s) => {
            let q = if s.contains('\'') && !s.contains('"') {
                '"'
            } else {
                '\''
            };
            let mut out = String::from(q);
            for c in s.chars() {
                match c {
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if c == q => {
                        out.push('\\');
                        out.push(c);
                    }
                    c => out.push(c),
                }
            }
            out.push(q);
            out
        }
        Value::Array(a) => format!("[{}]", a.iter().map(py_repr).collect::<Vec<_>>().join(", ")),
        Value::Object(o) => format!(
            "{{{}}}",
            o.iter()
                .map(|(k, v)| format!("{}: {}", py_repr(&Value::String(k.clone())), py_repr(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => py_str(other),
    }
}

/// Python's truthiness of a JSON value.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Bool(true)) => true,
    }
}

/// `a or b or …` over JSON values.
fn first_truthy<'a>(values: &[Option<&'a Value>]) -> Option<&'a Value> {
    values.iter().copied().find(|v| truthy(*v)).flatten()
}

/// Python's `float()` of a price.
fn price_float(v: Option<&Value>) -> Option<f64> {
    if !truthy(v) {
        return Some(0.0);
    }
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::String(s) => {
            let t = py::strip(s).replace('_', "");
            match t.to_ascii_lowercase().as_str() {
                "inf" | "+inf" | "infinity" | "+infinity" => Some(f64::INFINITY),
                "-inf" | "-infinity" => Some(f64::NEG_INFINITY),
                "nan" | "+nan" | "-nan" => Some(f64::NAN),
                _ => t.parse().ok(),
            }
        }
        _ => None,
    }
}

/// Prices per input and output token.
pub type Pricing = HashMap<String, (f64, f64)>;

fn models_from(data: &Value) -> Result<(Vec<String>, Pricing), Fail> {
    let items = match data {
        Value::Object(map) => map
            .get("data")
            .or_else(|| map.get("models"))
            .cloned()
            .unwrap_or(Value::Array(Vec::new())),
        other => other.clone(),
    };
    // `for item in items or []`: lists, the keys of an object, the letters of a text.
    let items: Vec<Value> = if !truthy(Some(&items)) {
        Vec::new()
    } else {
        match items {
            Value::Array(a) => a,
            Value::Object(o) => o.keys().map(|k| Value::String(k.clone())).collect(),
            Value::String(s) => s.chars().map(|c| Value::String(c.to_string())).collect(),
            Value::Number(_) => {
                return Err(Fail::Other("'int' object is not iterable".to_owned()));
            }
            _ => return Err(Fail::Other("'bool' object is not iterable".to_owned())),
        }
    };
    let mut ids: Vec<String> = Vec::new();
    let mut pricing = Pricing::new();
    for item in &items {
        let mid = match item {
            Value::Object(map) => {
                first_truthy(&[map.get("id"), map.get("name"), map.get("model")]).map(py_str)
            }
            other => Some(py_str(other)).filter(|s| !s.is_empty()),
        };
        let Some(mid) = mid else { continue };
        if !ids.contains(&mid) {
            ids.push(mid.clone());
        }
        if let Value::Object(map) = item
            && let Some(Value::Object(price)) = map.get("pricing")
            && let (Some(p_in), Some(p_out)) = (
                price_float(price.get("prompt")),
                price_float(price.get("completion")),
            )
            && p_in >= 0.0
            && p_out >= 0.0
        {
            pricing.insert(mid, (p_in, p_out));
        }
    }
    Ok((ids, pricing))
}

/// `list_ai_models_detailed`: the model ids the server offers (GET /models), and the
/// prices per input and output token where it gives them.
pub fn list_models(
    base_url: &str,
    key: &str,
    timeout: Duration,
    headers: &[(String, String)],
) -> Result<(Vec<String>, Pricing), AiError> {
    let (conn, path) = ai_open(base_url, timeout, timeout, None)?;
    let result = (|| -> Result<(Vec<String>, Pricing), Fail> {
        let mut resp = conn.request(
            "GET",
            &format!("{path}/models"),
            &ai_headers(key, headers),
            None,
        )?;
        let body = resp.read_all()?;
        if resp.status != 200 {
            return Err(Fail::Ai(http_error_text(resp.status, &body, "/models", "")));
        }
        let data: Value =
            serde_json::from_str(&text_of(&body)).map_err(|e| Fail::Json(e.to_string()))?;
        models_from(&data)
    })();
    result.map_err(|f| match f {
        Fail::Ai(s) => AiError(s),
        Fail::Timeout => AiError("The server didn't answer in time.".to_owned()),
        Fail::Json(_) => AiError(
            "The server's answer wasn't valid JSON. Is this an OpenAI-compatible API?".to_owned(),
        ),
        Fail::Io(e) => AiError(format!("Connection problem: {e}")),
        Fail::Other(e) => AiError(format!("Unexpected problem: {e}")),
    })
}

/// `check_openrouter_key`: asks OpenRouter whether the key is valid. A short note, or
/// "" when it can't be checked this way (the first alignment will show any problem).
pub fn check_openrouter_key(
    base_url: &str,
    key: &str,
    headers: &[(String, String)],
    timeout: Duration,
) -> Result<String, AiError> {
    for endpoint in ["/key", "/auth/key"] {
        let (conn, path) = ai_open(base_url, timeout, timeout, None)?;
        let answer = conn
            .request(
                "GET",
                &format!("{path}{endpoint}"),
                &ai_headers(key, headers),
                None,
            )
            .and_then(|mut resp| Ok((resp.status, resp.read_all()?)));
        let (status, body) = match answer {
            Ok(a) => a,
            Err(Fail::Timeout) => {
                return Err(AiError("OpenRouter didn't answer in time.".to_owned()));
            }
            Err(Fail::Io(e)) => return Err(AiError(format!("Connection problem: {e}"))),
            Err(Fail::Ai(e)) => return Err(AiError(e)),
            Err(Fail::Json(e) | Fail::Other(e)) => {
                return Err(AiError(format!("Unexpected problem: {e}")));
            }
        };
        if status == 404 {
            continue;
        }
        if status == 401 || status == 403 {
            return Err(AiError(
                "OpenRouter didn't accept this API key. Check that you copied all of it."
                    .to_owned(),
            ));
        }
        if status != 200 {
            return Err(AiError(http_error_text(status, &body, endpoint, "")));
        }
        let data = serde_json::from_str::<Value>(&text_of(&body))
            .ok()
            .and_then(|v| match v {
                Value::Object(mut m) => m.remove("data"),
                _ => None,
            });
        let left = match &data {
            Some(Value::Object(m)) => m.get("limit_remaining").and_then(Value::as_f64),
            _ => None,
        };
        return Ok(match left {
            Some(left) => format!("Key accepted (${left:.2} left of its spending limit)."),
            None => "Key accepted.".to_owned(),
        });
    }
    Ok(String::new())
}

/// What "Test connection" found (`AiCheckWorker.result`).
#[derive(Debug, Clone, Default)]
pub struct CheckResult {
    pub provider: String,
    pub ids: Vec<String>,
    pub pricing: Pricing,
    pub error: String,
    pub key_note: String,
}

/// `AiCheckWorker.run`: checks the key (OpenRouter) and lists the models. Blocks; run
/// it on a worker thread.
pub fn run_check(
    provider: &str,
    base_url: &str,
    key: &str,
    headers: &[(String, String)],
) -> CheckResult {
    let mut info = CheckResult {
        provider: provider.to_owned(),
        ..Default::default()
    };
    let result = (|| -> Result<(), AiError> {
        if provider == "openrouter" {
            info.key_note = check_openrouter_key(base_url, key, headers, Duration::from_secs(10))?;
        }
        let timeout = Duration::from_secs(if provider == "openrouter" { 15 } else { 6 });
        (info.ids, info.pricing) = list_models(base_url, key, timeout, headers)?;
        Ok(())
    })();
    if let Err(e) = result {
        info.error = e.0;
    }
    info
}

// ---------------------------------------------------------------------------------------
// The alignment request
// ---------------------------------------------------------------------------------------

/// A big model on a busy GPU can take minutes for the first token.
pub const ALIGN_READ_TIMEOUT: Duration = Duration::from_secs(600);
const ALIGN_CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

/// What the alignment worker reports (`AiAlignWorker`'s signals).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlignEvent {
    /// "waiting", "thinking" or "answering", and the answer lines so far.
    Progress {
        phase: &'static str,
        answered: usize,
    },
    /// The model's whole answer and why it stopped.
    Done { answer: String, finish: String },
    /// A message for the user, and the answer received so far.
    Failed { message: String, raw: String },
}

/// One alignment request.
#[derive(Debug, Clone)]
pub struct AlignRequest {
    pub base_url: String,
    pub model: String,
    pub key: String,
    pub messages: Vec<Message>,
    pub total_rows: usize,
    pub headers: Vec<(String, String)>,
}

enum ChatFail {
    /// `_MaxTokensRejected`.
    MaxTokens,
    Fail(Fail),
}

impl<T: Into<Fail>> From<T> for ChatFail {
    fn from(e: T) -> Self {
        ChatFail::Fail(e.into())
    }
}

/// The phase shown while the answer streams in.
pub fn progress_phase(raw: &str, thinking: bool) -> (&'static str, usize) {
    let answered = align::count_answer_lines(raw);
    let low = raw.to_lowercase();
    let in_think = low.contains("<think>") && !low.contains("</think>");
    let phase = if answered > 0 {
        "answering"
    } else if thinking || in_think {
        "thinking"
    } else {
        "waiting"
    };
    (phase, answered)
}

fn str_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn chat(
    req: &AlignRequest,
    model: &str,
    with_max_tokens: bool,
    cancel: &Cancel,
    raw: &mut String,
    emit: &mut dyn FnMut(AlignEvent),
) -> Result<(String, String), ChatFail> {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(|m| json!({"role": m.role, "content": m.content}))
        .collect();
    let mut body = json!({
        "model": model,
        "messages": messages,
        "temperature": 0.2,
        "stream": true,
    });
    if with_max_tokens {
        // Room for models that think first.
        body["max_tokens"] = json!(6000 + 32 * req.total_rows);
    }
    let (conn, path) = ai_open(
        &req.base_url,
        ALIGN_CONNECT_TIMEOUT,
        ALIGN_READ_TIMEOUT,
        Some(cancel),
    )?;
    if cancel.is_cancelled() {
        return Ok((String::new(), "cancelled".to_owned()));
    }
    emit(AlignEvent::Progress {
        phase: "waiting",
        answered: 0,
    });
    let payload = body.to_string();
    let mut resp = conn.request(
        "POST",
        &format!("{path}/chat/completions"),
        &ai_headers(&req.key, &req.headers),
        Some(payload.as_bytes()),
    )?;
    if resp.status != 200 {
        let err = resp.read_all()?;
        let low = text_of(&err).to_lowercase();
        if with_max_tokens
            && (resp.status == 400 || resp.status == 422)
            && (low.contains("max_tokens") || low.contains("context"))
        {
            return Err(ChatFail::MaxTokens);
        }
        return Err(Fail::Ai(http_error_text(
            resp.status,
            &err,
            "/chat/completions",
            model,
        ))
        .into());
    }

    let content_type = resp.header("Content-Type").unwrap_or_default();
    if !content_type.to_lowercase().contains("text/event-stream") {
        // The server ignored "stream".
        let all = resp.read_all()?;
        let data: Value =
            serde_json::from_str(&text_of(&all)).map_err(|e| Fail::Other(e.to_string()))?;
        let choice = match data.get("choices") {
            Some(Value::Array(a)) if !a.is_empty() => a[0].clone(),
            _ => json!({}),
        };
        let message = choice.get("message").filter(|m| truthy(Some(m)));
        let content = message.and_then(|m| m.get("content"));
        *raw = match first_truthy(&[content, choice.get("text")]) {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        let finish = str_of(first_truthy(&[choice.get("finish_reason")]));
        return Ok((raw.clone(), finish));
    }

    let mut content = String::new();
    let mut finish = String::new();
    let mut thinking = false;
    let mut last_emit: Option<Instant> = None;
    while !cancel.is_cancelled() {
        let line = resp.read_line()?;
        if line.is_empty() {
            break;
        }
        let line = text_of(&line);
        let line = py::strip(&line);
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        let payload = py::strip(payload);
        if payload == "[DONE]" {
            break;
        }
        let Ok(obj) = serde_json::from_str::<Value>(payload) else {
            continue;
        };
        if let Some(err) = obj.get("error").filter(|e| truthy(Some(e))) {
            let text = match err {
                Value::Object(m) => m.get("message").map_or_else(|| err.to_string(), py_str),
                other => py_str(other),
            };
            return Err(Fail::Ai(format!("The model reported an error: {text}")).into());
        }
        if let Some(Value::Array(choices)) = obj.get("choices") {
            for choice in choices {
                let delta = first_truthy(&[choice.get("delta"), choice.get("message")]);
                let piece =
                    first_truthy(&[delta.and_then(|d| d.get("content")), choice.get("text")]);
                if let Some(d) = delta
                    && first_truthy(&[d.get("reasoning_content"), d.get("reasoning")]).is_some()
                {
                    thinking = true;
                }
                if let Some(Value::String(piece)) = piece {
                    content.push_str(piece);
                }
                if let Some(f) = first_truthy(&[choice.get("finish_reason")]) {
                    finish = py_str(f);
                }
            }
        }
        let now = Instant::now();
        if last_emit.is_none_or(|t| now.duration_since(t) > Duration::from_millis(150)) {
            last_emit = Some(now);
            raw.clone_from(&content);
            let (phase, answered) = progress_phase(raw, thinking);
            emit(AlignEvent::Progress { phase, answered });
        }
    }
    raw.clone_from(&content);
    let finish = if cancel.is_cancelled() {
        "cancelled".to_owned()
    } else {
        finish
    };
    Ok((content, finish))
}

/// `AiAlignWorker.run`: sends the alignment request and streams the answer back
/// through `emit`. Blocks; run it on a worker thread. Once cancelled, it reports
/// nothing more.
pub fn run_alignment(req: &AlignRequest, cancel: &Cancel, mut emit: impl FnMut(AlignEvent)) {
    let mut raw = String::new();
    let mut guarded = |e: AlignEvent| {
        if !cancel.is_cancelled() {
            emit(e);
        }
    };
    let result = (|| -> Result<(String, String), Fail> {
        let mut model = req.model.clone();
        if model.is_empty() {
            let (ids, _) = list_models(
                &req.base_url,
                &req.key,
                Duration::from_secs(6),
                &req.headers,
            )?;
            model = ids
                .into_iter()
                .next()
                .unwrap_or_else(|| "local-model".to_owned());
        }
        match chat(req, &model, true, cancel, &mut raw, &mut guarded) {
            Ok(r) => Ok(r),
            Err(ChatFail::MaxTokens) => {
                match chat(req, &model, false, cancel, &mut raw, &mut guarded) {
                    Ok(r) => Ok(r),
                    Err(ChatFail::Fail(f)) => Err(f),
                    // Not asked for max_tokens, so never rejected for it.
                    Err(ChatFail::MaxTokens) => Err(Fail::Other("max_tokens".to_owned())),
                }
            }
            Err(ChatFail::Fail(f)) => Err(f),
        }
    })();
    cancel.forget();
    let event = match result {
        Ok((answer, finish)) => AlignEvent::Done { answer, finish },
        Err(Fail::Ai(message)) => AlignEvent::Failed { message, raw },
        Err(Fail::Timeout) => AlignEvent::Failed {
            message: "The model stopped answering (no reply for 10 minutes).".to_owned(),
            raw,
        },
        Err(Fail::Io(e) | Fail::Json(e) | Fail::Other(e)) => AlignEvent::Failed {
            message: format!("Connection problem: {e}"),
            raw,
        },
    };
    guarded(event);
}

// ---------------------------------------------------------------------------------------
// Settings and keys
// ---------------------------------------------------------------------------------------

/// Windows' per-user encryption (DPAPI), which the player supplies on Windows.
pub trait Cipher {
    fn protect(&self, data: &[u8]) -> Option<Vec<u8>>;
    fn unprotect(&self, data: &[u8]) -> Option<Vec<u8>>;
}

const SECRET_PREFIX: &str = "dpapi:";

fn b64_engine() -> base64::engine::GeneralPurpose {
    use base64::engine::{GeneralPurpose, GeneralPurposeConfig};
    // Python's b64decode accepts stray bits after the last full byte.
    GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
    )
}

/// `unprotect_secret`: a stored key as text. "" when it can't be decrypted (another
/// Windows account, or no DPAPI); a value without `dpapi:` is plain text.
pub fn unprotect_secret(stored: &str, cipher: Option<&dyn Cipher>) -> String {
    use base64::Engine;
    if stored.is_empty() {
        return String::new();
    }
    let Some(encoded) = stored.strip_prefix(SECRET_PREFIX) else {
        return stored.to_owned(); // stored before encryption existed, or none available
    };
    // `b64decode` without `validate` skips characters outside the alphabet.
    let cleaned: String = encoded
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
        .collect();
    let Some(cipher) = cipher else {
        return String::new();
    };
    b64_engine()
        .decode(cleaned)
        .ok()
        .and_then(|blob| cipher.unprotect(&blob))
        .and_then(|plain| String::from_utf8(plain).ok())
        .unwrap_or_default()
}

/// `protect_secret`: an API key as it is stored, encrypted for this Windows account
/// when that works (and reads back), otherwise as it is.
pub fn protect_secret(text: &str, cipher: Option<&dyn Cipher>) -> String {
    use base64::Engine;
    if text.is_empty() {
        return String::new();
    }
    if let Some(c) = cipher
        && let Some(blob) = c.protect(text.as_bytes())
    {
        let stored = format!("{SECRET_PREFIX}{}", b64_engine().encode(blob));
        if unprotect_secret(&stored, cipher) == text {
            return stored; // only trusted once it reads back
        }
    }
    text.to_owned()
}

/// `secrets_are_encrypted`.
pub fn secrets_are_encrypted(cipher: Option<&dyn Cipher>) -> bool {
    protect_secret("check", cipher).starts_with(SECRET_PREFIX)
}

/// The AI settings as stored (`ai_settings_raw`), keys decrypted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawSettings {
    pub enabled: bool,
    /// "local" or "openrouter".
    pub provider: &'static str,
    pub url: String,
    pub model: String,
    pub key: String,
    pub or_model: String,
    pub or_key: String,
}

pub fn raw_settings(s: &Settings, cipher: Option<&dyn Cipher>) -> RawSettings {
    RawSettings {
        enabled: s.get_bool("ai/enabled", false),
        provider: if s.get_str("ai/provider", "local") == "openrouter" {
            "openrouter"
        } else {
            "local"
        },
        url: normalize_ai_url(&s.get_str("ai/url", "")),
        model: s.get_str("ai/model", ""),
        key: unprotect_secret(&s.get_str("ai/key", ""), cipher),
        or_model: s.get_str("ai/or_model", ""),
        or_key: unprotect_secret(&s.get_str("ai/or_key", ""), cipher),
    }
}

/// The settings that apply now, for whichever provider is chosen (`ai_settings`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub provider: &'static str,
    pub url: String,
    pub model: String,
    pub key: String,
    /// Where the model runs, for the button's tooltip.
    pub where_: String,
    pub headers: Vec<(String, String)>,
    pub ready: bool,
    pub enabled: bool,
}

/// The header OpenRouter shows apps by.
pub fn openrouter_headers() -> Vec<(String, String)> {
    vec![("X-Title".to_owned(), crate::APP_TITLE.to_owned())]
}

pub fn config(raw: &RawSettings) -> Config {
    if raw.provider == "openrouter" {
        Config {
            provider: "openrouter",
            url: OPENROUTER_BASE.to_owned(),
            model: raw.or_model.clone(),
            key: raw.or_key.clone(),
            where_: "OpenRouter".to_owned(),
            headers: openrouter_headers(),
            ready: raw.enabled && !raw.or_key.is_empty() && !raw.or_model.is_empty(),
            enabled: raw.enabled,
        }
    } else {
        Config {
            provider: "local",
            url: raw.url.clone(),
            model: raw.model.clone(),
            key: raw.key.clone(),
            where_: raw.url.clone(),
            headers: Vec::new(),
            ready: raw.enabled && !raw.url.is_empty(),
            enabled: raw.enabled,
        }
    }
}

/// `save_ai_settings`. Returns true if anything changed.
pub fn save_settings(s: &mut Settings, raw: &RawSettings, cipher: Option<&dyn Cipher>) -> bool {
    let mut changed = false;
    changed |= s.set("ai/enabled", raw.enabled);
    changed |= s.set("ai/provider", raw.provider);
    changed |= s.set("ai/url", raw.url.as_str());
    changed |= s.set("ai/model", raw.model.as_str());
    changed |= s.set("ai/key", protect_secret(&raw.key, cipher));
    changed |= s.set("ai/or_model", raw.or_model.as_str());
    changed |= s.set("ai/or_key", protect_secret(&raw.or_key, cipher));
    changed
}

// ---------------------------------------------------------------------------------------
// The texts of the AI settings window (`AiSettingsDialog`)
// ---------------------------------------------------------------------------------------

pub const LOCAL_HINT: &str = "Works with any server that offers an OpenAI-compatible API, such as LM Studio (port 1234), Ollama (11434), llama.cpp (8080) or KoboldCpp (5001). Lyrics are sent only to this address.";

/// The hint under the settings for OpenRouter.
pub fn openrouter_hint(encrypted: bool) -> String {
    let mut s = "Lyrics and the translation are sent to OpenRouter and to the company that runs the model you choose. OpenRouter charges per use; the estimate is for a typical song.".to_owned();
    if encrypted {
        s.push_str(" Your key is stored encrypted for your Windows account.");
    }
    s
}

/// `_update_cost`: the line under the OpenRouter model, and whether it is an error.
pub fn cost_text(model: &str, models: &[String], pricing: &Pricing) -> (String, bool) {
    let model = py::strip(model);
    if model.is_empty() || models.is_empty() {
        return (String::new(), false);
    }
    if !models.iter().any(|m| m == model) {
        let low = model.to_lowercase();
        let matches = models
            .iter()
            .filter(|m| m.to_lowercase().contains(&low))
            .count();
        if matches > 0 {
            // Still typing: point at the list instead of complaining.
            let s = if matches != 1 { "s" } else { "" };
            return (
                format!("{matches} matching model{s}: pick one from the list."),
                false,
            );
        }
        return (format!("OpenRouter has no model called “{model}”."), true);
    }
    let Some(&(p_in, p_out)) = pricing.get(model) else {
        return (String::new(), false);
    };
    // A typical song: ~3k tokens in, up to ~1.5k out.
    let cost = 3000.0 * p_in + 1500.0 * p_out;
    if cost == 0.0 {
        return (
            "Free model: usually slower, and limited to a few requests a minute.".to_owned(),
            false,
        );
    }
    if cost < 0.01 {
        (format!("About ${cost:.4} per song"), false)
    } else {
        (format!("About ${cost:.3} per song"), false)
    }
}

/// The models OpenRouter offers that contain `typed` (`QCompleter` with
/// `MatchContains`, case-insensitive), in the server's order.
pub fn matching_models<'a>(typed: &str, models: &'a [String]) -> Vec<&'a String> {
    let low = typed.to_lowercase();
    models
        .iter()
        .filter(|m| m.to_lowercase().contains(&low))
        .collect()
}

/// What a local "Test connection" means (`_on_test_result` for a local server): the
/// status, whether it is an error, and the model to put in the Model field.
pub fn local_test_result(ids: &[String], current: &str) -> (String, bool, Option<String>) {
    let current = py::strip(current);
    let known = ids.iter().any(|m| m == current);
    let fill = (!ids.is_empty() && (current.is_empty() || (!known && ids.len() == 1)))
        .then(|| ids[0].clone());
    if ids.is_empty() {
        return (
            "Connected, but the server didn't list any models. Type the model name yourself."
                .to_owned(),
            false,
            fill,
        );
    }
    if !current.is_empty() && !known && ids.len() > 1 {
        return (
            format!(
                "Connected, but “{current}” isn't one of its {} models. Pick one with Choose.",
                ids.len()
            ),
            true,
            fill,
        );
    }
    let s = if ids.len() != 1 { "s" } else { "" };
    (
        format!("Connected. {} model{s} available.", ids.len()),
        false,
        fill,
    )
}

/// The status after a successful OpenRouter "Test connection".
pub fn openrouter_test_result(key_note: &str, n_models: usize) -> String {
    let note = if key_note.is_empty() {
        "Connected."
    } else {
        key_note
    };
    format!("{note} {n_models} models to choose from: type in Model to search them.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_addresses() {
        assert_eq!(
            normalize_ai_url("localhost:1234"),
            "http://localhost:1234/v1"
        );
        assert_eq!(
            normalize_ai_url(" http://127.0.0.1:1234/v1/chat/completions/ "),
            "http://127.0.0.1:1234/v1"
        );
        assert_eq!(normalize_ai_url("http://x:11434/api"), "http://x:11434/v1");
        assert_eq!(normalize_ai_url(""), "");
        assert_eq!(
            normalize_ai_url("HTTPS://Host/v1/models?x=1"),
            "https://Host/v1/models"
        );
    }

    struct Rot;
    impl Cipher for Rot {
        fn protect(&self, d: &[u8]) -> Option<Vec<u8>> {
            Some(d.iter().map(|b| b.wrapping_add(1)).collect())
        }
        fn unprotect(&self, d: &[u8]) -> Option<Vec<u8>> {
            Some(d.iter().map(|b| b.wrapping_sub(1)).collect())
        }
    }

    #[test]
    fn secrets() {
        let stored = protect_secret("sk-1", Some(&Rot));
        assert!(stored.starts_with("dpapi:"));
        assert_eq!(unprotect_secret(&stored, Some(&Rot)), "sk-1");
        assert_eq!(unprotect_secret(&stored, None), "");
        assert_eq!(protect_secret("sk-1", None), "sk-1");
        assert_eq!(unprotect_secret("plain", None), "plain");
        assert_eq!(unprotect_secret("dpapi:!!!", Some(&Rot)), "");
        assert!(secrets_are_encrypted(Some(&Rot)));
        assert!(!secrets_are_encrypted(None));
    }

    #[test]
    fn costs() {
        let models = vec![
            "a/qwen".to_owned(),
            "b/qwen-big".to_owned(),
            "free".to_owned(),
        ];
        let mut pricing = Pricing::new();
        pricing.insert("a/qwen".to_owned(), (0.000001, 0.000002));
        pricing.insert("free".to_owned(), (0.0, 0.0));
        pricing.insert("b/qwen-big".to_owned(), (0.00001, 0.00002));
        assert_eq!(
            cost_text("a/qwen", &models, &pricing).0,
            "About $0.0060 per song"
        );
        assert_eq!(
            cost_text("b/qwen-big", &models, &pricing).0,
            "About $0.060 per song"
        );
        assert!(
            cost_text("free", &models, &pricing)
                .0
                .starts_with("Free model")
        );
        assert_eq!(
            cost_text("QWEN", &models, &pricing).0,
            "2 matching models: pick one from the list."
        );
        assert_eq!(
            cost_text("zzz", &models, &pricing),
            ("OpenRouter has no model called “zzz”.".to_owned(), true)
        );
        assert_eq!(cost_text("", &models, &pricing).0, "");
    }

    #[test]
    fn local_results() {
        let ids = vec!["m1".to_owned(), "m2".to_owned()];
        assert_eq!(local_test_result(&ids, "").2, Some("m1".to_owned()));
        assert!(local_test_result(&ids, "zz").1);
        assert_eq!(
            local_test_result(&ids[..1], "zz"),
            (
                "Connected. 1 model available.".to_owned(),
                false,
                Some("m1".to_owned())
            )
        );
    }
}
