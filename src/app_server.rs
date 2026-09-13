//! Official Codex App Server connection boundary.
//!
//! The server owns ChatGPT authentication and its durable Codex threads. This
//! module never reads, copies, or persists credentials; it only exchanges the
//! documented JSONL control messages with a local `codex app-server` process.
use serde_json::{Value, json};
use std::os::windows::process::CommandExt;
use std::{
    cmp::Ordering,
    collections::{HashMap, VecDeque},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Command as ProcessCommand, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
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
pub struct Approval {
    pub id: String,
    pub workspace: Uuid,
    pub description: String,
}

#[derive(Clone, Debug)]
pub struct Question {
    pub id: String,
    pub question: String,
    pub options: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum Update {
    Status(AccountStatus),
    LoginUrl(String),
    ThreadStarted {
        workspace: Uuid,
        thread_id: String,
    },
    Item {
        workspace: Uuid,
        id: String,
        kind: String,
        text: String,
    },
    ApprovalRequested(Approval),
    QuestionsRequested {
        workspace: Uuid,
        id: String,
        questions: Vec<Question>,
    },
    TurnFinished {
        workspace: Uuid,
        error: Option<String>,
    },
    ThreadLoaded {
        workspace: Uuid,
        thread_id: String,
    },
    TokenUsage {
        workspace: Uuid,
        thread_id: String,
        used: i64,
        context_window: Option<i64>,
    },
    WorkspaceError {
        workspace: Uuid,
        message: String,
    },
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
        model: Option<String>,
        effort: Option<String>,
        attachments: Vec<PathBuf>,
    },
    ReadThread {
        workspace: Uuid,
        thread_id: String,
    },
    Interrupt {
        workspace: Uuid,
    },
    ResolveApproval {
        id: String,
        accept: bool,
    },
    AnswerQuestions {
        id: String,
        answers: Vec<(String, Vec<String>)>,
    },
    Inspect {
        method: String,
        params: Value,
        reply: async_channel::Sender<Result<Value, String>>,
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
        let _ = child.kill();
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
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
        "params": {
            "clientInfo": {"name": "Codex Air", "version": env!("CARGO_PKG_VERSION"), "title": "Codex Air"},
            "capabilities": {"experimentalApi": true}
        }
    })) {
        drop(wire);
        let _ = writer.join();
        let _ = child.kill();
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
    let mut thread_start = None::<(
        i64,
        Uuid,
        PathBuf,
        String,
        Option<String>,
        Option<String>,
        Vec<PathBuf>,
    )>;
    let mut turn_start = None::<(i64, Uuid, String)>;
    let mut active_turn = None::<(Uuid, String, String)>;
    let mut thread_workspaces = HashMap::<String, Uuid>::new();
    let mut thread_read_pending = HashMap::<i64, (Uuid, String, Instant)>::new();
    let mut interrupt_pending = HashMap::<i64, Uuid>::new();
    let mut item_text = HashMap::<(Uuid, String), (String, String)>::new();
    let mut pending_approvals = HashMap::<String, (Value, String, Uuid, Value)>::new();
    let mut pending_questions = HashMap::<String, (Value, Uuid)>::new();
    let mut inspect_pending =
        HashMap::<i64, (async_channel::Sender<Result<Value, String>>, Instant)>::new();
    let mut deferred_requests = VecDeque::<Request>::new();
    'connection: loop {
        loop {
            let request = if let Some(request) = deferred_requests.pop_front() {
                request
            } else {
                match requests.try_recv() {
                    Ok(request) => request,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => break 'connection,
                }
            };
            if !initialized {
                let _ = updates.send_blocking(Update::Status(AccountStatus {
                    detail: Some("Connecting to Codex App Server…".into()),
                    ..Default::default()
                }));
                deferred_requests.push_back(request);
                break;
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
                Request::Inspect {
                    method,
                    params,
                    reply,
                } => {
                    let id = next_id;
                    next_id += 1;
                    inspect_pending.insert(id, (reply, Instant::now()));
                    if !send(json!({"id": id, "method": method, "params": params}))
                        && let Some((reply, _)) = inspect_pending.remove(&id)
                    {
                        let _ =
                            reply.send_blocking(Err("Codex App Server connection closed.".into()));
                    }
                }
                Request::ReadThread {
                    workspace,
                    thread_id,
                } => {
                    let id = next_id;
                    next_id += 1;
                    thread_read_pending.insert(id, (workspace, thread_id.clone(), Instant::now()));
                    if !send(json!({
                        "id": id,
                        "method": "thread/read",
                        "params": {"threadId": thread_id, "includeTurns": true}
                    })) {
                        thread_read_pending.remove(&id);
                        let _ = updates.send_blocking(Update::WorkspaceError {
                            workspace,
                            message:
                                "Codex App Server connection closed while loading the session."
                                    .into(),
                        });
                    }
                }
                Request::Interrupt { workspace } => {
                    let Some((active_workspace, thread_id, turn_id)) = active_turn.as_ref() else {
                        let _ = updates.send_blocking(Update::WorkspaceError {
                            workspace,
                            message: "There is no active Codex turn to stop.".into(),
                        });
                        continue;
                    };
                    if *active_workspace != workspace || turn_id.is_empty() {
                        let _ = updates.send_blocking(Update::WorkspaceError {
                            workspace,
                            message: "There is no active Codex turn to stop in this workspace."
                                .into(),
                        });
                        continue;
                    }
                    let thread_id = thread_id.clone();
                    let turn_id = turn_id.clone();
                    if thread_id.is_empty() || turn_id.is_empty() {
                        let _ = updates.send_blocking(Update::WorkspaceError {
                            workspace,
                            message: "There is no active Codex turn to stop.".into(),
                        });
                        continue;
                    }
                    let id = next_id;
                    next_id += 1;
                    interrupt_pending.insert(id, workspace);
                    if !send(json!({
                        "id": id,
                        "method": "turn/interrupt",
                        "params": {"threadId": thread_id, "turnId": turn_id}
                    })) {
                        interrupt_pending.remove(&id);
                        let _ = updates.send_blocking(Update::WorkspaceError {
                            workspace,
                            message: "Codex App Server connection closed while stopping the turn."
                                .into(),
                        });
                    }
                }
                Request::AnswerQuestions { id, answers } => {
                    let Some((request_id, _workspace)) = pending_questions.remove(&id) else {
                        let _ = updates.send_blocking(Update::Error(
                            "That Codex question request is no longer active.".into(),
                        ));
                        continue;
                    };
                    let mut answer_map = serde_json::Map::new();
                    for (question_id, values) in answers {
                        answer_map.insert(question_id, json!({"answers": values}));
                    }
                    let _ = send(json!({
                        "id": request_id,
                        "result": {"answers": answer_map}
                    }));
                }
                Request::ResolveApproval { id, accept } => {
                    let Some((request_id, method, workspace, requested_permissions)) =
                        pending_approvals.remove(&id)
                    else {
                        let _ = updates.send_blocking(Update::Error(
                            "That Codex approval request is no longer active.".into(),
                        ));
                        continue;
                    };
                    let result = match method.as_str() {
                        "item/commandExecution/requestApproval"
                        | "item/fileChange/requestApproval" => {
                            json!({"decision": if accept { "accept" } else { "decline" }})
                        }
                        "item/permissions/requestApproval" => {
                            json!({
                                "permissions": if accept {
                                    requested_permissions
                                } else {
                                    json!({"fileSystem": null, "network": null})
                                },
                                "scope": "turn"
                            })
                        }
                        _ => {
                            let _ = updates.send_blocking(Update::Error(
                                "Codex sent an unsupported approval request.".into(),
                            ));
                            continue;
                        }
                    };
                    let _ = workspace;
                    let _ = send(json!({"id": request_id, "result": result}));
                }
                Request::StartTurn {
                    workspace,
                    cwd,
                    text,
                    thread_id,
                    model,
                    effort,
                    attachments,
                } => {
                    if thread_start.is_some() || turn_start.is_some() || active_turn.is_some() {
                        let _ = updates.send_blocking(Update::Error(
                            "Codex is already working on this workspace.".into(),
                        ));
                        continue;
                    }
                    let id = next_id;
                    next_id += 1;
                    thread_start = Some((
                        id,
                        workspace,
                        cwd.clone(),
                        text,
                        model.clone(),
                        effort.clone(),
                        attachments,
                    ));
                    let (method, mut params, activity) = if let Some(thread_id) = thread_id {
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
                                "serviceName": "codex_air"
                            }),
                            "Starting Codex session…",
                        )
                    };
                    if let Some(model) = model {
                        params["model"] = json!(model);
                    }
                    let _ = updates.send_blocking(Update::Item {
                        workspace,
                        id: format!("local-start-{id}"),
                        kind: "status".into(),
                        text: activity.into(),
                    });
                    if !send(json!({"id": id, "method": method, "params": params})) {
                        thread_start = None;
                        let detail =
                            "Codex App Server connection closed while starting the session.";
                        let _ = updates.send_blocking(Update::Error(detail.into()));
                        let _ = updates.send_blocking(Update::TurnFinished {
                            workspace,
                            error: Some(detail.into()),
                        });
                    }
                }
            }
        }
        let message = match line_receiver.recv_timeout(std::time::Duration::from_millis(80)) {
            Ok(message) => message,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                expire_thread_reads(&mut thread_read_pending, &updates);
                let expired = inspect_pending
                    .iter()
                    .filter_map(|(id, (_, started))| {
                        (started.elapsed() >= Duration::from_secs(30)).then_some(*id)
                    })
                    .collect::<Vec<_>>();
                for id in expired {
                    if let Some((reply, _)) = inspect_pending.remove(&id) {
                        let _ =
                            reply.send_blocking(Err("Codex App Server request timed out.".into()));
                    }
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some((workspace, _, _)) = active_turn.take() {
                    let _ = updates.send_blocking(Update::TurnFinished {
                        workspace,
                        error: Some("Codex App Server disconnected.".into()),
                    });
                }
                let _ =
                    updates.send_blocking(Update::Error("Codex App Server disconnected.".into()));
                break;
            }
        };
        expire_thread_reads(&mut thread_read_pending, &updates);
        let expired = inspect_pending
            .iter()
            .filter_map(|(id, (_, started))| {
                (started.elapsed() >= Duration::from_secs(30)).then_some(*id)
            })
            .collect::<Vec<_>>();
        for id in expired {
            if let Some((reply, _)) = inspect_pending.remove(&id) {
                let _ = reply.send_blocking(Err("Codex App Server request timed out.".into()));
            }
        }
        if let Some(id) = message.get("id").and_then(Value::as_i64)
            && let Some((workspace, requested_thread_id, _)) = thread_read_pending.remove(&id)
        {
            if let Some(error) = message.pointer("/error/message").and_then(Value::as_str) {
                let _ = updates.send_blocking(Update::WorkspaceError {
                    workspace,
                    message: error.to_owned(),
                });
                continue;
            }
            let Some(thread) = message.pointer("/result/thread") else {
                let _ = updates.send_blocking(Update::WorkspaceError {
                    workspace,
                    message: "Codex returned an invalid session transcript.".into(),
                });
                continue;
            };
            let thread_id = thread
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or(&requested_thread_id)
                .to_owned();
            thread_workspaces.insert(thread_id.clone(), workspace);
            let _ = updates.send_blocking(Update::ThreadLoaded {
                workspace,
                thread_id: thread_id.clone(),
            });
            if let Some(turns) = thread.get("turns").and_then(Value::as_array) {
                for turn in turns {
                    let Some(items) = turn.get("items").and_then(Value::as_array) else {
                        continue;
                    };
                    for item in items {
                        let Some(item_id) = item.get("id").and_then(Value::as_str) else {
                            continue;
                        };
                        let kind = item
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or("item")
                            .to_owned();
                        let text = item_text_from_authoritative(item);
                        item_text.insert(
                            (workspace, item_id.to_owned()),
                            (kind.clone(), text.clone()),
                        );
                        let _ = updates.send_blocking(Update::Item {
                            workspace,
                            id: item_id.to_owned(),
                            kind,
                            text,
                        });
                    }
                }
            }
            continue;
        }
        if let Some(id) = message.get("id").and_then(Value::as_i64)
            && let Some(workspace) = interrupt_pending.remove(&id)
        {
            if let Some(error) = message.pointer("/error/message").and_then(Value::as_str) {
                let _ = updates.send_blocking(Update::WorkspaceError {
                    workspace,
                    message: error.to_owned(),
                });
            }
            continue;
        }
        if let Some(id) = message.get("id").and_then(Value::as_i64)
            && let Some((reply, _)) = inspect_pending.remove(&id)
        {
            let result = if let Some(error) = message.get("error") {
                Err(error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex App Server request failed.")
                    .to_owned())
            } else {
                Ok(message.get("result").cloned().unwrap_or(Value::Null))
            };
            let _ = reply.send_blocking(result);
            continue;
        }
        if message.get("id") == Some(&json!(1)) && message.get("result").is_some() {
            initialized = true;
            let _ = send(json!({"method": "initialized", "params": {}}));
            let _ =
                send(json!({"id": 2, "method": "account/read", "params": {"refreshToken": false}}));
            continue;
        }
        if message.get("id") == Some(&json!(1)) && message.get("error").is_some() {
            let detail = message
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Codex App Server initialization failed.")
                .to_owned();
            let _ = updates.send_blocking(Update::Status(AccountStatus {
                detail: Some(detail.clone()),
                ..Default::default()
            }));
            let _ = updates.send_blocking(Update::Error(detail.clone()));
            for request in deferred_requests.drain(..) {
                reject_request(request, &updates, &detail);
            }
            break 'connection;
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
        if let Some((id, workspace, cwd, text, model, effort, attachments)) = thread_start.as_ref()
            && message.get("id") == Some(&json!(id))
        {
            let Some(thread_id) = message
                .pointer("/result/thread/id")
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                let workspace = *workspace;
                thread_start = None;
                let detail = message
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex could not start a session.")
                    .to_owned();
                let _ = updates.send_blocking(Update::Error(detail.clone()));
                let _ = updates.send_blocking(Update::TurnFinished {
                    workspace,
                    error: Some(detail),
                });
                continue;
            };
            let request_id = next_id;
            next_id += 1;
            turn_start = Some((request_id, *workspace, thread_id.clone()));
            let workspace = *workspace;
            let cwd = cwd.clone();
            let text = text.clone();
            let model = model.clone();
            let effort = effort.clone();
            let attachments = attachments.clone();
            let input = turn_input(&text, &attachments);
            let mut turn_params = json!({
                "threadId": thread_id,
                "input": input,
                "cwd": cwd
            });
            if let Some(model) = model {
                turn_params["model"] = json!(model);
            }
            if let Some(effort) = effort {
                turn_params["effort"] = json!(effort);
            }
            thread_start = None;
            let _ = updates.send_blocking(Update::ThreadStarted {
                workspace,
                thread_id: thread_id.clone(),
            });
            thread_workspaces.insert(thread_id.clone(), workspace);
            if !send(json!({
                "id": request_id,
                "method": "turn/start",
                "params": turn_params
            })) {
                turn_start = None;
                let detail =
                    "Codex App Server connection closed while starting the turn.".to_owned();
                let _ = updates.send_blocking(Update::Error(detail.clone()));
                let _ = updates.send_blocking(Update::TurnFinished {
                    workspace,
                    error: Some(detail),
                });
            }
            continue;
        }
        if turn_start
            .as_ref()
            .is_some_and(|(id, _, _)| message.get("id") == Some(&json!(id)))
        {
            let (_, workspace, thread_id) = turn_start.take().expect("turn start exists");
            if let Some(error) = message.pointer("/error/message").and_then(Value::as_str) {
                let _ = updates.send_blocking(Update::Error(error.to_owned()));
                let _ = updates.send_blocking(Update::TurnFinished {
                    workspace,
                    error: Some(error.to_owned()),
                });
            } else {
                let turn_id = message
                    .pointer("/result/turn/id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                active_turn = Some((workspace, thread_id, turn_id));
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
                | "item/reasoning/textDelta"
                | "item/commandExecution/outputDelta"
                | "item/fileChange/outputDelta"
                | "item/commandExecution/terminalInteraction" => {
                    let params = message.get("params").cloned().unwrap_or(Value::Null);
                    let Some(item_id) = params.get("itemId").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(thread_id) = params.get("threadId").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(workspace) = thread_workspaces.get(thread_id).copied() else {
                        continue;
                    };
                    let delta = params
                        .get("delta")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let kind = item_text
                        .get(&(workspace, item_id.to_owned()))
                        .map(|(kind, _)| kind.clone())
                        .unwrap_or_else(|| delta_kind(method).into());
                    let entry = item_text
                        .entry((workspace, item_id.to_owned()))
                        .or_insert_with(|| (kind.clone(), String::new()));
                    entry.1.push_str(delta);
                    let _ = updates.send_blocking(Update::Item {
                        workspace,
                        id: item_id.to_owned(),
                        kind: entry.0.clone(),
                        text: entry.1.clone(),
                    });
                }
                "item/started" => {
                    let params = message.get("params").cloned().unwrap_or(Value::Null);
                    let Some(thread_id) = params.get("threadId").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(workspace) = thread_workspaces.get(thread_id).copied() else {
                        continue;
                    };
                    if let Some(item) = params.get("item")
                        && let Some(item_id) = item.get("id").and_then(Value::as_str)
                    {
                        let kind = item
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or("item")
                            .to_owned();
                        let text = item_text_from_authoritative(item);
                        item_text.insert(
                            (workspace, item_id.to_owned()),
                            (kind.clone(), text.clone()),
                        );
                        let _ = updates.send_blocking(Update::Item {
                            workspace,
                            id: item_id.to_owned(),
                            kind,
                            text,
                        });
                    }
                }
                "item/completed" => {
                    let params = message.get("params").cloned().unwrap_or(Value::Null);
                    let Some(thread_id) = params.get("threadId").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(workspace) = thread_workspaces.get(thread_id).copied() else {
                        continue;
                    };
                    if let Some(item) = params.get("item")
                        && let Some(item_id) = item.get("id").and_then(Value::as_str)
                    {
                        let kind = item
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or("item")
                            .to_owned();
                        let text = item_text_from_authoritative(item);
                        item_text.insert(
                            (workspace, item_id.to_owned()),
                            (kind.clone(), text.clone()),
                        );
                        let _ = updates.send_blocking(Update::Item {
                            workspace,
                            id: item_id.to_owned(),
                            kind,
                            text,
                        });
                    }
                }
                "turn/completed" => {
                    let params = message.get("params").cloned().unwrap_or(Value::Null);
                    let Some(thread_id) = params.get("threadId").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some((workspace, active_thread_id, active_turn_id)) =
                        active_turn.take().or_else(|| {
                            turn_start.take().map(|(_, workspace, thread_id)| {
                                (workspace, thread_id, String::new())
                            })
                        })
                    else {
                        continue;
                    };
                    if active_thread_id != thread_id {
                        active_turn = Some((workspace, active_thread_id, active_turn_id));
                        continue;
                    }
                    let error = params
                        .pointer("/turn/error/message")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or_else(|| {
                            params
                                .pointer("/turn/status")
                                .and_then(Value::as_str)
                                .and_then(|status| {
                                    (status == "failed").then(|| "Codex turn failed.".to_owned())
                                })
                        });
                    let _ = updates.send_blocking(Update::TurnFinished { workspace, error });
                }
                "error" => {
                    let detail = message
                        .pointer("/params/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("Codex reported an error.")
                        .to_owned();
                    let _ = updates.send_blocking(Update::Error(detail));
                }
                "thread/tokenUsage/updated" => {
                    let Some(params) = message.get("params") else {
                        continue;
                    };
                    let Some(thread_id) = params.get("threadId").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(workspace) = thread_workspaces.get(thread_id).copied() else {
                        continue;
                    };
                    let used = params
                        .pointer("/tokenUsage/last/totalTokens")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    let context_window = params
                        .pointer("/tokenUsage/modelContextWindow")
                        .and_then(Value::as_i64);
                    let _ = updates.send_blocking(Update::TokenUsage {
                        workspace,
                        thread_id: thread_id.to_owned(),
                        used,
                        context_window,
                    });
                }
                "item/tool/requestUserInput" => {
                    let Some(request_id) = message.get("id").cloned() else {
                        continue;
                    };
                    let params = message.get("params").cloned().unwrap_or(Value::Null);
                    let Some(thread_id) = params.get("threadId").and_then(Value::as_str) else {
                        send_server_error(
                            &send,
                            &request_id,
                            "Codex question is missing a thread id.",
                        );
                        continue;
                    };
                    let Some(workspace) = thread_workspaces.get(thread_id).copied() else {
                        send_server_error(
                            &send,
                            &request_id,
                            "Codex question belongs to an unknown workspace.",
                        );
                        continue;
                    };
                    let questions = params
                        .get("questions")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|question| {
                            let id = question.get("id")?.as_str()?.to_owned();
                            let text = question.get("question")?.as_str()?.to_owned();
                            let options = question
                                .get("options")
                                .and_then(Value::as_array)
                                .into_iter()
                                .flatten()
                                .filter_map(|option| option.get("label").and_then(Value::as_str))
                                .map(str::to_owned)
                                .collect();
                            Some(Question {
                                id,
                                question: text,
                                options,
                            })
                        })
                        .collect::<Vec<_>>();
                    if questions.is_empty() {
                        send_server_error(
                            &send,
                            &request_id,
                            "Codex sent an empty question request.",
                        );
                        continue;
                    }
                    let id = request_id_string(&request_id);
                    pending_questions.insert(id.clone(), (request_id, workspace));
                    let _ = updates.send_blocking(Update::QuestionsRequested {
                        workspace,
                        id,
                        questions,
                    });
                }
                "mcpServer/elicitation/request" => {
                    if let Some(request_id) = message.get("id") {
                        let _ = send(json!({"id": request_id, "result": {"action": "cancel"}}));
                    }
                }
                "item/tool/call" => {
                    if let Some(request_id) = message.get("id") {
                        let _ = send(json!({
                            "id": request_id,
                            "result": {"success": false, "contentItems": []}
                        }));
                    }
                }
                "item/commandExecution/requestApproval"
                | "item/fileChange/requestApproval"
                | "item/permissions/requestApproval" => {
                    let Some(request_id) = message.get("id").cloned() else {
                        continue;
                    };
                    let params = message.get("params").cloned().unwrap_or(Value::Null);
                    let Some(thread_id) = params.get("threadId").and_then(Value::as_str) else {
                        send_server_error(
                            &send,
                            &request_id,
                            "Codex approval is missing a thread id.",
                        );
                        continue;
                    };
                    let Some(workspace) = thread_workspaces.get(thread_id).copied() else {
                        send_server_error(
                            &send,
                            &request_id,
                            "Codex approval belongs to an unknown workspace.",
                        );
                        continue;
                    };
                    let id = request_id_string(&request_id);
                    let description = approval_description(method, &params);
                    let requested_permissions = params
                        .get("permissions")
                        .cloned()
                        .unwrap_or_else(|| json!({"fileSystem": null, "network": null}));
                    pending_approvals.insert(
                        id.clone(),
                        (
                            request_id,
                            method.to_owned(),
                            workspace,
                            requested_permissions,
                        ),
                    );
                    let _ = updates.send_blocking(Update::ApprovalRequested(Approval {
                        id,
                        workspace,
                        description,
                    }));
                }
                _ => {
                    if let Some(request_id) = message.get("id") {
                        send_server_error(
                            &send,
                            request_id,
                            "Codex Air does not support this App Server request.",
                        );
                    }
                }
            }
        }
    }
    if let Some((workspace, _, _)) = active_turn.take() {
        let _ = updates.send_blocking(Update::TurnFinished {
            workspace,
            error: Some("Codex App Server stopped.".into()),
        });
    }
    for (_, (workspace, _, _)) in thread_read_pending.drain() {
        let _ = updates.send_blocking(Update::WorkspaceError {
            workspace,
            message: "Codex App Server stopped while loading the session.".into(),
        });
    }
    for (_, workspace) in interrupt_pending.drain() {
        let _ = updates.send_blocking(Update::WorkspaceError {
            workspace,
            message: "Codex App Server stopped while stopping the turn.".into(),
        });
    }
    for (_, (_, workspace)) in pending_questions.drain() {
        let _ = updates.send_blocking(Update::WorkspaceError {
            workspace,
            message: "Codex App Server stopped while waiting for your answers.".into(),
        });
    }
    for request in deferred_requests.drain(..) {
        reject_request(
            request,
            &updates,
            "Codex App Server stopped before the request could run.",
        );
    }
    for (_, (reply, _)) in inspect_pending.drain() {
        let _ = reply.send_blocking(Err("Codex App Server stopped.".into()));
    }
    drop(wire);
    let _ = writer.join();
    let _ = child.kill();
}

