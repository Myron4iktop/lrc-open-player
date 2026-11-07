//! The AI client against a small fake server on 127.0.0.1: model lists, the key
//! check's errors, streamed and unstreamed answers, the retry without `max_tokens`,
//! and a cancel that closes the socket.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use lrc_core::ai::{self, AlignEvent, AlignRequest, Cancel};
use lrc_core::align::Message;

/// What the fake server does with one connection.
type Handler = Box<dyn FnOnce(&mut TcpStream) + Send>;

/// Reads one request: the request line, headers and body.
fn read_request(stream: &mut TcpStream) -> (String, String) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    let mut length = 0;
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).unwrap();
        if h.trim().is_empty() {
            break;
        }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
            length = v.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    (first.trim().to_owned(), String::from_utf8(body).unwrap())
}

/// A server that answers each connection with the next handler; returns its address
/// and what each request was.
fn serve(handlers: Vec<Handler>) -> (String, mpsc::Receiver<(String, String)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}/v1", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for handler in handlers {
            let (mut stream, _) = listener.accept().unwrap();
            let req = read_request(&mut stream);
            tx.send(req).unwrap();
            handler(&mut stream);
        }
    });
    (addr, rx)
}

fn reply(status: &str, content_type: &str, body: &str) -> Handler {
    let text = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    Box::new(move |s: &mut TcpStream| {
        s.write_all(text.as_bytes()).unwrap();
    })
}

fn chunked_sse(events: Vec<String>) -> Handler {
    Box::new(move |s: &mut TcpStream| {
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n")
            .unwrap();
        for e in events {
            let data = format!("data: {e}\n\n");
            s.write_all(format!("{:x}\r\n{data}\r\n", data.len()).as_bytes())
                .unwrap();
        }
        s.write_all(b"0\r\n\r\n").unwrap();
    })
}

fn request(url: &str, model: &str) -> AlignRequest {
    AlignRequest {
        base_url: url.to_owned(),
        model: model.to_owned(),
        key: "k".to_owned(),
        messages: vec![Message {
            role: "user",
            content: "hi".to_owned(),
        }],
        total_rows: 2,
        headers: vec![("X-Title".to_owned(), "LRC Open Player".to_owned())],
    }
}

fn delta(text: &str) -> String {
    serde_json::json!({"choices": [{"delta": {"content": text}}]}).to_string()
}

#[test]
fn lists_models_with_prices() {
    let body = r#"{"data": [{"id": "a", "pricing": {"prompt": "0.000001", "completion": "0.000002"}}, {"name": "b"}, {"id": "a"}, "c"]}"#;
    let (url, rx) = serve(vec![reply("200 OK", "application/json", body)]);
    let (ids, pricing) = ai::list_models(&url, "key", Duration::from_secs(5), &[]).unwrap();
    assert_eq!(ids, ["a", "b", "c"]);
    assert_eq!(pricing["a"], (0.000001, 0.000002));
    let (line, _) = rx.recv().unwrap();
    assert!(line.starts_with("GET /v1/models HTTP/1.1"), "{line}");
}

#[test]
fn model_list_errors_are_worded_like_python() {
    let (url, _rx) = serve(vec![
        reply("401 Unauthorized", "application/json", "{}"),
        reply("200 OK", "text/html", "<html>"),
    ]);
    let e = ai::list_models(&url, "", Duration::from_secs(5), &[]).unwrap_err();
    assert_eq!(
        e.0,
        "The server refused the request (HTTP 401). Check the API key."
    );
    let e = ai::list_models(&url, "", Duration::from_secs(5), &[]).unwrap_err();
    assert_eq!(
        e.0,
        "The server's answer wasn't valid JSON. Is this an OpenAI-compatible API?"
    );
}

#[test]
fn nothing_listening() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let e = ai::list_models(
        &format!("http://127.0.0.1:{port}/v1"),
        "",
        // Windows takes about two seconds to report a refused connection to localhost
        // (it retries first), so a 2 s timeout raced it and read as "didn't respond".
        Duration::from_secs(10),
        &[],
    )
    .unwrap_err();
    assert_eq!(
        e.0,
        format!("Nothing is answering at 127.0.0.1:{port}. Is the model server running?")
    );
    let e = ai::list_models("ftp://x/v1", "", Duration::from_secs(2), &[]).unwrap_err();
    assert_eq!(e.0, "“ftp://x/v1” isn't a valid server address.");
}

#[test]
fn key_check_falls_back_and_reads_the_limit() {
    let (url, rx) = serve(vec![
        reply("404 Not Found", "text/plain", "no"),
        reply(
            "200 OK",
            "application/json",
            r#"{"data": {"limit_remaining": 4.256}}"#,
        ),
    ]);
    let note = ai::check_openrouter_key(&url, "k", &[], Duration::from_secs(5)).unwrap();
    assert_eq!(note, "Key accepted ($4.26 left of its spending limit).");
    assert!(rx.recv().unwrap().0.contains("/v1/key "));
    assert!(rx.recv().unwrap().0.contains("/v1/auth/key "));
}

