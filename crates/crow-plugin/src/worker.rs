//! One dedicated thread and one Rhai engine per enabled plugin. The worker owns
//! script evaluation and the operation/deadline budget; the UI only enqueues.
//! File excerpts are read by the host and arrive as immutable request input.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crow_core::{Descriptor, ExtensionId, HookEvent, LocalId, PluginId, Snapshot};
use rhai::{AST, Array, Dynamic, Engine, EvalAltResult, Map, Position, Scope};

use crate::dto::{Call, CallError, CallErrorKind, Reply, RequestInput, RequestStamp, WorkerMessage};
use crate::host::Inbox;
use crate::limits::Limits;
use crate::manifest::API_VERSION;
use crate::registry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnqueueError {
    Busy,
}

/// The result of queueing a hook. The host, not the mailbox, owns the terminal
/// event for a displaced or rejected call.
pub(crate) enum RefreshEnqueue {
    Queued,
    Replaced(Call),
    Full,
    Closed,
}

pub(crate) struct Mailbox {
    state: Mutex<MailboxState>,
    ready: Condvar,
    command_capacity: usize,
    refresh_capacity: usize,
}

struct MailboxState {
    commands: VecDeque<Call>,
    refreshes: VecDeque<Call>,
    closed: bool,
}

impl Mailbox {
    pub fn new(limits: &Limits) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(MailboxState { commands: VecDeque::new(), refreshes: VecDeque::new(), closed: false }),
            ready: Condvar::new(),
            command_capacity: limits.command_mailbox,
            refresh_capacity: limits.refresh_mailbox,
        })
    }

    pub fn enqueue_command(&self, call: Call) -> Result<(), EnqueueError> {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return Err(EnqueueError::Busy);
        }
        if state.commands.len() >= self.command_capacity {
            return Err(EnqueueError::Busy);
        }
        state.commands.push_back(call);
        self.ready.notify_one();
        Ok(())
    }

    /// Refresh work coalesces by extension/event/scope/context. A displaced or
    /// rejected call goes back to the host so it can release the request record
    /// and emit exactly one terminal event. Explicit commands report busy.
    pub fn enqueue_refresh(&self, call: Call) -> RefreshEnqueue {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return RefreshEnqueue::Closed;
        }
        if let Some(key) = call.coalesce_key()
            && let Some(index) = state.refreshes.iter().position(|queued| queued.coalesce_key().as_ref() == Some(&key))
        {
            let displaced = std::mem::replace(&mut state.refreshes[index], call);
            self.ready.notify_one();
            return RefreshEnqueue::Replaced(displaced);
        }
        if state.refreshes.len() >= self.refresh_capacity {
            return RefreshEnqueue::Full;
        }
        state.refreshes.push_back(call);
        self.ready.notify_one();
        RefreshEnqueue::Queued
    }

    fn recv(&self, cancel: &AtomicBool) -> Option<Call> {
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(call) = state.commands.pop_front() {
                return Some(call);
            }
            if let Some(call) = state.refreshes.pop_front() {
                return Some(call);
            }
            if state.closed || cancel.load(Ordering::Relaxed) {
                return None;
            }
            let (guard, _) = self.ready.wait_timeout(state, Duration::from_millis(10)).unwrap();
            state = guard;
        }
    }

    pub fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        for call in state.commands.iter().chain(state.refreshes.iter()) {
            call.cancel();
        }
        self.ready.notify_all();
    }
}

