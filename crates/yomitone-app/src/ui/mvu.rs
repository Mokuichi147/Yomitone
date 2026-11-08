//! naui の UI タスクキューを使う、小さな Model–View–Update ランタイム。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use naui::{Result, Sender, Settings, Tasks, Timer, Ui};

pub(crate) trait Program: 'static {
    type Message: Send + 'static;
    type View: 'static;

    fn update(&mut self, message: Self::Message) -> Effect<Self::Message>;
    fn render(&self, view: &Self::View);
}

/// UI コールバックからモデルへ送る、型付きメッセージの送り先。
pub(crate) struct Dispatch<M: Send + 'static>(Sender<M>);

impl<M: Send + 'static> Clone for Dispatch<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<M: Send + 'static> Dispatch<M> {
    pub(crate) fn send(&self, message: M) {
        let _ = self.0.send(message);
    }

    pub(crate) fn to(&self, mut make: impl FnMut() -> M + 'static) -> impl FnMut() + 'static {
        let dispatch = self.clone();
        move || dispatch.send(make())
    }

    pub(crate) fn with<T: ?Sized>(&self, mut make: impl FnMut(&T) -> M + 'static) -> impl FnMut(&T) + 'static {
        let dispatch = self.clone();
        move |value| dispatch.send(make(value))
    }

    pub(crate) fn map<T>(&self, mut make: impl FnMut(T) -> M + 'static) -> impl FnMut(T) + 'static {
        let dispatch = self.clone();
        move |value| dispatch.send(make(value))
    }
}

enum Command<M: Send + 'static> {
    Perform(Box<dyn FnOnce() + 'static>),
    Thread(Box<dyn FnOnce() -> M + Send + 'static>),
    Channel(Box<dyn FnOnce(Dispatch<M>) + 'static>),
    Every(&'static str, Duration, Box<dyn FnMut() -> M + 'static>),
    Cancel(&'static str),
}

/// `update` が要求する非同期処理。起動は `render` の後に行う。
pub(crate) struct Effect<M: Send + 'static> {
    commands: Vec<Command<M>>,
}

impl<M: Send + 'static> Effect<M> {
    pub(crate) fn none() -> Self {
        Self { commands: Vec::new() }
    }

    pub(crate) fn thread(work: impl FnOnce() -> M + Send + 'static) -> Self {
        Self { commands: vec![Command::Thread(Box::new(work))] }
    }

    pub(crate) fn perform(work: impl FnOnce() + 'static) -> Self {
        Self { commands: vec![Command::Perform(Box::new(work))] }
    }

    /// UI スレッドで `start` を呼び、送り先を渡す。複数回進捗を送る処理向け。
    pub(crate) fn channel(start: impl FnOnce(Dispatch<M>) + 'static) -> Self {
        Self { commands: vec![Command::Channel(Box::new(start))] }
    }

    pub(crate) fn every(key: &'static str, interval: Duration, mut make: impl FnMut() -> M + 'static) -> Self {
        Self { commands: vec![Command::Every(key, interval, Box::new(move || make()))] }
    }

    pub(crate) fn cancel(key: &'static str) -> Self {
        Self { commands: vec![Command::Cancel(key)] }
    }

    pub(crate) fn batch(effects: impl IntoIterator<Item = Self>) -> Self {
        Self { commands: effects.into_iter().flat_map(|effect| effect.commands).collect() }
    }
}

struct Running<P: Program> {
    model: P,
    view: P::View,
    tasks: Tasks,
    timers: HashMap<&'static str, Timer>,
}

impl<P: Program> Drop for Running<P> {
    fn drop(&mut self) {
        for timer in self.timers.values() {
            timer.cancel();
        }
    }
}

pub(crate) fn run<P, F>(settings: Settings, model: P, build: F) -> Result<()>
where
    P: Program,
    F: FnOnce(&Ui, &Dispatch<P::Message>) -> Result<P::View> + 'static,
{
    let state: Rc<RefCell<Option<Running<P>>>> = Rc::default();
    let build_state = state.clone();
    naui::run(settings, move |ui| {
        let weak = Rc::downgrade(&build_state);
        let sender = ui.tasks().channel(move |message| deliver(&weak, message));
        let dispatch = Dispatch(sender);
        let view = build(ui, &dispatch)?;
        let running = Running { model, view, tasks: ui.tasks(), timers: HashMap::new() };
        running.model.render(&running.view);
        *build_state.borrow_mut() = Some(running);
        Ok(())
    })
}

fn deliver<P: Program>(weak: &Weak<RefCell<Option<Running<P>>>>, message: P::Message) {
    let Some(state) = weak.upgrade() else {
        return;
    };
    let effect = {
        let mut slot = state.borrow_mut();
        let Some(running) = slot.as_mut() else {
            return;
        };
        let effect = running.model.update(message);
        running.model.render(&running.view);
        effect
    };
    run_effects(weak, effect);
}

fn run_effects<P: Program>(weak: &Weak<RefCell<Option<Running<P>>>>, effect: Effect<P::Message>) {
    let Some(state) = weak.upgrade() else {
        return;
    };
    for command in effect.commands {
        let mut slot = state.borrow_mut();
        let Some(running) = slot.as_mut() else {
            return;
        };
        match command {
            Command::Perform(work) => work(),
            Command::Thread(work) => {
                let sender = message_sender(&running.tasks, weak.clone());
                let started = std::thread::Builder::new().name("yomitone mvu effect".into()).spawn(move || {
                    let _ = sender.send(work());
                });
                if let Err(error) = started {
                    eprintln!("バックグラウンド処理を開始できませんでした: {error}");
                }
            }
            Command::Channel(start) => {
                let sender = message_sender(&running.tasks, weak.clone());
                start(Dispatch(sender));
            }
            Command::Every(key, interval, mut make) => {
                if let Some(previous) = running.timers.remove(key) {
                    previous.cancel();
                }
                let sender = message_sender(&running.tasks, weak.clone());
                let timer = running.tasks.every(interval, move || {
                    let _ = sender.send(make());
                });
                running.timers.insert(key, timer);
            }
            Command::Cancel(key) => {
                if let Some(timer) = running.timers.remove(key) {
                    timer.cancel();
                }
            }
        }
    }
}

fn message_sender<P: Program>(tasks: &Tasks, weak: Weak<RefCell<Option<Running<P>>>>) -> Sender<P::Message> {
    tasks.channel(move |message| deliver(&weak, message))
}