#[test]
fn streams_an_answer() {
    let (url, rx) = serve(vec![chunked_sse(vec![
        serde_json::json!({"choices": [{"delta": {"reasoning_content": "hmm"}}]}).to_string(),
        delta("O1: T1\n"),
        "not json".to_owned(),
        delta("O2: none"),
        serde_json::json!({"choices": [{"delta": {}, "finish_reason": "stop"}]}).to_string(),
        "[DONE]".to_owned(),
    ])]);
    let mut events = Vec::new();
    ai::run_alignment(&request(&url, "m"), &Cancel::new(), |e| events.push(e));
    assert_eq!(
        events.first(),
        Some(&AlignEvent::Progress {
            phase: "waiting",
            answered: 0
        })
    );
    assert_eq!(
        events.last(),
        Some(&AlignEvent::Done {
            answer: "O1: T1\nO2: none".to_owned(),
            finish: "stop".to_owned()
        })
    );
    let (line, body) = rx.recv().unwrap();
    assert!(line.starts_with("POST /v1/chat/completions"));
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["max_tokens"], 6064);
    assert_eq!(body["stream"], true);
    assert_eq!(body["temperature"], 0.2);
}

#[test]
fn takes_an_unstreamed_answer_and_retries_without_max_tokens() {
    let (url, rx) = serve(vec![
        reply(
            "400 Bad Request",
            "application/json",
            r#"{"error": "max_tokens is too large"}"#,
        ),
        reply(
            "200 OK",
            "application/json",
            r#"{"choices": [{"message": {"content": "O1: T1"}, "finish_reason": "length"}]}"#,
        ),
    ]);
    let mut events = Vec::new();
    ai::run_alignment(&request(&url, "m"), &Cancel::new(), |e| events.push(e));
    assert_eq!(
        events.last(),
        Some(&AlignEvent::Done {
            answer: "O1: T1".to_owned(),
            finish: "length".to_owned()
        })
    );
    let first: serde_json::Value = serde_json::from_str(&rx.recv().unwrap().1).unwrap();
    let second: serde_json::Value = serde_json::from_str(&rx.recv().unwrap().1).unwrap();
    assert!(first.get("max_tokens").is_some());
    assert!(second.get("max_tokens").is_none());
}

#[test]
fn asks_for_the_first_model_when_none_is_set() {
    let (url, rx) = serve(vec![
        reply(
            "200 OK",
            "application/json",
            r#"{"data": [{"id": "only"}]}"#,
        ),
        reply(
            "404 Not Found",
            "application/json",
            r#"{"error": {"message": "model 'only' not found"}}"#,
        ),
    ]);
    let mut events = Vec::new();
    ai::run_alignment(&request(&url, ""), &Cancel::new(), |e| events.push(e));
    rx.recv().unwrap();
    let body: serde_json::Value = serde_json::from_str(&rx.recv().unwrap().1).unwrap();
    assert_eq!(body["model"], "only");
    assert_eq!(
        events.last(),
        Some(&AlignEvent::Failed {
            message: "The server doesn't have the model “only” loaded: model 'only' not found"
                .to_owned(),
            raw: String::new()
        })
    );
}

#[test]
fn cancel_closes_the_socket() {
    let (gone_tx, gone_rx) = mpsc::channel();
    let (url, _rx) = serve(vec![Box::new(move |s: &mut TcpStream| {
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n")
            .unwrap();
        s.write_all(format!("data: {}\n\n", delta("O1: T1\n")).as_bytes())
            .unwrap();
        // Waits for the client to hang up, as a server generating slowly would notice.
        let mut buf = [0u8; 16];
        let _ = s.set_read_timeout(Some(Duration::from_secs(20)));
        let n = s.read(&mut buf).unwrap_or(1);
        gone_tx.send(n).unwrap();
    })]);
    let cancel = Cancel::new();
    let c2 = cancel.clone();
    let (ev_tx, ev_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        ai::run_alignment(&request(&url, "m"), &c2, |e| ev_tx.send(e).unwrap());
    });
    // The first progress arrives, then the user cancels.
    assert!(matches!(
        ev_rx.recv_timeout(Duration::from_secs(5)),
        Ok(AlignEvent::Progress { .. })
    ));
    thread::sleep(Duration::from_millis(100));
    let start = Instant::now();
    cancel.cancel();
    worker.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(3));
    assert_eq!(
        gone_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        0,
        "the server saw the socket close"
    );
    // Nothing is reported once cancelled.
    while let Ok(e) = ev_rx.try_recv() {
        assert!(matches!(e, AlignEvent::Progress { .. }), "{e:?}");
    }
}
