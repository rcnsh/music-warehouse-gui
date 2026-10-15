//! Sign in with Spotify, so the Worker mints a read token for this Mac
//! instead of the user copying one.
//!
//! This is the Worker's native-app flow: OAuth-style with PKCE (RFC 7636) and
//! a loopback redirect (RFC 8252). The app listens on an OS-assigned port on
//! 127.0.0.1, sends the browser to `/auth/login`, and the Worker redirects the
//! browser back to that port with a one-time code. The code is worthless
//! without the verifier, which never leaves this process, and it is exchanged
//! at `/auth/token` for the token. The Worker only ever mints read tokens
//! here, and this module refuses anything else it might return.

use std::process::Command;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use url::Url;

use crate::api::Secret;
use crate::models::ErrorBody;
use crate::runtime::AbortOnDrop;

/// How long to wait for the user to finish in the browser.
const BROWSER_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// A connection that sends no request line in this time is dropped, so a
/// browser's speculative preconnect cannot hold anything up.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REQUEST_HEAD: usize = 16 * 1024;
const CALLBACK_PATH: &str = "/callback";
/// The Worker's limit on token names, counted as JavaScript does (UTF-16).
const MAX_NAME_UNITS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignInError {
    #[error("Sign-in was cancelled in the browser.")]
    Cancelled,
    #[error(
        "That Spotify account does not own this warehouse. Sign in with the account that \
         authorized it."
    )]
    NotOwner,
    #[error("The Worker could not confirm your Spotify account. Try again later.")]
    Unavailable,
    #[error("The Worker refused the sign-in: {0}")]
    Refused(String),
    #[error("Timed out waiting for the browser. Try again.")]
    TimedOut,
    #[error("The sign-in code expired or was already used. Try again.")]
    InvalidGrant,
    #[error("The Worker returned a token that is not read-only, so it was not saved.")]
    NotReadOnly,
    #[error("The Worker returned HTTP {status}: {message}")]
    Server { status: u16, message: String },
    #[error("Could not reach the Worker: {0}")]
    Network(String),
    #[error("Unexpected response from the Worker: {0}")]
    Decode(String),
    #[error("Could not listen for the browser: {0}")]
    Local(String),
}

impl SignInError {
    /// The `?error=` values the Worker sends back to `return_to`.
    fn from_callback(code: &str) -> Self {
        match code {
            "access_denied" => Self::Cancelled,
            "not_owner" => Self::NotOwner,
            "unavailable" => Self::Unavailable,
            other => Self::Refused(other.chars().take(100).collect()),
        }
    }
}

/// A PKCE verifier and its S256 challenge.
pub struct Pkce {
    pub verifier: Secret,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Result<Self, SignInError> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| SignInError::Local(e.to_string()))?;
        // 32 bytes encode to 43 characters, all in the verifier alphabet.
        Ok(Self::from_verifier(&URL_SAFE_NO_PAD.encode(bytes)))
    }

    fn from_verifier(verifier: &str) -> Self {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self {
            verifier: Secret::new(verifier),
            challenge,
        }
    }
}

/// The token name the Worker lists this Mac under, e.g. `macOS app (Studio)`.
pub fn device_name(computer: &str) -> String {
    const PREFIX: &str = "macOS app (";
    let computer = computer.trim();
    if computer.is_empty() {
        return "macOS app".into();
    }
    let budget = MAX_NAME_UNITS - PREFIX.len() - 1;
    let mut used = 0;
    let host: String = computer
        .chars()
        .take_while(|c| {
            used += c.len_utf16();
            used <= budget
        })
        .collect();
    format!("{PREFIX}{})", host.trim_end())
}

/// The Mac's user-visible name from Sharing settings, if it has one.
fn computer_name() -> String {
    Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// `<base>/auth/<route>`, keeping any path prefix on the Worker URL.
fn auth_url(base: &Url, route: &str) -> Url {
    let mut url = base.clone();
    url.set_path(&format!(
        "{}/auth/{route}",
        base.path().trim_end_matches('/')
    ));
    url.set_query(None);
    url
}

pub fn login_url(base: &Url, name: &str, return_to: &str, challenge: &str) -> Url {
    let mut url = auth_url(base, "login");
    url.query_pairs_mut()
        .append_pair("name", name)
        .append_pair("return_to", return_to)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    url
}

/// What one request to the loopback listener was.
#[derive(Debug, PartialEq, Eq)]
pub enum Callback {
    Code(String),
    Refused(SignInError),
    /// Anything else, such as the browser asking for `/favicon.ico`.
    Other,
}

/// Reads the request line of an HTTP request head. Only `GET /callback`
/// counts; the Worker sends either `code` or `error` in its query.
pub fn parse_callback(head: &str) -> Callback {
    let mut parts = head.lines().next().unwrap_or_default().split_whitespace();
    let (Some("GET"), Some(target)) = (parts.next(), parts.next()) else {
        return Callback::Other;
    };
    if !target.starts_with('/') {
        return Callback::Other;
    }
    let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
        return Callback::Other;
    };
    if url.path() != CALLBACK_PATH {
        return Callback::Other;
    }
    let param = |key: &str| {
        url.query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
            .filter(|v| !v.is_empty())
    };
    match (param("code"), param("error")) {
        (Some(code), _) => Callback::Code(code),
        (None, Some(error)) => Callback::Refused(SignInError::from_callback(&error)),
        (None, None) => Callback::Refused(SignInError::Refused("no code in the redirect".into())),
    }
}

