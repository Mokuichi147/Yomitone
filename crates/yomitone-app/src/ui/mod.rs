//! naui を使った Model–View–Update 構成と画面の組み立て。

mod model;
mod mvu;
mod parts;
mod results_page;
mod run_page;
mod settings_pages;

use std::cell::Cell;
use std::time::Duration;

use naui::{
    CloseResponse, Grid, GridCell, Result, Settings, Sidebar, SidebarItem, SidebarSection, Toast, ToolbarIcon, Track,
    Ui, Widget, Window,
};
use yomitone::diagnostics::{check_jev, check_llm, check_tts};
use yomitone::setup::SetupError;

use self::model::{AppModel, CheckTarget, Message, LOG_EFFECT};
use self::mvu::{Dispatch, Effect, Program};
use self::parts::{feedback, scrollable};
use self::results_page::ResultsView;
use self::run_page::RunPageView;
use self::settings_pages::SettingsViews;
use crate::credentials::Credentials;
use crate::settings::{SpeakerBackend, TtsBackendSetting};
use crate::worker::{self, Event, Outcome};

const LOG_REFRESH: Duration = Duration::from_millis(200);
const SIDEBAR_WIDTH: f64 = 250.0;
const PAGE_COUNT: usize = 6;

const MAIN_PAGES: [(&str, ToolbarIcon); 2] = [("読み上げ", ToolbarIcon::Open), ("結果", ToolbarIcon::Info)];
const SETTINGS_PAGES: [(&str, ToolbarIcon); 4] = [
    (SpeakerBackend::Llm.page_title(), ToolbarIcon::Settings),
    (SpeakerBackend::Jev.page_title(), ToolbarIcon::Search),
    (TtsBackendSetting::Openai.page_title(), ToolbarIcon::Share),
    (TtsBackendSetting::Wyoming.page_title(), ToolbarIcon::Share),
];

fn sidebar_sections() -> Vec<SidebarSection> {
    let items = |pages: &[(&str, ToolbarIcon)]| {
        pages.iter().map(|(title, icon)| SidebarItem::new(*title).icon(*icon)).collect::<Vec<_>>()
    };
    vec![SidebarSection::untitled(items(&MAIN_PAGES)), SidebarSection::new("設定", items(&SETTINGS_PAGES))]
}

/// naui を起動し、AppModel をメッセージキュー経由で更新する。
pub fn run() -> Result<()> {
    mvu::run(Settings::new("Yomitone").app_id("io.github.mokuichi147.yomitone"), AppModel::load(), build)
}

struct AppView {
    window: Window,
    root: Grid,
    sidebar: Sidebar,
    screens: Vec<Box<dyn Widget>>,
    displayed_page: Cell<usize>,
    run: RunPageView,
    results: ResultsView,
    settings: SettingsViews,
    toast: Toast,
    shown_notice: Cell<u64>,
    closed: Cell<bool>,
}

fn build(ui: &Ui, dispatch: &Dispatch<Message>) -> Result<AppView> {
    let window = ui.window("Yomitone", 980.0, 720.0)?;
    window.set_min_size(760.0, 540.0);

    let root = ui.grid()?;
    root.set_column_track(0, Track::FILL);
    root.set_row_track(0, Track::FILL);

    let (run_page, run_view) = run_page::build(ui, dispatch)?;
    let (results_page, results_view) = results_page::build(ui)?;
    let (settings_pages, settings_view) = settings_pages::build(ui, dispatch)?;
    let mut screens: Vec<Box<dyn Widget>> = vec![Box::new(run_page), Box::new(scrollable(ui, &results_page)?)];
    for page in &settings_pages {
        screens.push(Box::new(scrollable(ui, page)?));
    }

    root.replace(screens[0].as_ref(), GridCell::new(0, 0));
    let sidebar = ui.sidebar()?;
    sidebar.set_sections(&sidebar_sections());
    sidebar.set_width(SIDEBAR_WIDTH);
    sidebar.set_selected(0);
    sidebar.on_select(dispatch.map(Message::PageSelected));
    window.on_close_request({
        let dispatch = dispatch.clone();
        move || {
            // close 要求もメッセージにして、読み上げ中なら停止後に閉じる。
            dispatch.send(Message::CloseRequested);
            CloseResponse::KeepOpen
        }
    });
    window.set_child(&root);
    window.set_sidebar(&sidebar);
    window.show();

    Ok(AppView {
        window,
        root,
        sidebar,
        screens,
        displayed_page: Cell::new(0),
        run: run_view,
        results: results_view,
        settings: settings_view,
        toast: ui.toast("")?,
        shown_notice: Cell::new(0),
        closed: Cell::new(false),
    })
}

