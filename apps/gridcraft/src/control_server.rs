//! Localhost JSON-lines control server. Each connection may send many requests; each line
//! `{"id":…, "method":…, "params":…}` gets one reply line `{"id":…, "ok":…, …}`.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use gridcraft_ui_egui::ControlRequest;
use serde_json::{Value as Json, json};

const MAX_LINE: usize = 64 * 1024 * 1024;

pub fn start(port: u16, ctx: egui::Context) -> std::io::Result<Receiver<ControlRequest>> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let (tx, rx) = channel();
    std::thread::Builder::new().name("control".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let tx = tx.clone();
            let ctx = ctx.clone();
            let _ = std::thread::Builder::new().name("control-conn".into()).spawn(move || serve(stream, tx, ctx));
        }
    })?;
    log::info!("control channel listening on 127.0.0.1:{port}");
    Ok(rx)
}

fn serve(stream: TcpStream, tx: Sender<ControlRequest>, ctx: egui::Context) {
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    let reader = BufReader::new(read);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > MAX_LINE {
            let _ = writeln!(out, "{}", json!({"ok": false, "error": "request too large"}));
            continue;
        }
        let msg: Json = match serde_json::from_str(&line) {
            Ok(v) => v,
            // Close on a line that isn't JSON: an HTTP request (say, a web page's `fetch` to this
            // port) starts with a request line that never parses, so it can't carry a command in
            // its body. Matches WordCraft's and LightCraft's control servers.
            Err(e) => {
                let _ = writeln!(out, "{}", json!({"ok": false, "error": format!("invalid JSON: {e}; closing the connection")}));
                break;
            }
        };
        let id = msg.get("id").cloned().unwrap_or(Json::Null);
        let method = msg.get("method").and_then(Json::as_str).unwrap_or("").to_string();
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        let (req, rx) = ControlRequest::new(method, params);
        if tx.send(req).is_err() {
            break;
        }
        // Wake the UI thread: egui only runs frames when something happens.
        ctx.request_repaint();
        let mut reply = rx
            .recv_timeout(Duration::from_secs(60))
            .unwrap_or_else(|_| json!({"ok": false, "error": "timed out waiting for the UI (is the window minimised?)"}));
        if let Some(o) = reply.as_object_mut() {
            o.insert("id".into(), id);
        }
        if writeln!(out, "{reply}").is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// A listener on a free loopback port whose connections go to `serve`, plus the request queue.
    fn boot() -> (u16, Receiver<ControlRequest>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                std::thread::spawn(move || serve(stream, tx, egui::Context::default()));
            }
        });
        (port, rx)
    }

    fn connect(port: u16) -> TcpStream {
        let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s
    }

    /// Everything the server sends until it closes the connection (or the read times out).
    fn read_to_close(s: &mut TcpStream) -> (String, bool) {
        let mut buf = Vec::new();
        let closed = s.read_to_end(&mut buf).is_ok();
        (String::from_utf8_lossy(&buf).into_owned(), closed)
    }

    #[test]
    fn json_requests_reach_the_ui() {
        let (port, rx) = boot();
        let mut s = connect(port);
        writeln!(s, r#"{{"id":1,"method":"ping","params":{{}}}}"#).unwrap();
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(req.method, "ping");
    }

    #[test]
    fn bad_json_closes_the_connection() {
        let (port, rx) = boot();
        let mut s = connect(port);
        writeln!(s, "not json").unwrap();
        let (out, closed) = read_to_close(&mut s);
        assert!(out.contains("closing the connection"), "{out}");
        assert!(closed, "connection should be closed after a non-JSON line");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn http_request_cannot_smuggle_a_command() {
        // What a web page's `fetch("http://127.0.0.1:<port>/", {method: "POST", body})` sends.
        let (port, rx) = boot();
        let body = "\n{\"id\":1,\"method\":\"file.save_as\",\"params\":{\"path\":\"/tmp/x.xlsx\"}}\n";
        let req = format!("POST / HTTP/1.1\r\nHost: 127.0.0.1:7979\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        let mut s = connect(port);
        s.write_all(req.as_bytes()).unwrap();
        let (_, closed) = read_to_close(&mut s);
        assert!(closed);
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "the JSON body line must not run");
    }
}