#[derive(Deserialize)]
struct Minted {
    token: String,
    name: String,
    access: String,
    #[serde(default)]
    expires_ms: Option<i64>,
}

/// The token the Worker minted for this Mac.
#[derive(Debug)]
pub struct SignedIn {
    pub token: Secret,
    pub name: String,
    pub expires_ms: Option<i64>,
}

impl SignedIn {
    /// For the setup screen, e.g. `Signed in as "macOS app (Studio)" until 13 Jan 2027.`
    pub fn describe(&self) -> String {
        let until = self
            .expires_ms
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|at| {
                format!(
                    " until {}",
                    at.with_timezone(&chrono::Local).format("%-d %b %Y")
                )
            })
            .unwrap_or_default();
        format!("Signed in as \"{}\"{until}.", self.name)
    }
}

/// Maps the `/auth/token` response. Kept free of I/O so it can be tested.
pub fn exchange_outcome(status: u16, body: &str) -> Result<SignedIn, SignInError> {
    if status != 200 {
        let parsed: ErrorBody = serde_json::from_str(body).unwrap_or_default();
        return Err(match (status, parsed.error) {
            (400, Some(error)) if error == "invalid_grant" => SignInError::InvalidGrant,
            (_, error) => SignInError::Server {
                status,
                message: error.unwrap_or_else(|| body.chars().take(200).collect()),
            },
        });
    }
    let minted: Minted =
        serde_json::from_str(body).map_err(|e| SignInError::Decode(e.to_string()))?;
    if minted.access != "read" {
        return Err(SignInError::NotReadOnly);
    }
    if minted.token.trim().is_empty() {
        return Err(SignInError::Decode("empty token".into()));
    }
    Ok(SignedIn {
        token: Secret::new(minted.token),
        name: minted.name,
        expires_ms: minted.expires_ms,
    })
}

/// A sign-in waiting for the browser. Dropping it closes the listener.
pub struct Pending {
    pub login_url: Url,
    base: Url,
    listener: TcpListener,
    verifier: Secret,
}

/// Starts listening and builds the URL to open. Must run on the Tokio runtime.
pub async fn start(base: Url) -> Result<Pending, SignInError> {
    // The IP literal, not `localhost`: the Worker refuses names a hosts file
    // could point elsewhere.
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| SignInError::Local(e.to_string()))?;
    let port = listener
        .local_addr()
        .map_err(|e| SignInError::Local(e.to_string()))?
        .port();
    let return_to = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
    let pkce = Pkce::generate()?;
    let name = device_name(&computer_name());
    Ok(Pending {
        login_url: login_url(&base, &name, &return_to, &pkce.challenge),
        base,
        listener,
        verifier: pkce.verifier,
    })
}

impl Pending {
    /// Waits for the browser to come back, then exchanges the code. Must run
    /// on the Tokio runtime.
    pub async fn finish(self) -> Result<SignedIn, SignInError> {
        let code = wait_for_code(self.listener, BROWSER_TIMEOUT).await?;
        exchange(&self.base, &code, &self.verifier).await
    }
}

/// Failed accepts in a row before sign-in gives up, and the pause after each.
const ACCEPT_FAILURES: u32 = 20;
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// Serves the loopback listener until one request carries the Worker's
/// answer, then stops listening. Each connection is handled on its own task,
/// so a connection that never sends anything cannot block the real redirect.
async fn wait_for_code(listener: TcpListener, timeout: Duration) -> Result<String, SignInError> {
    let (tx, mut rx) = mpsc::channel(1);
    let accept = tokio::spawn(async move {
        let mut failures = 0;
        loop {
            let stream = match listener.accept().await {
                Ok((stream, _)) => stream,
                // Usually transient (a connection reset before it was
                // accepted, or briefly out of file descriptors), but one
                // that persists would otherwise spin a core until timeout.
                Err(e) => {
                    failures += 1;
                    if failures >= ACCEPT_FAILURES {
                        let message = format!("the loopback listener failed: {e}");
                        let _ = tx.send(Err(SignInError::Local(message))).await;
                        return;
                    }
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                    continue;
                }
            };
            failures = 0;
            let tx = tx.clone();
            tokio::spawn(async move {
                if let Some(outcome) = serve(stream).await {
                    let _ = tx.send(outcome).await;
                }
            });
        }
    });
    // Closes the listener however this function ends, including cancellation.
    let _accept = AbortOnDrop(accept.abort_handle());
    match tokio::time::timeout(timeout, rx.recv()).await {
        Ok(Some(outcome)) => outcome,
        Ok(None) => Err(SignInError::Local("listener stopped".into())),
        Err(_) => Err(SignInError::TimedOut),
    }
}