impl Program for AppModel {
    type Message = Message;
    type View = AppView;

    fn update(&mut self, message: Message) -> Effect<Message> {
        match message {
            Message::PageSelected(page) => {
                if page < PAGE_COUNT {
                    self.page = page;
                }
                Effect::none()
            }
            Message::TextChanged(field, value) => self.apply_text(field, value),
            Message::ApiKeyChanged(key, value) => {
                self.apply_api_key(key, value);
                Effect::none()
            }
            Message::NumberChanged(field, value) => self.apply_number(field, value),
            Message::ExportAudioChanged(enabled) => {
                self.settings.export_audio = enabled;
                self.persist_effect()
            }
            Message::SpeakerBackendChanged(backend) => {
                self.settings.speaker_backend = backend;
                self.persist_effect()
            }
            Message::TtsBackendChanged(backend) => {
                self.settings.tts_backend = backend;
                self.persist_effect()
            }
            Message::Start => self.start(),
            Message::Stop => {
                self.stop();
                Effect::none()
            }
            Message::CloseRequested => {
                self.close_requested = true;
                self.stop();
                Effect::none()
            }
            Message::WorkerEvent(event) => self.worker_event(event),
            Message::FlushLog => {
                self.run.flush_log();
                Effect::none()
            }
            Message::CheckConnection(target) => self.check_connection(target),
            Message::CheckFinished(target, message) => {
                if self.checking == Some(target) {
                    self.checking = None;
                }
                self.notify(message);
                Effect::none()
            }
            Message::LoadApiKey(key) => {
                Effect::thread(move || Message::ApiKeyLoaded(key, Credentials::new().read(key)))
            }
            Message::SaveApiKey(key) => {
                let value = key.value(&self.settings).to_owned();
                Effect::thread(move || Message::ApiKeySaved(key, Credentials::new().save(key, &value)))
            }
            Message::DeleteApiKey(key) => {
                Effect::thread(move || Message::ApiKeyDeleted(key, Credentials::new().delete(key)))
            }
            Message::ApiKeyLoaded(key, Ok(Some(value))) => {
                self.set_loaded_api_key(key, value);
                self.notify("API キーを読み込みました");
                Effect::none()
            }
            Message::ApiKeyLoaded(key, Ok(None)) => {
                self.set_loaded_api_key(key, String::new());
                self.notify("保存済みの API キーはありません");
                Effect::none()
            }
            Message::ApiKeyLoaded(key, Err(error)) => {
                self.set_credential_error(key, Some(error.clone()));
                self.notify(&error);
                Effect::none()
            }
            Message::ApiKeySaved(key, Ok(())) => {
                self.set_credential_error(key, None);
                self.notify("API キーを保存しました");
                Effect::none()
            }
            Message::ApiKeySaved(_, Err(error)) | Message::ApiKeyDeleted(_, Err(error)) => {
                self.notify(error);
                Effect::none()
            }
            Message::ApiKeyDeleted(key, Ok(())) => {
                self.set_loaded_api_key(key, String::new());
                self.notify("API キーを削除しました");
                Effect::none()
            }
            Message::FetchVoices => self.fetch_voices(),
            Message::VoiceListLoaded(Ok(voices)) => {
                self.voice_fetching = false;
                self.fetched_voices = voices;
                if self.fetched_voices.is_empty() {
                    self.voice_list_text.clear();
                    self.notify("言語に一致する音声がありません");
                } else {
                    self.voice_list_text = self.fetched_voices.join(", ");
                    self.notify("音声一覧を取得しました");
                }
                Effect::none()
            }
            Message::VoiceListLoaded(Err(error)) => {
                self.voice_fetching = false;
                self.fetched_voices.clear();
                self.voice_list_text.clear();
                self.notify(error);
                Effect::none()
            }
            Message::ApplyVoices => {
                if !self.fetched_voices.is_empty() {
                    self.settings.wyoming_tts.voices = self.fetched_voices.join(", ");
                    self.notify("使用する音声に設定しました");
                    return self.persist_effect();
                }
                Effect::none()
            }
        }
    }

