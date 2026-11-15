//! `--serve`: the HTML report on 127.0.0.1, with route cards that can be renamed.
//!
//! One connection at a time, on the standard library alone: the page is only
//! for whoever runs it. A rename must carry the `X-Digirun` header, which another
//! site's page cannot send without a CORS preflight that is never answered, and a
//! local `Host`, so a DNS name pointed at 127.0.0.1 is turned away too.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use crate::args::Args;
use crate::dt::Date;
use crate::html;
use crate::metrics::Metrics;
use crate::naming::{Change, RecentRoute, recent_routes, save};

/// A rename is a route index and a name, so its body stays small.
const MAX_BODY: usize = 4096;
const RENAME_HEADER: (&str, &str) = ("x-digirun", "rename");

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Request {
    pub method: String,
    pub path: String,
    /// Names in lower case.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Response {
    fn text(status: u16, body: &str) -> Response {
        Response {
            status,
            content_type: "text/plain; charset=utf-8",
            body: body.as_bytes().to_vec(),
        }
    }
}

/// What the server holds between requests.
pub struct State<'m, 'a> {
    pub metrics: &'m mut Metrics<'a>,
    pub args: Args,
    /// The names file renames are saved to.
    pub path: PathBuf,
    pub reference: Date,
    /// Every route, with its automatic name.
    routes: Vec<RecentRoute>,
}

impl<'m, 'a> State<'m, 'a> {
    pub fn new(
        metrics: &'m mut Metrics<'a>,
        args: &Args,
        path: PathBuf,
        reference: Date,
    ) -> State<'m, 'a> {
        let every = Args {
            start: None,
            end: None,
            ..args.clone()
        };
        let routes = recent_routes(metrics, &every);
        State {
            metrics,
            args: Args {
                serve: true,
                ..args.clone()
            },
            path,
            reference,
            routes,
        }
    }
}

pub fn handle(request: &Request, state: &mut State) -> Response {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => Response {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: html::render(state.metrics, &state.args, state.reference).into_bytes(),
        },
        ("POST", "/route-name") => rename(request, state),
        _ => Response::text(404, "Not found"),
    }
}

fn rename(request: &Request, state: &mut State) -> Response {
    let port = state.args.port;
    let local = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    let host_ok = request
        .header("host")
        .is_some_and(|h| local.iter().any(|l| l == h));
    if !host_ok || request.header(RENAME_HEADER.0) != Some(RENAME_HEADER.1) {
        return Response::text(403, "Forbidden");
    }
    let Some(form) = std::str::from_utf8(&request.body)
        .ok()
        .and_then(decode_form)
    else {
        return Response::text(400, "The form could not be read.");
    };
    let field = |key: &str| form.iter().find(|(k, _)| k == key).map(|(_, v)| v.trim());
    let Some(index) = field("route").and_then(|r| r.parse::<usize>().ok()) else {
        return Response::text(400, "No such route.");
    };
    let Some(route) = state.routes.iter().find(|r| r.index == index).cloned() else {
        return Response::text(400, "No such route.");
    };
    let name = field("name").unwrap_or_default();
    if name.contains(['#', '\n', '\r']) {
        return Response::text(
            400,
            "A name can't contain # or a line break, as routes.txt would read it differently.",
        );
    }
    let name = (!name.is_empty()).then(|| name.to_string());
    let change = Change {
        route: index,
        name: name.clone(),
    };
    if let Err(error) = save(&state.path, &[change], state.metrics, &state.routes) {
        eprintln!("error: {error}");
        return Response::text(500, &format!("Could not save: {error}"));
    }
    let shown = name.unwrap_or_else(|| route.auto.clone());
    for session in &mut state.metrics.sessions {
        if session.route == Some(index) {
            session.route_name.clone_from(&shown);
        }
    }
    println!("{} is now {shown}", route.auto);
    if let Some(path) = &state.args.html
        && let Err(error) = html::write_report(path, state.metrics, &state.args, state.reference)
    {
        eprintln!("error: {error}");
    }
    Response {
        status: 204,
        content_type: "text/plain; charset=utf-8",
        body: Vec::new(),
    }
}

/// `application/x-www-form-urlencoded` pairs, or None when not valid UTF-8 or escapes.
pub fn decode_form(body: &str) -> Option<Vec<(String, String)>> {
    body.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            Some((decode_component(key)?, decode_component(value)?))
        })
        .collect()
}

fn decode_component(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&b, tail)) = rest.split_first() {
        match b {
            b'+' => bytes.push(b' '),
            b'%' => {
                let hex = std::str::from_utf8(tail.get(..2)?).ok()?;
                bytes.push(u8::from_str_radix(hex, 16).ok()?);
                rest = &tail[2..];
                continue;
            }
            _ => bytes.push(b),
        }
        rest = tail;
    }
    String::from_utf8(bytes).ok()
}

fn read_request(stream: &TcpStream) -> io::Result<Option<Request>> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(path)) = (parts.next(), parts.next()) else {
        return Ok(None);
    };
    let mut request = Request {
        method: method.to_string(),
        // The page asks for nothing but / and /route-name, so a query is dropped.
        path: path.split('?').next().unwrap_or_default().to_string(),
        ..Request::default()
    };
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            request
                .headers
                .push((name.trim().to_lowercase(), value.trim().to_string()));
        }
    }
    let length: usize = request
        .header("content-length")
        .and_then(|l| l.parse().ok())
        .unwrap_or(0);
    if length > MAX_BODY {
        return Ok(None);
    }
    request.body = vec![0; length];
    reader.read_exact(&mut request.body)?;
    Ok(Some(request))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Internal Server Error",
    }
}

fn answer(mut stream: &TcpStream, state: &mut State) -> io::Result<()> {
    let response = match read_request(stream)? {
        Some(request) => handle(&request, state),
        None => Response::text(400, "Bad request"),
    };
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        response.status,
        reason(response.status),
        response.content_type,
        response.body.len()
    )?;
    stream.write_all(&response.body)?;
    stream.flush()
}

/// Serve until interrupted; returns the exit code.
pub fn run(state: &mut State) -> i32 {
    let listener = match TcpListener::bind(("127.0.0.1", state.args.port)) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!(
                "error: cannot listen on 127.0.0.1:{}: {e} (try --port)",
                state.args.port
            );
            return 1;
        }
    };
    println!(
        "Serving the report at http://127.0.0.1:{}/ (Ctrl-C to stop)",
        state.args.port
    );
    println!("Renamed routes are saved to {}", state.path.display());
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        // A client that stops sending must not hold up the next one.
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        if let Err(e) = answer(&stream, state) {
            eprintln!("warning: {e}");
        }
    }
    0
}