/// Answers one connection. `None` means it was not the redirect.
async fn serve(mut stream: TcpStream) -> Option<Result<String, SignInError>> {
    let head = tokio::time::timeout(REQUEST_TIMEOUT, read_head(&mut stream))
        .await
        .ok()?;
    let (status, page, outcome) = match parse_callback(&head) {
        Callback::Other => ("404 Not Found", page("Not found", ""), None),
        Callback::Code(code) => (
            "200 OK",
            page(
                "Signed in",
                "You can close this tab and return to Music Warehouse.",
            ),
            Some(Ok(code)),
        ),
        Callback::Refused(error) => {
            let message = format!("{error} You can close this tab.");
            ("200 OK", page("Sign-in failed", &message), Some(Err(error)))
        }
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
    outcome
}

/// Reads up to the blank line that ends a request head. A GET has no body.
async fn read_head(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    while buf.len() < MAX_REQUEST_HEAD && !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><meta charset=\"utf-8\"><title>{title}</title>\
         <style>body{{font:16px/1.5 system-ui,sans-serif;max-width:36rem;margin:3rem auto;\
         padding:0 1rem}}</style><h1>{title}</h1><p>{}</p>",
        escape_html(body)
    )
}

fn escape_html(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '&' | '<' | '>' | '"' | '\'' => format!("&#{};", c as u32),
            c => c.to_string(),
        })
        .collect()
}

pub fn exchange_body(code: &str, verifier: &Secret) -> serde_json::Value {
    serde_json::json!({ "code": code, "code_verifier": verifier.expose() })
}