pub(crate) struct Worker {
    pub mailbox: Arc<Mailbox>,
    cancel: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn spawn(
        plugin: PluginId,
        generation: u64,
        snapshot: Arc<crate::package::PackageSnapshot>,
        limits: Limits,
        inbox: Sender<Inbox>,
    ) -> Result<Self, String> {
        let mailbox = Mailbox::new(&limits);
        let cancel = Arc::new(AtomicBool::new(false));
        let runtime = Arc::new(Runtime::new(cancel.clone()));
        let thread_mailbox = mailbox.clone();
        let thread_cancel = cancel.clone();
        let join = thread::Builder::new()
            .name(format!("crow-plugin-{plugin}"))
            .spawn(move || {
                worker_main(plugin, generation, snapshot, limits, thread_mailbox, thread_cancel, runtime, inbox);
            })
            .map_err(|error| format!("could not start worker: {error}"))?;
        Ok(Self { mailbox, cancel, join: Some(join) })
    }

    pub fn stop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.mailbox.close();
    }

    pub fn take_join(&mut self) -> Option<JoinHandle<()>> {
        self.join.take()
    }
}

struct Runtime {
    cancel: Arc<AtomicBool>,
    deadline: Mutex<Option<Instant>>,
    terminate: AtomicU8,
    violation: AtomicBool,
    call_cancel: Mutex<Option<crate::dto::CancelToken>>,
}

impl Runtime {
    fn new(cancel: Arc<AtomicBool>) -> Self {
        Self {
            cancel,
            deadline: Mutex::new(None),
            terminate: AtomicU8::new(0),
            violation: AtomicBool::new(false),
            call_cancel: Mutex::new(None),
        }
    }

    fn begin(&self, deadline: Instant, call_cancel: Option<crate::dto::CancelToken>) {
        *self.deadline.lock().unwrap() = Some(deadline);
        *self.call_cancel.lock().unwrap() = call_cancel;
        self.terminate.store(0, Ordering::Relaxed);
        self.violation.store(false, Ordering::Relaxed);
    }

    fn end(&self) {
        *self.deadline.lock().unwrap() = None;
        *self.call_cancel.lock().unwrap() = None;
    }

    /// Read before `end()` clears the call token. A cancellation that races a
    /// budget or deadline termination still reports as cancelled; otherwise a
    /// user's Cancel could pause the whole plugin.
    fn call_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
            || self.call_cancel.lock().unwrap().as_ref().is_some_and(|token| token.load(Ordering::Relaxed))
    }
}

