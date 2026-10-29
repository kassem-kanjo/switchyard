//! `switchyard view`: serve the production viewer locally.
//!
//! Release binaries embed `viewer/dist` (feature `embed-viewer`) and serve it
//! on `127.0.0.1` only: the viewer is a dev/demo tool, never exposed to the
//! network. The request logic lives in the pure [`respond`] so it is tested
//! without starting a server or building the frontend.

use std::process::Command;
use std::process::ExitCode;

use clap::Args;
use tiny_http::Method;

/// The embedded production viewer (`viewer/dist`), present only when the
/// `embed-viewer` feature is on. `include_dir!` fails the build when the
/// directory is missing, which is why the feature is opt-in.
#[cfg(feature = "embed-viewer")]
static VIEWER: include_dir::Dir<'static> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/../../viewer/dist");

/// Arguments of `switchyard view`.
#[derive(Debug, Args)]
pub struct ViewArgs {
    /// A scenario preset to load in the viewer (`small`, `medium`, `large`,
    /// or any other preset the viewer understands).
    #[arg(long, default_value = "medium")]
    scenario: String,
    /// Port to serve on, bound to 127.0.0.1 only.
    #[arg(long, default_value_t = 8765)]
    port: u16,
    /// Do not open a web browser; just serve and print the URL.
    #[arg(long)]
    no_open: bool,
}

/// An HTTP response produced by [`respond`].
#[derive(Debug)]
pub struct Response {
    /// HTTP status code.
    pub status: u16,
    /// Value of the `Content-Type` header.
    pub mime: &'static str,
    /// Bytes of the response body.
    pub body: Vec<u8>,
}

/// Run the subcommand; errors go to stderr.
pub fn main(args: &ViewArgs) -> ExitCode {
    if !cfg!(feature = "embed-viewer") {
        eprintln!(
            "this build of switchyard has no embedded viewer; use a release binary, or build \
             with `npm run build` in viewer/ and then `cargo build --features embed-viewer`."
        );
        return ExitCode::FAILURE;
    }
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Validate arguments, bind the server, print the URL and serve until killed.
fn run(args: &ViewArgs) -> Result<(), String> {
    if args.scenario.is_empty()
        || !args
            .scenario
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("scenario must contain only [A-Za-z0-9_-]".into());
    }
    let url = format!("http://127.0.0.1:{}/?preset={}", args.port, args.scenario);
    // Bind explicitly to loopback: the viewer is not for the network.
    let server = match tiny_http::Server::http(("127.0.0.1", args.port)) {
        Ok(server) => server,
        Err(e) => {
            return Err(format!("cannot bind 127.0.0.1:{}: {e}", args.port));
        }
    };
    println!("Serving the Switchyard viewer at {url}  (Ctrl-C to stop)");
    if !args.no_open && !open_browser(&url) {
        eprintln!("could not open a browser; open {url} manually");
    }
    serve(server);
    Ok(())
}

/// Open `url` in the platform's default browser; failure is not fatal.
fn open_browser(url: &str) -> bool {
    if cfg!(target_os = "windows") {
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .is_ok()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn().is_ok()
    } else {
        Command::new("xdg-open").arg(url).spawn().is_ok()
    }
}

/// Answer requests until the process is stopped (Ctrl-C terminates it).
fn serve(server: tiny_http::Server) {
    for request in server.incoming_requests() {
        let head = *request.method() == Method::Head;
        if !matches!(*request.method(), Method::Get | Method::Head) {
            let response = Response {
                status: 405,
                mime: "text/plain",
                body: b"method not allowed".to_vec(),
            };
            let _ = request.respond(to_http(response, false));
            continue;
        }
        let response = respond(request.url(), &lookup);
        let _ = request.respond(to_http(response, head));
    }
}

/// The embedded viewer file at `path`, or `None` when this build has none.
#[cfg(feature = "embed-viewer")]
fn lookup(path: &str) -> Option<Vec<u8>> {
    VIEWER.get_file(path).map(|file| file.contents().to_vec())
}

/// Without the feature nothing is embedded; `main` never reaches the server.
#[cfg(not(feature = "embed-viewer"))]
fn lookup(_path: &str) -> Option<Vec<u8>> {
    None
}

/// Convert a [`Response`] to a tiny_http response with the required headers.
/// `head` responses carry the headers but an empty body.
fn to_http(response: Response, head: bool) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let body = if head { Vec::new() } else { response.body };
    tiny_http::Response::from_data(body)
        .with_status_code(response.status)
        .with_header(header("Content-Type", response.mime))
        .with_header(header("Cache-Control", "no-store"))
        .with_header(header("X-Content-Type-Options", "nosniff"))
}

/// Build a header from trusted static text.
fn header(name: &'static str, value: &'static str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(name, value).expect("static header values are valid")
}