async fn exchange(base: &Url, code: &str, verifier: &Secret) -> Result<SignedIn, SignInError> {
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("music-warehouse-gui/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| SignInError::Local(e.to_string()))?;
    let response = http
        .post(auth_url(base, "token"))
        .json(&exchange_body(code, verifier))
        .send()
        .await
        .map_err(|e| SignInError::Network(e.without_url().to_string()))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| SignInError::Network(e.without_url().to_string()))?;
    exchange_outcome(status.as_u16(), &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn base() -> Url {
        crate::api::parse_base_url("https://music-api.example.com/").unwrap()
    }

    #[test]
    fn pkce_matches_rfc_7636_appendix_b() {
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(
            pkce.challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn generated_verifiers_fit_the_workers_rules() {
        let a = Pkce::generate().unwrap();
        let b = Pkce::generate().unwrap();
        for pkce in [&a, &b] {
            let v = pkce.verifier.expose();
            assert!((43..=128).contains(&v.len()));
            assert!(
                v.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c))
            );
            assert_eq!(pkce.challenge.len(), 43);
            assert!(!pkce.challenge.contains('='));
        }
        assert_ne!(a.verifier, b.verifier);
    }

    #[test]
    fn device_name_fits_the_workers_limit() {
        assert_eq!(device_name("Studio"), "macOS app (Studio)");
        assert_eq!(device_name("  "), "macOS app");
        let long = device_name(&"é".repeat(100));
        assert!(long.encode_utf16().count() <= MAX_NAME_UNITS);
        assert!(long.starts_with("macOS app (é") && long.ends_with("é)"));
        // A character outside the BMP counts twice, as in JavaScript.
        let emoji = device_name(&"🎧".repeat(40));
        assert!(emoji.encode_utf16().count() <= MAX_NAME_UNITS);
    }

    #[test]
    fn login_url_carries_every_parameter() {
        let url = login_url(
            &base(),
            "macOS app (Studio)",
            "http://127.0.0.1:5123/callback",
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
        );
        assert_eq!(
            url.as_str(),
            "https://music-api.example.com/auth/login?name=macOS+app+%28Studio%29\
             &return_to=http%3A%2F%2F127.0.0.1%3A5123%2Fcallback\
             &code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM\
             &code_challenge_method=S256"
        );
        let prefixed = crate::api::parse_base_url("https://example.com/warehouse/").unwrap();
        assert_eq!(
            auth_url(&prefixed, "token").as_str(),
            "https://example.com/warehouse/auth/token"
        );
    }

    #[test]
    fn callback_requests_are_classified() {
        let req = |line: &str| format!("{line}\r\nHost: 127.0.0.1\r\n\r\n");
        assert_eq!(
            parse_callback(&req("GET /callback?code=abc%2B1 HTTP/1.1")),
            Callback::Code("abc+1".into())
        );
        assert_eq!(
            parse_callback(&req("GET /callback?error=access_denied HTTP/1.1")),
            Callback::Refused(SignInError::Cancelled)
        );
        assert_eq!(
            parse_callback(&req("GET /callback?error=not_owner HTTP/1.1")),
            Callback::Refused(SignInError::NotOwner)
        );
        assert_eq!(
            parse_callback(&req("GET /callback?error=unavailable HTTP/1.1")),
            Callback::Refused(SignInError::Unavailable)
        );
        assert!(matches!(
            parse_callback(&req("GET /callback HTTP/1.1")),
            Callback::Refused(SignInError::Refused(_))
        ));
        assert_eq!(
            parse_callback(&req("GET /favicon.ico HTTP/1.1")),
            Callback::Other
        );
        assert_eq!(
            parse_callback(&req("POST /callback?code=abc HTTP/1.1")),
            Callback::Other
        );
        assert_eq!(parse_callback(""), Callback::Other);
    }

    #[test]
    fn exchange_responses_are_classified() {
        let ok = exchange_outcome(
            200,
            r#"{"id":"t1","token":"mw_abc","name":"macOS app (Studio)","access":"read","expires_ms":1800000000000}"#,
        )
        .unwrap();
        assert_eq!(ok.token.expose(), "mw_abc");
        assert_eq!(ok.name, "macOS app (Studio)");
        assert_eq!(ok.expires_ms, Some(1_800_000_000_000));
        assert!(
            ok.describe()
                .starts_with("Signed in as \"macOS app (Studio)\" until ")
        );

        assert_eq!(
            exchange_outcome(400, r#"{"error":"invalid_grant"}"#).unwrap_err(),
            SignInError::InvalidGrant
        );
        assert_eq!(
            exchange_outcome(
                200,
                r#"{"id":"t1","token":"x","name":"n","access":"admin","expires_ms":null}"#
            )
            .unwrap_err(),
            SignInError::NotReadOnly
        );
        assert!(matches!(
            exchange_outcome(500, "<html>oops</html>").unwrap_err(),
            SignInError::Server { status: 500, .. }
        ));
        assert!(matches!(
            exchange_outcome(200, "{}").unwrap_err(),
            SignInError::Decode(_)
        ));
    }

    #[test]
    fn exchange_body_is_code_and_verifier() {
        let body = exchange_body("abc", &Secret::new("verifier"));
        assert_eq!(
            body,
            serde_json::json!({ "code": "abc", "code_verifier": "verifier" })
        );
    }

    fn test_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    }

    /// Plays the browser against a real loopback listener: an idle
    /// preconnect, a favicon request, then the redirect.
    #[test]
    fn listener_ignores_strays_and_answers_the_redirect() {
        let rt = test_runtime();
        let listener = rt.block_on(TcpListener::bind(("127.0.0.1", 0))).unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiting = rt.spawn(wait_for_code(listener, Duration::from_secs(10)));

        let get = |path: &str| {
            let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        };
        let _idle = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        assert!(get("/favicon.ico").starts_with("HTTP/1.1 404"));
        let page = get("/callback?code=one-time");
        assert!(page.starts_with("HTTP/1.1 200"));
        assert!(page.contains("You can close this tab"));

        assert_eq!(rt.block_on(waiting).unwrap().unwrap(), "one-time");
        // The listener is closed once the answer is in.
        std::thread::sleep(Duration::from_millis(50));
        assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());
    }

    #[test]
    fn listener_reports_a_refusal_and_times_out() {
        let rt = test_runtime();
        let listener = rt.block_on(TcpListener::bind(("127.0.0.1", 0))).unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiting = rt.spawn(wait_for_code(listener, Duration::from_secs(10)));
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(stream, "GET /callback?error=not_owner HTTP/1.1\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("does not own this warehouse"));
        assert_eq!(
            rt.block_on(waiting).unwrap().unwrap_err(),
            SignInError::NotOwner
        );

        let listener = rt.block_on(TcpListener::bind(("127.0.0.1", 0))).unwrap();
        assert_eq!(
            rt.block_on(wait_for_code(listener, Duration::from_millis(50)))
                .unwrap_err(),
            SignInError::TimedOut
        );
    }

    #[test]
    fn refusal_pages_escape_what_the_redirect_said() {
        assert!(!page("Sign-in failed", "<script>").contains("<script>"));
    }
}