#[allow(clippy::too_many_arguments)]
fn worker_main(
    plugin: PluginId,
    generation: u64,
    snapshot: Arc<crate::package::PackageSnapshot>,
    limits: Limits,
    mailbox: Arc<Mailbox>,
    cancel: Arc<AtomicBool>,
    runtime: Arc<Runtime>,
    inbox: Sender<Inbox>,
) {
    let source = match std::str::from_utf8(snapshot.entry_source()) {
        Ok(source) => source,
        Err(_) => {
            send(
                &inbox,
                WorkerMessage::Registered {
                    plugin,
                    generation,
                    result: Err(CallError::script("entry script is not UTF-8")),
                },
            );
            return;
        }
    };
    let assets: Arc<BTreeMap<String, Arc<[u8]>>> = Arc::new(
        snapshot
            .manifest
            .assets
            .iter()
            .filter_map(|name| snapshot.files.get(name).map(|bytes| (name.clone(), bytes.clone())))
            .collect(),
    );
    let mut engine = build_engine(&limits, runtime.clone(), assets.clone());
    let ast = match engine.compile(source) {
        Ok(ast) => ast,
        Err(error) => {
            send(
                &inbox,
                WorkerMessage::Registered {
                    plugin,
                    generation,
                    result: Err(CallError::script(format!("compile error: {error}"))),
                },
            );
            return;
        }
    };
    if !ast.iter_functions().any(|function| function.name == "register" && function.params.is_empty()) {
        send(
            &inbox,
            WorkerMessage::Registered {
                plugin,
                generation,
                result: Err(CallError::invalid("script has no register() entry point")),
            },
        );
        return;
    }

    let Some(Call::Register) = mailbox.recv(&cancel) else {
        return;
    };
    let deadline = Instant::now() + limits.call_deadline;
    let registered = register_package(&engine, &ast, &plugin, &snapshot, &runtime, deadline, &limits);
    match registered {
        Ok(descriptors) => {
            send(
                &inbox,
                WorkerMessage::Registered { plugin: plugin.clone(), generation, result: Ok(descriptors.clone()) },
            );
            let mut by_local: BTreeMap<LocalId, Descriptor> =
                descriptors.into_iter().map(|descriptor| (descriptor.id.local.clone(), descriptor)).collect();
            run_loop(
                &plugin,
                generation,
                &mut engine,
                &ast,
                &snapshot,
                &limits,
                &mailbox,
                &cancel,
                &runtime,
                &inbox,
                &mut by_local,
            );
        }
        Err(error) => {
            send(&inbox, WorkerMessage::Registered { plugin, generation, result: Err(error) });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_loop(
    plugin: &PluginId,
    generation: u64,
    engine: &mut Engine,
    ast: &AST,
    snapshot: &crate::package::PackageSnapshot,
    limits: &Limits,
    mailbox: &Mailbox,
    cancel: &AtomicBool,
    runtime: &Runtime,
    inbox: &Sender<Inbox>,
    registered: &mut BTreeMap<LocalId, Descriptor>,
) {
    while let Some(call) = mailbox.recv(cancel) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let request = call.stamp();
        // The host also checks the token at admission. This second check covers
        // a call that was queued first and cancelled before dequeue: it still
        // gets one terminal result without running the script.
        if call.cancel_token().is_some_and(|token| token.load(Ordering::Relaxed)) {
            send(
                inbox,
                WorkerMessage::Failed { plugin: plugin.clone(), generation, request, error: CallError::cancelled() },
            );
            continue;
        }
        let deadline = Instant::now() + limits.call_deadline;
        let result: Result<(ExtensionId, RequestStamp, Reply), CallError> = match call {
            Call::Register => Err(CallError::script("register() ran twice")),
            Call::Evaluate { extension, input, stamp, cancel } => evaluate(
                engine,
                ast,
                snapshot,
                runtime,
                registered,
                extension.clone(),
                input,
                stamp,
                cancel,
                deadline,
                limits,
            )
            .map(|(stamp, reply)| (extension, stamp, reply)),
            Call::Notify { extension, event, stamp, cancel } => {
                notify(engine, ast, runtime, registered, extension.clone(), event, stamp, cancel, deadline, limits)
                    .map(|(stamp, reply)| (extension, stamp, reply))
            }
        };
        match result {
            Ok((extension, stamp, reply)) => {
                send(inbox, WorkerMessage::Replied { plugin: plugin.clone(), generation, extension, stamp, reply });
            }
            Err(error) => {
                send(
                    inbox,
                    WorkerMessage::Failed { plugin: plugin.clone(), generation, request, error: error.clone() },
                );
                if error.kind != CallErrorKind::Cancelled {
                    break;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn register_package(
    engine: &Engine,
    ast: &AST,
    plugin: &PluginId,
    snapshot: &crate::package::PackageSnapshot,
    runtime: &Runtime,
    deadline: Instant,
    limits: &Limits,
) -> Result<Vec<Descriptor>, CallError> {
    runtime.begin(deadline, None);
    let value = engine.call_fn::<Dynamic>(&mut Scope::new(), ast, "register", ());
    let cancelled = runtime.call_cancelled();
    let violation = runtime.violation.load(Ordering::Relaxed);
    runtime.end();
    if violation {
        return Err(CallError::capability("script attempted a denied host capability"));
    }
    let value = value.map_err(|error| call_error(error, runtime, cancelled))?;
    registry::validate_registration(plugin, &snapshot.manifest, snapshot, ast, value, limits)
}

#[allow(clippy::too_many_arguments)]
fn evaluate(
    engine: &Engine,
    ast: &AST,
    snapshot: &crate::package::PackageSnapshot,
    runtime: &Runtime,
    registered: &BTreeMap<LocalId, Descriptor>,
    extension: ExtensionId,
    input: RequestInput,
    stamp: RequestStamp,
    cancel: crate::dto::CancelToken,
    deadline: Instant,
    limits: &Limits,
) -> Result<(RequestStamp, Reply), CallError> {
    input.validate(limits).map_err(CallError::invalid)?;
    let descriptor = registered
        .get(&extension.local)
        .ok_or_else(|| CallError::invalid(format!("unknown extension \"{extension}\"")))?;
    if descriptor.kind == crow_core::ExtensionKind::Hook || descriptor.kind == crow_core::ExtensionKind::IconPack {
        return Err(CallError::invalid(format!("extension \"{extension}\" is not evaluated directly")));
    }
    let function = descriptor.function().ok_or_else(|| CallError::invalid("extension has no function"))?;
    runtime.begin(deadline, Some(cancel));
    let context = input_map(&input);
    let value = engine.call_fn::<Dynamic>(&mut Scope::new(), ast, function, (context,));
    let cancelled = runtime.call_cancelled();
    let violation = runtime.violation.load(Ordering::Relaxed);
    runtime.end();
    if violation {
        return Err(CallError::capability("script attempted a denied host capability"));
    }
    let value = value.map_err(|error| call_error(error, runtime, cancelled))?;
    let reply = registry::reply_for(descriptor, value, &input, limits)?;
    if snapshot.manifest.id != extension.plugin {
        return Err(CallError::invalid("extension plugin does not match the package"));
    }
    Ok((stamp, reply))
}

#[allow(clippy::too_many_arguments)]
fn notify(
    engine: &Engine,
    ast: &AST,
    runtime: &Runtime,
    registered: &BTreeMap<LocalId, Descriptor>,
    extension: ExtensionId,
    event: HookEvent,
    stamp: RequestStamp,
    cancel: crate::dto::CancelToken,
    deadline: Instant,
    limits: &Limits,
) -> Result<(RequestStamp, Reply), CallError> {
    let descriptor = registered
        .get(&extension.local)
        .ok_or_else(|| CallError::invalid(format!("unknown extension \"{extension}\"")))?;
    if descriptor.kind != crow_core::ExtensionKind::Hook {
        return Err(CallError::invalid(format!("extension \"{extension}\" is not a hook")));
    }
    let function = descriptor.function().ok_or_else(|| CallError::invalid("hook has no function"))?;
    runtime.begin(deadline, Some(cancel));
    let context = event_map(&event);
    let value = engine.call_fn::<Dynamic>(&mut Scope::new(), ast, function, (context,));
    let cancelled = runtime.call_cancelled();
    let violation = runtime.violation.load(Ordering::Relaxed);
    runtime.end();
    if violation {
        return Err(CallError::capability("script attempted a denied host capability"));
    }
    let value = value.map_err(|error| call_error(error, runtime, cancelled))?;
    let reply = registry::reply_for(descriptor, value, &empty_input(), limits)?;
    Ok((stamp, reply))
}

fn empty_input() -> RequestInput {
    RequestInput { snapshot: Snapshot { scope: String::new(), selection: None, entries: Vec::new() }, excerpt: None }
}

fn build_engine(limits: &Limits, runtime: Arc<Runtime>, assets: Arc<BTreeMap<String, Arc<[u8]>>>) -> Engine {
    let mut engine = Engine::new();
    engine.set_max_operations(limits.max_operations);
    engine.set_max_expr_depths(limits.max_expression_depth as usize, limits.max_expression_depth as usize);
    engine.set_max_call_levels(limits.max_call_levels);
    engine.set_max_array_size(limits.max_array_size);
    engine.set_max_map_size(limits.max_map_size);
    engine.set_max_string_size(limits.max_result_bytes);
    engine.set_max_modules(0);
    engine.on_print(|_| {});
    engine.on_debug(|_, _, _| {});
    let progress = runtime.clone();
    engine.on_progress(move |_| {
        let call_cancelled =
            progress.call_cancel.lock().unwrap().as_ref().is_some_and(|token| token.load(Ordering::Relaxed));
        if progress.cancel.load(Ordering::Relaxed) || call_cancelled {
            progress.terminate.store(2, Ordering::Relaxed);
            return Some(Dynamic::UNIT);
        }
        if progress.deadline.lock().unwrap().is_some_and(|deadline| Instant::now() >= deadline) {
            progress.terminate.store(1, Ordering::Relaxed);
            return Some(Dynamic::UNIT);
        }
        None
    });
    #[cfg(test)]
    engine.register_fn("hold_worker", |name: &str| {
        test_gate::hold(name);
    });
    let asset_closure = assets;
    let asset_runtime = runtime.clone();
    engine.register_fn("asset", move |name: &str| -> Result<String, Box<EvalAltResult>> {
        let Some(bytes) = asset_closure.get(name) else {
            asset_runtime.violation.store(true, Ordering::Relaxed);
            return Err(runtime_error(format!("asset \"{name}\" is not declared in plugin.toml")));
        };
        String::from_utf8(bytes.to_vec()).map_err(|_| runtime_error(format!("asset \"{name}\" is not UTF-8 text")))
    });
    engine
}

fn input_map(input: &RequestInput) -> Map {
    let mut map = Map::new();
    map.insert("api".into(), Dynamic::from(API_VERSION as i64));
    map.insert("scope".into(), Dynamic::from(input.snapshot.scope.clone()));
    if let Some(selection) = input.snapshot.selection {
        map.insert("selection".into(), Dynamic::from(selection.0 as i64));
    }
    let entries: Array = input
        .snapshot
        .entries
        .iter()
        .map(|entry| {
            let mut item = Map::new();
            item.insert("handle".into(), Dynamic::from(entry.handle.0 as i64));
            item.insert("name".into(), Dynamic::from(entry.name.clone()));
            item.insert("is_dir".into(), Dynamic::from(entry.is_dir));
            item.insert("size".into(), Dynamic::from(entry.size.min(i64::MAX as u64) as i64));
            item.insert("kind".into(), Dynamic::from(entry.kind.clone()));
            if let Some(excerpt) = &input.excerpt
                && excerpt.handle == entry.handle
            {
                item.insert("excerpt".into(), Dynamic::from(excerpt.text.clone()));
            }
            Dynamic::from(item)
        })
        .collect();
    map.insert("entries".into(), Dynamic::from(entries));
    map
}

fn event_map(event: &HookEvent) -> Map {
    let mut map = Map::new();
    map.insert("api".into(), Dynamic::from(API_VERSION as i64));
    map.insert("kind".into(), Dynamic::from(event.kind().to_owned()));
    if let Some(scope) = event.scope() {
        map.insert("scope".into(), Dynamic::from(scope.to_owned()));
    }
    map
}

fn call_error(error: Box<EvalAltResult>, runtime: &Runtime, cancelled: bool) -> CallError {
    if runtime.violation.load(Ordering::Relaxed) {
        return CallError::capability(error.to_string());
    }
    if cancelled {
        return CallError::cancelled();
    }
    match *error {
        EvalAltResult::ErrorTooManyOperations(_) => CallError::budget("operation budget exhausted"),
        EvalAltResult::ErrorTerminated(_, _) => match runtime.terminate.load(Ordering::Relaxed) {
            2 => CallError::cancelled(),
            _ => CallError::budget("evaluation deadline exceeded"),
        },
        EvalAltResult::ErrorDataTooLarge(_, _) => CallError::invalid(error.to_string()),
        other => CallError::script(other.to_string()),
    }
}

fn runtime_error(message: impl Into<String>) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorRuntime(Dynamic::from(message.into()), Position::NONE))
}

fn send(inbox: &Sender<Inbox>, message: WorkerMessage) {
    let _ = inbox.send(Inbox::Worker(message));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{ExcerptInput, RequestInput};
    use crate::manifest::Manifest;
    use crow_core::{EntryData, EntryHandle, ExtensionId};
    use std::sync::mpsc::{Receiver, channel};

    fn limits() -> Limits {
        Limits::default()
    }

    fn manifest(assets: &[&str]) -> Manifest {
        let assets = if assets.is_empty() {
            String::new()
        } else {
            format!("assets = [{}]\n", assets.iter().map(|name| format!("\"{name}\"")).collect::<Vec<_>>().join(", "))
        };
        crate::manifest::parse_manifest(
            &format!(
                r#"
id = "example.test"
name = "Test"
version = "0.1.0"
api = 1
entry = "main.rhai"
provides = ["command"]
{assets}permissions = {{ fs = "none" }}
"#
            ),
            &limits(),
        )
        .unwrap()
    }

    fn snapshot(source: &str, assets: &[(&str, &str)]) -> Arc<crate::package::PackageSnapshot> {
        let manifest = manifest(&assets.iter().map(|(name, _)| *name).collect::<Vec<_>>());
        let mut files = BTreeMap::new();
        files.insert("plugin.toml".into(), Arc::from(&b"manifest"[..]));
        files.insert("main.rhai".into(), Arc::from(source.as_bytes()));
        for (name, content) in assets {
            files.insert((*name).to_owned(), Arc::from(content.as_bytes()));
        }
        Arc::new(crate::package::PackageSnapshot { manifest, files, digest: "a".repeat(64) })
    }

    fn input() -> RequestInput {
        RequestInput {
            snapshot: Snapshot {
                scope: "/tmp".into(),
                selection: Some(EntryHandle(1)),
                entries: vec![EntryData {
                    handle: EntryHandle(1),
                    name: "a.txt".into(),
                    is_dir: false,
                    size: 5,
                    kind: "Text".into(),
                }],
            },
            excerpt: None,
        }
    }

    fn input_with_excerpt(text: &str) -> RequestInput {
        RequestInput { excerpt: Some(ExcerptInput { handle: EntryHandle(1), text: text.into() }), ..input() }
    }

    fn next(rx: &Receiver<Inbox>, timeout: Duration) -> WorkerMessage {
        let started = Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(10)) {
                Ok(Inbox::Worker(message)) => return message,
                Ok(Inbox::Command(_)) | Ok(Inbox::Submission) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    assert!(started.elapsed() < timeout, "worker reply timed out");
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => panic!("worker channel closed"),
            }
        }
    }

    fn spawn(source: &str, assets: &[(&str, &str)], limits: Limits) -> (Worker, Receiver<Inbox>) {
        let (tx, rx) = channel();
        let worker =
            Worker::spawn(PluginId::parse("example.test").unwrap(), 1, snapshot(source, assets), limits, tx).unwrap();
        worker.mailbox.enqueue_command(Call::Register).unwrap();
        match next(&rx, Duration::from_secs(2)) {
            WorkerMessage::Registered { result: Ok(_), .. } => {}
            other => panic!("registration failed: {other:?}"),
        }
        (worker, rx)
    }

    fn command_source(body: &str) -> String {
        format!(
            r#"
fn register() {{ [ #{{ kind: "command", id: "copy", title: "Copy", function: "run" }} ] }}
fn run(ctx) {{ {body} }}
"#
        )
    }

    fn evaluate(worker: &Worker, input: RequestInput) {
        evaluate_with_cancel(worker, input, Arc::new(AtomicBool::new(false)));
    }

    fn evaluate_with_cancel(worker: &Worker, input: RequestInput, cancel: Arc<AtomicBool>) {
        let extension = ExtensionId::new(PluginId::parse("example.test").unwrap(), LocalId::parse("copy").unwrap());
        worker
            .mailbox
            .enqueue_command(Call::Evaluate {
                extension,
                input,
                stamp: RequestStamp { generation: 1, request_id: 1, context_revision: 4 },
                cancel,
            })
            .unwrap();
    }

    #[test]
    fn a_host_read_excerpt_arrives_as_entry_input() {
        let (mut worker, rx) =
            spawn(&command_source(r#"#{ kind: "copy_text", text: ctx.entries[0].excerpt }"#), &[], limits());
        evaluate(&worker, input_with_excerpt("hello"));
        match next(&rx, Duration::from_secs(2)) {
            WorkerMessage::Replied { reply: Reply::Intent(crow_core::CommandIntent::CopyText(text)), .. } => {
                assert_eq!(text, "hello");
            }
            other => panic!("{other:?}"),
        }
        worker.stop();
    }

    #[test]
    fn undeclared_asset_is_a_capability_failure() {
        let (worker, rx) =
            spawn(&command_source(r#"#{ kind: "copy_text", text: asset("missing.txt") }"#), &[], limits());
        evaluate(&worker, input());
        match next(&rx, Duration::from_secs(2)) {
            WorkerMessage::Failed { error, .. } => assert_eq!(error.kind, CallErrorKind::Capability),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn declared_asset_is_readable_and_undeclared_is_not() {
        let (mut worker, rx) = spawn(
            &command_source(r#"#{ kind: "copy_text", text: asset("note.txt") }"#),
            &[("note.txt", "declared")],
            limits(),
        );
        evaluate(&worker, input());
        match next(&rx, Duration::from_secs(2)) {
            WorkerMessage::Replied { reply: Reply::Intent(crow_core::CommandIntent::CopyText(text)), .. } => {
                assert_eq!(text, "declared");
            }
            other => panic!("{other:?}"),
        }
        worker.stop();
    }

    #[test]
    fn deadline_and_operation_budget_terminate_and_report_budget() {
        let (worker, rx) = spawn(&command_source("while true { }"), &[], {
            let mut limits = limits();
            limits.max_operations = 0;
            limits.call_deadline = Duration::from_millis(20);
            limits
        });
        evaluate(&worker, input());
        match next(&rx, Duration::from_secs(2)) {
            WorkerMessage::Failed { error, .. } => assert_eq!(error.kind, CallErrorKind::Budget),
            other => panic!("{other:?}"),
        }

        let (worker, rx) = spawn(&command_source("while true { }"), &[], {
            let mut limits = limits();
            limits.max_operations = 2_000;
            limits.call_deadline = Duration::from_secs(5);
            limits
        });
        evaluate(&worker, input());
        match next(&rx, Duration::from_secs(2)) {
            WorkerMessage::Failed { error, .. } => assert_eq!(error.kind, CallErrorKind::Budget),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn per_call_cancel_stops_the_evaluation_and_keeps_the_worker() {
        let (mut worker, rx) = spawn(
            &command_source("while true { }"),
            &[],
            Limits { max_operations: 0, call_deadline: Duration::from_secs(5), ..limits() },
        );
        let cancel = Arc::new(AtomicBool::new(false));
        evaluate_with_cancel(&worker, input(), cancel.clone());
        std::thread::sleep(Duration::from_millis(10));
        cancel.store(true, Ordering::Relaxed);
        match next(&rx, Duration::from_secs(2)) {
            WorkerMessage::Failed { error, .. } => assert_eq!(error.kind, CallErrorKind::Cancelled),
            other => panic!("{other:?}"),
        }
        worker.stop();
    }

    #[test]
    fn mailbox_coalesces_refreshes_and_prioritizes_commands() {
        let mut limits = limits();
        limits.command_mailbox = 1;
        limits.refresh_mailbox = 2;
        let mailbox = Mailbox::new(&limits);
        let extension = ExtensionId::new(PluginId::parse("example.test").unwrap(), LocalId::parse("copy").unwrap());
        let notify = |request_id| Call::Notify {
            extension: extension.clone(),
            event: crow_core::HookEvent::FilesChanged { scope: "/tmp".into() },
            stamp: RequestStamp { generation: 1, request_id, context_revision: 1 },
            cancel: Arc::new(AtomicBool::new(false)),
        };
        assert!(matches!(mailbox.enqueue_refresh(notify(1)), RefreshEnqueue::Queued));
        match mailbox.enqueue_refresh(notify(2)) {
            RefreshEnqueue::Replaced(Call::Notify { stamp, .. }) => {
                assert_eq!(stamp.request_id, 1, "the host receives the displaced call");
            }
            _ => panic!("a coalesced hook must return the displaced call"),
        }
        let cancel = AtomicBool::new(false);
        match mailbox.recv(&cancel).unwrap() {
            Call::Notify { stamp, .. } => assert_eq!(stamp.request_id, 2, "same key keeps the newest stamp"),
            other => panic!("{other:?}"),
        }
        cancel.store(true, Ordering::Relaxed);
        assert!(mailbox.recv(&cancel).is_none(), "one refresh was coalesced away");

        let cancel = AtomicBool::new(false);
        mailbox.enqueue_refresh(notify(3));
        mailbox
            .enqueue_command(Call::Evaluate {
                extension,
                input: input(),
                stamp: RequestStamp { generation: 1, request_id: 4, context_revision: 1 },
                cancel: Arc::new(AtomicBool::new(false)),
            })
            .unwrap();
        match mailbox.recv(&cancel).unwrap() {
            Call::Evaluate { stamp, .. } => assert_eq!(stamp.request_id, 4, "commands go first"),
            other => panic!("{other:?}"),
        }
        assert!(mailbox.enqueue_command(Call::Register).is_ok());
        assert_eq!(mailbox.enqueue_command(Call::Register), Err(EnqueueError::Busy));
    }
}

/// A deterministic test-only barrier callable from a fixture script. It holds
/// the worker inside a host function so the test can saturate the command lane
/// without racing a short script.
#[cfg(test)]
pub(crate) mod test_gate {
    use std::collections::HashMap;
    use std::sync::{Arc, Condvar, LazyLock, Mutex};
    use std::time::{Duration, Instant};

    struct Gate {
        entered: bool,
        released: bool,
    }

    type GateHandle = Arc<(Mutex<Gate>, Condvar)>;
    type GateMap = Mutex<HashMap<String, GateHandle>>;

    fn gates() -> &'static GateMap {
        static GATES: LazyLock<GateMap> = LazyLock::new(|| Mutex::new(HashMap::new()));
        &GATES
    }

    fn gate(name: &str) -> GateHandle {
        gates()
            .lock()
            .unwrap()
            .entry(name.to_owned())
            .or_insert_with(|| Arc::new((Mutex::new(Gate { entered: false, released: false }), Condvar::new())))
            .clone()
    }

    pub(crate) fn hold(name: &str) {
        let pair = gate(name);
        let (lock, ready) = &*pair;
        let mut state = lock.lock().unwrap();
        state.entered = true;
        ready.notify_all();
        while !state.released {
            state = ready.wait(state).unwrap();
        }
    }

    pub(crate) fn wait_until_entered(name: &str) {
        let pair = gate(name);
        let (lock, ready) = &*pair;
        let mut state = lock.lock().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !state.entered {
            assert!(Instant::now() < deadline, "worker never reached gate {name}");
            let (next, _) = ready.wait_timeout(state, Duration::from_millis(10)).unwrap();
            state = next;
        }
    }

    pub(crate) fn release(name: &str) {
        let pair = gate(name);
        let (lock, ready) = &*pair;
        lock.lock().unwrap().released = true;
        ready.notify_all();
    }
}