fn reject_request(request: Request, updates: &async_channel::Sender<Update>, detail: &str) {
    match request {
        Request::Inspect { reply, .. } => {
            let _ = reply.send_blocking(Err(detail.to_owned()));
        }
        Request::ReadThread { workspace, .. } | Request::Interrupt { workspace } => {
            let _ = updates.send_blocking(Update::WorkspaceError {
                workspace,
                message: detail.to_owned(),
            });
        }
        Request::StartTurn { workspace, .. } => {
            let _ = updates.send_blocking(Update::Error(detail.to_owned()));
            let _ = updates.send_blocking(Update::TurnFinished {
                workspace,
                error: Some(detail.to_owned()),
            });
        }
        Request::ResolveApproval { .. }
        | Request::AnswerQuestions { .. }
        | Request::StartChatGptLogin
        | Request::Refresh => {
            let _ = updates.send_blocking(Update::Error(detail.to_owned()));
        }
    }
}

fn send_server_error(send: &impl Fn(Value) -> bool, id: &Value, message: &str) {
    let _ = send(json!({
        "id": id,
        "error": {"code": -32601, "message": message}
    }));
}

fn expire_thread_reads(
    pending: &mut HashMap<i64, (Uuid, String, Instant)>,
    updates: &async_channel::Sender<Update>,
) {
    let expired = pending
        .iter()
        .filter_map(|(id, (_, _, started))| {
            (started.elapsed() >= Duration::from_secs(30)).then_some(*id)
        })
        .collect::<Vec<_>>();
    for id in expired {
        if let Some((workspace, _, _)) = pending.remove(&id) {
            let _ = updates.send_blocking(Update::WorkspaceError {
                workspace,
                message: "Codex App Server request timed out while loading the session.".into(),
            });
        }
    }
}