/// Resolve `url` against the viewer files supplied by `lookup`.
///
/// The query and fragment are ignored, `/` maps to `index.html`, and any
/// traversal or encoded-traversal attempt is rejected before the lookup.
/// Unknown files are 404s. Being a pure function of its inputs, it is tested
/// with an in-memory fixture.
pub fn respond(url: &str, lookup: &dyn Fn(&str) -> Option<Vec<u8>>) -> Response {
    let target = url.split(['?', '#']).next().unwrap_or("");
    let path = target.strip_prefix('/').unwrap_or(target);

    if forbidden(path) {
        return Response {
            status: 400,
            mime: "text/plain",
            body: b"bad request".to_vec(),
        };
    }

    let file = if path.is_empty() { "index.html" } else { path };
    match lookup(file) {
        Some(body) => Response {
            status: 200,
            mime: mime_for(file),
            body,
        },
        None => Response {
            status: 404,
            mime: "text/plain",
            body: b"not found".to_vec(),
        },
    }
}

/// Whether a stripped path may not be served: no traversal, no backslashes or
/// NUL bytes, no absolute/empty segments, and no percent-encoded separators.
fn forbidden(path: &str) -> bool {
    if path.contains("..")
        || path.contains('\\')
        || path.contains('\0')
        || path.starts_with('/')
        || path.contains("//")
        || path.ends_with('/')
    {
        return true;
    }
    let lower = path.to_ascii_lowercase();
    lower.contains("%2e") || lower.contains("%2f") || lower.contains("%5c")
}

/// The MIME type for `path`, chosen by its extension.
fn mime_for(path: &str) -> &'static str {
    let ext = path
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());
    match ext.as_deref() {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("md") => "text/markdown; charset=utf-8",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    // The determinism rule (clippy.toml) bans hashed collections in the
    // simulator; an in-memory request fixture has no iteration-order contract.
    #![allow(clippy::disallowed_types)]

    use std::collections::HashMap;

    use super::{Response, respond};

    /// A tiny stand-in for `viewer/dist`.
    fn viewer() -> HashMap<&'static str, Vec<u8>> {
        HashMap::from([
            ("index.html", b"<html>viewer</html>".to_vec()),
            ("assets/x.wasm", b"\0asm".to_vec()),
            ("reports/a.md", b"# report".to_vec()),
            ("assets/app.js", b"console.log(1)".to_vec()),
            ("assets/app.css", b"body{}".to_vec()),
        ])
    }

    fn get(url: &str) -> Response {
        let files = viewer();
        let lookup = |path: &str| files.get(path).cloned();
        respond(url, &lookup)
    }

    #[test]
    fn root_serves_index_html() {
        let response = get("/");
        assert_eq!(response.status, 200);
        assert_eq!(response.mime, "text/html; charset=utf-8");
        assert_eq!(response.body, b"<html>viewer</html>");
    }

    #[test]
    fn wasm_has_the_wasm_mime() {
        let response = get("/assets/x.wasm");
        assert_eq!(response.status, 200);
        assert_eq!(response.mime, "application/wasm");
    }

    #[test]
    fn markdown_has_the_markdown_mime() {
        let response = get("/reports/a.md");
        assert_eq!(response.status, 200);
        assert_eq!(response.mime, "text/markdown; charset=utf-8");
    }

    #[test]
    fn javascript_and_css_have_their_mimes() {
        assert_eq!(get("/assets/app.js").mime, "text/javascript; charset=utf-8");
        assert_eq!(get("/assets/app.css").mime, "text/css; charset=utf-8");
    }

    #[test]
    fn unknown_file_is_404() {
        let response = get("/nope.js");
        assert_eq!(response.status, 404);
        assert_eq!(response.mime, "text/plain");
        assert_eq!(response.body, b"not found");
    }

    #[test]
    fn traversal_and_encoded_separators_are_400() {
        for url in ["/../secret", "/a/../b", "/%2e%2e/x", "/a\\b"] {
            let response = get(url);
            assert_eq!(response.status, 400, "{url} should be rejected");
            assert_eq!(response.mime, "text/plain");
        }
    }

    #[test]
    fn query_is_stripped_before_lookup() {
        let response = get("/?preset=small");
        assert_eq!(response.status, 200);
        assert_eq!(response.mime, "text/html; charset=utf-8");
        assert_eq!(response.body, b"<html>viewer</html>");
    }

    #[test]
    fn server_binds_loopback() {
        let server =
            tiny_http::Server::http(("127.0.0.1", 0)).expect("bind an ephemeral loopback port");
        let addr = server
            .server_addr()
            .to_ip()
            .expect("the listener has an IP address");
        assert!(addr.ip().is_loopback());
    }
}
