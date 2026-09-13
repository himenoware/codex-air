//! Official Codex App Server connection boundary.
//!
//! The server owns ChatGPT authentication and its durable Codex threads. This
//! module never reads, copies, or persists credentials; it only exchanges the
//! documented JSONL control messages with a local `codex app-server` process.
use serde_json::{Value, json};
use std::os::windows::process::CommandExt;
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Command as ProcessCommand, Stdio},
    sync::mpsc,
    thread,
};
use uuid::Uuid;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug, Default)]
pub struct AccountStatus {
    pub connected: bool,
    pub email: Option<String>,
    pub plan: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Update {
    Status(AccountStatus),
    LoginUrl(String),
    ThreadStarted { workspace: Uuid, thread_id: String },
    Activity(String),
    TurnFinished,
    Error(String),
}

#[derive(Clone, Debug)]
pub enum Request {
    StartChatGptLogin,
    Refresh,
    StartTurn {
        workspace: Uuid,
        cwd: PathBuf,
        text: String,
        thread_id: Option<String>,
    },
}

pub fn start() -> (mpsc::Sender<Request>, async_channel::Receiver<Update>) {
    let (requests, request_receiver) = mpsc::channel();
    let (updates, update_receiver) = async_channel::unbounded();
    thread::Builder::new()
        .name("air-app-server".into())
        .spawn(move || run(request_receiver, updates))
        .expect("Could not start Codex App Server worker");
    (requests, update_receiver)
}

fn run(requests: mpsc::Receiver<Request>, updates: async_channel::Sender<Update>) {
    let executable = match codex_executable() {
        Ok(path) => path,
        Err(error) => {
            let _ = updates.send_blocking(Update::Status(AccountStatus {
                detail: Some(format!("Codex CLI is unavailable: {error}")),
                ..Default::default()
            }));
            return;
        }
    };
    let mut child = match ProcessCommand::new(executable)
        .args(["app-server", "--stdio"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ = updates.send_blocking(Update::Status(AccountStatus {
                detail: Some(format!("Could not start Codex App Server: {error}")),
                ..Default::default()
            }));
            return;
        }
    };

    let Some(stdin) = child.stdin.take() else {
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        return;
    };
    let (wire, wire_receiver) = mpsc::channel::<Value>();
    let writer = thread::spawn(move || {
        let mut stdin = stdin;
        while let Ok(message) = wire_receiver.recv() {
            if serde_json::to_writer(&mut stdin, &message).is_err()
                || stdin.write_all(b"\n").is_err()
                || stdin.flush().is_err()
            {
                break;
            }
        }
    });

    let send = |message: Value| wire.send(message).is_ok();
    if !send(json!({
        "id": 1,
        "method": "initialize",
        "params": {"clientInfo": {"name": "Codex Air", "version": env!("CARGO_PKG_VERSION"), "title": "Codex Air"}}
    })) {
        return;
    }

    let (lines, line_receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(message) = serde_json::from_str::<Value>(&line)
                && lines.send(message).is_err()
            {
                break;
            }
        }
    });

    let mut initialized = false;
    let mut next_id = 3_i64;
    let mut login_id = None::<i64>;
    let mut thread_start = None::<(i64, Uuid, PathBuf, String)>;
    let mut turn_start = None::<i64>;
    'connection: loop {
        loop {
            let request = match requests.try_recv() {
                Ok(request) => request,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => break 'connection,
            };
            if !initialized {
                let _ = updates.send_blocking(Update::Status(AccountStatus {
                    detail: Some("Connecting to Codex App Server…".into()),
                    ..Default::default()
                }));
                continue;
            }
            match request {
                Request::StartChatGptLogin => {
                    if login_id.is_none() {
                        let id = next_id;
                        next_id += 1;
                        login_id = Some(id);
                        let _ = send(json!({
                            "id": id,
                            "method": "account/login/start",
                            "params": {
                                "type": "chatgpt",
                                "appBrand": "codex",
                                "useHostedLoginSuccessPage": true
                            }
                        }));
                    }
                }
                Request::Refresh => {
                    let id = next_id;
                    next_id += 1;
                    let _ = send(
                        json!({"id": id, "method": "account/read", "params": {"refreshToken": false}}),
                    );
                }
                Request::StartTurn {
                    workspace,
                    cwd,
                    text,
                    thread_id,
                } => {
                    if thread_start.is_some() || turn_start.is_some() {
                        let _ = updates.send_blocking(Update::Error(
                            "Codex is already working on this workspace.".into(),
                        ));
                        continue;
                    }
                    let id = next_id;
                    next_id += 1;
                    thread_start = Some((id, workspace, cwd.clone(), text));
                    let (method, params, activity) = if let Some(thread_id) = thread_id {
                        (
                            "thread/resume",
                            json!({"threadId": thread_id}),
                            "Resuming Codex session…",
                        )
                    } else {
                        (
                            "thread/start",
                            json!({
                                "cwd": cwd,
                                "approvalPolicy": "unlessTrusted",
                                "sandbox": "workspaceWrite",
                                "serviceName": "codex_air"
                            }),
                            "Starting Codex session…",
                        )
                    };
                    let _ = updates.send_blocking(Update::Activity(activity.into()));
                    let _ = send(json!({"id": id, "method": method, "params": params}));
                }
            }
        }
        let message = match line_receiver.recv_timeout(std::time::Duration::from_millis(80)) {
            Ok(message) => message,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if message.get("id") == Some(&json!(1)) && message.get("result").is_some() {
            initialized = true;
            let _ = send(json!({"method": "initialized", "params": {}}));
            let _ =
                send(json!({"id": 2, "method": "account/read", "params": {"refreshToken": false}}));
            continue;
        }
        if login_id.is_some_and(|id| message.get("id") == Some(&json!(id))) {
            login_id = None;
            if let Some(url) = message.pointer("/result/authUrl").and_then(Value::as_str) {
                let _ = updates.send_blocking(Update::LoginUrl(url.to_owned()));
                let _ = updates.send_blocking(Update::Status(AccountStatus {
                    detail: Some(
                        "Finish signing in in your browser. Codex Air will update automatically."
                            .into(),
                    ),
                    ..Default::default()
                }));
            } else if let Some(error) = message.pointer("/error/message").and_then(Value::as_str) {
                let _ = updates.send_blocking(Update::Status(AccountStatus {
                    detail: Some(error.to_owned()),
                    ..Default::default()
                }));
            }
            continue;
        }
        if let Some((id, workspace, cwd, text)) = thread_start.as_ref()
            && message.get("id") == Some(&json!(id))
        {
            let Some(thread_id) = message
                .pointer("/result/thread/id")
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                thread_start = None;
                let detail = message
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex could not start a session.")
                    .to_owned();
                let _ = updates.send_blocking(Update::Error(detail));
                continue;
            };
            let request_id = next_id;
            next_id += 1;
            turn_start = Some(request_id);
            let workspace = *workspace;
            let cwd = cwd.clone();
            let text = text.clone();
            thread_start = None;
            let _ = updates.send_blocking(Update::ThreadStarted {
                workspace,
                thread_id: thread_id.clone(),
            });
            let _ = send(json!({
                "id": request_id,
                "method": "turn/start",
                "params": {
                    "threadId": thread_id,
                    "input": [{"type": "text", "text": text}],
                    "cwd": cwd,
                    "approvalPolicy": "unlessTrusted",
                    "sandboxPolicy": {
                        "type": "workspaceWrite",
                        "writableRoots": [cwd],
                        "networkAccess": true
                    },
                    "effort": "medium",
                    "summary": "concise"
                }
            }));
            continue;
        }
        if turn_start.is_some_and(|id| message.get("id") == Some(&json!(id))) {
            turn_start = None;
            if let Some(error) = message.pointer("/error/message").and_then(Value::as_str) {
                let _ = updates.send_blocking(Update::Error(error.to_owned()));
            }
            continue;
        }
        if message.get("method").and_then(Value::as_str) == Some("account/login/completed") {
            if message.pointer("/params/success") == Some(&json!(true)) {
                let id = next_id;
                next_id += 1;
                let _ = send(
                    json!({"id": id, "method": "account/read", "params": {"refreshToken": false}}),
                );
            } else {
                let detail = message
                    .pointer("/params/error")
                    .and_then(Value::as_str)
                    .unwrap_or("ChatGPT sign-in did not complete.");
                let _ = updates.send_blocking(Update::Status(AccountStatus {
                    detail: Some(detail.to_owned()),
                    ..Default::default()
                }));
            }
            continue;
        }
        if message
            .get("result")
            .and_then(|result| result.get("requiresOpenaiAuth"))
            .is_some()
        {
            let account = message.pointer("/result/account");
            let status = AccountStatus {
                connected: account.is_some_and(|value| !value.is_null()),
                email: account
                    .and_then(|value| value.get("email"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                plan: account
                    .and_then(|value| value.get("planType"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                detail: None,
            };
            let _ = updates.send_blocking(Update::Status(status));
        }
        if let Some(method) = message.get("method").and_then(Value::as_str) {
            match method {
                "item/agentMessage/delta"
                | "item/reasoning/summaryTextDelta"
                | "item/commandExecution/outputDelta" => {
                    if let Some(delta) = message.pointer("/params/delta").and_then(Value::as_str) {
                        let _ = updates.send_blocking(Update::Activity(delta.to_owned()));
                    }
                }
                "item/started" => {
                    if let Some(kind) = message.pointer("/params/item/type").and_then(Value::as_str)
                    {
                        let _ = updates.send_blocking(Update::Activity(format!("Codex: {kind}")));
                    }
                }
                "turn/completed" => {
                    let _ = updates.send_blocking(Update::TurnFinished);
                }
                _ => {}
            }
        }
    }
    drop(wire);
    let _ = writer.join();
    let _ = child.kill();
}

fn codex_executable() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CODEX_AIR_CODEX_PATH") {
        return Ok(path.into());
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let bundled = PathBuf::from(local).join("Programs\\OpenAI\\Codex\\bin\\codex.exe");
        if bundled.is_file() {
            return Ok(bundled);
        }
    }
    Ok(PathBuf::from("codex"))
}