fn request_id_string(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn delta_kind(method: &str) -> &'static str {
    match method {
        "item/agentMessage/delta" => "agentMessage",
        "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => "reasoning",
        "item/commandExecution/outputDelta" | "item/commandExecution/terminalInteraction" => {
            "commandExecution"
        }
        "item/fileChange/outputDelta" => "fileChange",
        _ => "item",
    }
}

fn item_text_from_authoritative(item: &Value) -> String {
    if let Some(text) = item.get("text").and_then(Value::as_str) {
        return text.to_owned();
    }
    if let Some(command) = item.get("command").and_then(Value::as_str) {
        if let Some(output) = item.get("aggregatedOutput").and_then(Value::as_str) {
            return format!("$ {command}\n\n{output}");
        }
        return format!("$ {command}");
    }
    if let Some(output) = item.get("aggregatedOutput").and_then(Value::as_str) {
        return output.to_owned();
    }
    if let Some(summary) = item.get("summary").and_then(Value::as_array) {
        return summary
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    if let Some(content) = item.get("content").and_then(Value::as_array) {
        return content
            .iter()
            .filter_map(|value| {
                value
                    .as_str()
                    .or_else(|| value.get("text").and_then(Value::as_str))
            })
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    item.get("changes")
        .and_then(Value::as_array)
        .map(|changes| {
            changes
                .iter()
                .filter_map(|change| {
                    let path = change.get("path").and_then(Value::as_str)?;
                    let kind = change
                        .pointer("/kind/type")
                        .and_then(Value::as_str)
                        .unwrap_or("change");
                    let diff = change
                        .get("diff")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    Some(if diff.is_empty() {
                        format!("{kind} {path}")
                    } else {
                        format!("{kind} {path}\n{diff}")
                    })
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default()
}

fn turn_input(text: &str, attachments: &[PathBuf]) -> Vec<Value> {
    let mut prompt = text.to_owned();
    let mut input = Vec::with_capacity(attachments.len() + 1);
    for path in attachments {
        let absolute_path = absolute_attachment_path(path);
        if supported_local_image(&absolute_path) && absolute_path.is_file() {
            input.push(json!({
                "type": "localImage",
                "path": absolute_path.to_string_lossy(),
                "detail": "auto"
            }));
        } else {
            prompt.push_str("\n\nAttached local file (available in the workspace): ");
            prompt.push_str(&absolute_path.to_string_lossy());
        }
    }
    input.insert(0, json!({"type": "text", "text": prompt}));
    input
}

fn absolute_attachment_path(path: &PathBuf) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.clone()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.clone())
    };
    std::fs::canonicalize(&absolute).unwrap_or(absolute)
}

fn supported_local_image(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif"
            )
        })
        .unwrap_or(false)
}