    fn render(&self, view: &AppView) {
        view.run.render(self);
        view.results.render(&self.run.characters, &self.run.voices);
        view.settings.render(self);

        if view.displayed_page.get() != self.page {
            if let Some(screen) = view.screens.get(self.page) {
                view.root.replace(screen.as_ref(), GridCell::new(0, 0));
                view.sidebar.set_selected(self.page);
                view.displayed_page.set(self.page);
            }
        }

        if let Some((id, message)) = &self.notice {
            if view.shown_notice.get() != *id {
                feedback(&view.toast, message);
                view.shown_notice.set(*id);
            }
        }

        if self.close_requested && self.run.cancel.is_none() && !view.closed.replace(true) {
            view.window.close();
        }
    }
}

impl AppModel {
    fn start(&mut self) -> Effect<Message> {
        if self.run.cancel.is_some() || self.close_requested {
            return Effect::none();
        }
        if let Err(message) = self.settings.validate() {
            self.notify(message);
            return Effect::none();
        }

        let settings = self.settings.clone();
        let cancel = yomitone::application::cancel::CancelFlag::new();
        self.run.clear();
        self.run.cancel = Some(cancel.clone());
        let worker = Effect::channel(move |dispatch| {
            let worker_dispatch = dispatch.clone();
            let started = std::thread::Builder::new().name("yomitone narration".into()).spawn(move || {
                worker::run(settings, cancel, move |event| {
                    worker_dispatch.send(Message::WorkerEvent(event));
                });
            });
            if let Err(error) = started {
                dispatch.send(Message::WorkerEvent(Event::Done(Outcome::Failed(format!(
                    "読み上げ処理を開始できませんでした: {error}"
                )))));
            }
        });
        let refresh_log = Effect::every(LOG_EFFECT, LOG_REFRESH, || Message::FlushLog);
        Effect::batch([worker, refresh_log])
    }

    fn stop(&mut self) {
        if let Some(cancel) = &self.run.cancel {
            cancel.cancel();
            self.run.stopping = true;
        }
    }

    fn worker_event(&mut self, event: Event) -> Effect<Message> {
        match event {
            Event::Log(line) => self.run.push_log(line),
            Event::Loaded { counts, synthesize } => {
                self.run.progress = crate::progress::Progress::new(counts, synthesize);
            }
            Event::ChapterStarted { index, characters } => {
                self.run.progress.chapter_started(index);
                self.run.characters = characters;
            }
            Event::SentenceEstimated => self.run.progress.sentence_estimated(),
            Event::SynthesisProgressed { done, .. } => self.run.progress.synthesis_progressed(done),
            Event::Finished { characters, voices } => {
                self.run.characters = characters;
                self.run.voices = voices.unwrap_or_default();
            }
            Event::Done(outcome) => {
                self.finish_run(outcome);
                return Effect::cancel(LOG_EFFECT);
            }
        }
        Effect::none()
    }

    fn check_connection(&mut self, target: CheckTarget) -> Effect<Message> {
        if self.checking.is_some() {
            return Effect::none();
        }
        self.checking = Some(target);
        let settings = self.settings.clone();
        Effect::thread(move || {
            let message = match target {
                CheckTarget::Llm => check_llm(&settings.llm_options()).message().to_owned(),
                CheckTarget::Jev => check_jev(&settings.jev_options()).message().to_owned(),
                CheckTarget::Tts(backend) => {
                    let options = settings.tts_server(backend).options(backend.backend());
                    check_tts(&options, |error: &SetupError| worker::describe_setup_error(error, backend))
                        .message()
                        .to_owned()
                }
            };
            Message::CheckFinished(target, message)
        })
    }

    fn fetch_voices(&mut self) -> Effect<Message> {
        if self.voice_fetching {
            return Effect::none();
        }
        self.voice_fetching = true;
        self.voice_list_text.clear();
        self.fetched_voices.clear();
        let settings = self.settings.clone();
        Effect::thread(move || {
            let result = settings_pages::fetch_wyoming_voices(&settings);
            Message::VoiceListLoaded(result)
        })
    }
}