fn approval_description(method: &str, params: &Value) -> String {
    match method {
        "item/commandExecution/requestApproval" => params
            .get("command")
            .and_then(Value::as_str)
            .or_else(|| params.get("reason").and_then(Value::as_str))
            .unwrap_or("Codex wants to run a command.")
            .to_owned(),
        "item/fileChange/requestApproval" => params
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("Codex wants to modify files.")
            .to_owned(),
        "item/permissions/requestApproval" => params
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("Codex requests additional workspace permissions.")
            .to_owned(),
        _ => "Codex requests approval.".into(),
    }
}

fn codex_executable() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CODEX_AIR_CODEX_PATH") {
        return Ok(path.into());
    }

    let mut candidates = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("Programs\\OpenAI\\Codex\\bin\\codex.exe"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            candidates.push(directory.join("codex.exe"));
            candidates.push(directory.join("codex.cmd"));
        }
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        let extensions = PathBuf::from(profile).join(".vscode\\extensions");
        if let Ok(entries) = std::fs::read_dir(extensions) {
            for entry in entries.flatten() {
                let path = entry.path();
                if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
                    && entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("openai.chatgpt-")
                {
                    candidates.push(path.join("bin\\windows-x86_64\\codex.exe"));
                }
            }
        }
    }

    let mut best = None::<(PathBuf, CliVersion)>;
    let mut unversioned = None::<PathBuf>;
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        let Some(version) = codex_version(&candidate) else {
            unversioned.get_or_insert(candidate);
            continue;
        };
        if best.as_ref().is_none_or(|(_, current)| version > *current) {
            best = Some((candidate, version));
        }
    }
    Ok(best
        .map(|(path, _)| path)
        .or(unversioned)
        .unwrap_or_else(|| PathBuf::from("codex")))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CliVersion {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Option<Vec<CliVersionPart>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CliVersionPart {
    Numeric(u64),
    Text(String),
}

impl Ord for CliVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(left), Some(right)) => left.cmp(right),
            })
    }
}

impl PartialOrd for CliVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CliVersionPart {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Numeric(left), Self::Numeric(right)) => left.cmp(right),
            (Self::Numeric(_), Self::Text(_)) => Ordering::Less,
            (Self::Text(_), Self::Numeric(_)) => Ordering::Greater,
            (Self::Text(left), Self::Text(right)) => left.cmp(right),
        }
    }
}

impl PartialOrd for CliVersionPart {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn codex_version(path: &std::path::Path) -> Option<CliVersion> {
    let output = ProcessCommand::new(path)
        .arg("--version")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    parse_cli_version(&String::from_utf8_lossy(&output.stdout))
}

fn parse_cli_version(output: &str) -> Option<CliVersion> {
    output.split_whitespace().find_map(|token| {
        let token = token.trim_matches(|character: char| {
            !character.is_ascii_alphanumeric() && character != '.' && character != '-'
        });
        let token = token.strip_prefix('v').unwrap_or(token);
        let mut sections = token.splitn(2, '-');
        let core = sections.next()?;
        let mut numbers = core.split('.');
        let major = numbers.next()?.parse().ok()?;
        let minor = numbers.next()?.parse().ok()?;
        let patch = numbers.next()?.parse().ok()?;
        let prerelease = sections.next().map(|value| {
            value
                .split('.')
                .map(|part| {
                    part.parse::<u64>()
                        .map(CliVersionPart::Numeric)
                        .unwrap_or_else(|_| CliVersionPart::Text(part.to_owned()))
                })
                .collect()
        });
        Some(CliVersion {
            major,
            minor,
            patch,
            prerelease,
        })
    })
}
