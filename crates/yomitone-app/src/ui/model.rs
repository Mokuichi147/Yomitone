//! アプリの状態と、画面から送るメッセージ。

use std::collections::VecDeque;
use std::path::PathBuf;

use yomitone::application::cancel::CancelFlag;
use yomitone::domain::character::Character;

use super::mvu::Effect;
use crate::credentials::{ApiKey, Credentials};
use crate::progress::Progress;
use crate::settings::{self, AppSettings, SpeakerBackend, TtsBackendSetting};
use crate::worker::{Event, Outcome};

pub(super) const LOG_EFFECT: &str = "narration-log";
pub(super) const MAX_LOG_LINES: usize = 3000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TextField {
    InputFolder,
    OutputFolder,
    LlmHost,
    LlmPort,
    LlmModel,
    JevModel,
    JevBaseUrl,
    TtsUrl(TtsBackendSetting),
    TtsModel,
    TtsLanguage,
    TtsVoices(TtsBackendSetting),
    TtsNarratorVoice(TtsBackendSetting),
    TtsMaleVoices(TtsBackendSetting),
    TtsFemaleVoices(TtsBackendSetting),
}

impl TextField {
    pub(super) fn value(self, settings: &AppSettings) -> &str {
        match self {
            Self::InputFolder => &settings.input_folder,
            Self::OutputFolder => &settings.output_folder,
            Self::LlmHost => &settings.llm_host,
            Self::LlmPort => &settings.llm_port,
            Self::LlmModel => &settings.llm_model,
            Self::JevModel => &settings.jev_model,
            Self::JevBaseUrl => &settings.jev_base_url,
            Self::TtsUrl(backend) => &settings.tts_server(backend).url,
            Self::TtsModel => &settings.openai_tts.model,
            Self::TtsLanguage => &settings.wyoming_tts.language,
            Self::TtsVoices(backend) => &settings.tts_server(backend).voices,
            Self::TtsNarratorVoice(backend) => &settings.tts_server(backend).narrator_voice,
            Self::TtsMaleVoices(backend) => &settings.tts_server(backend).male_voices,
            Self::TtsFemaleVoices(backend) => &settings.tts_server(backend).female_voices,
        }
    }

    fn set(self, settings: &mut AppSettings, value: String) {
        match self {
            Self::InputFolder => settings.input_folder = value,
            Self::OutputFolder => settings.output_folder = value,
            Self::LlmHost => settings.llm_host = value,
            Self::LlmPort => settings.llm_port = value,
            Self::LlmModel => settings.llm_model = value,
            Self::JevModel => settings.jev_model = value,
            Self::JevBaseUrl => settings.jev_base_url = value,
            Self::TtsUrl(backend) => settings.tts_server_mut(backend).url = value,
            Self::TtsModel => settings.openai_tts.model = value,
            Self::TtsLanguage => settings.wyoming_tts.language = value,
            Self::TtsVoices(backend) => settings.tts_server_mut(backend).voices = value,
            Self::TtsNarratorVoice(backend) => settings.tts_server_mut(backend).narrator_voice = value,
            Self::TtsMaleVoices(backend) => settings.tts_server_mut(backend).male_voices = value,
            Self::TtsFemaleVoices(backend) => settings.tts_server_mut(backend).female_voices = value,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum NumberField {
    Timeout(TtsBackendSetting),
    MaxChars(TtsBackendSetting),
    Pause(TtsBackendSetting),
}

impl NumberField {
    pub(super) fn value(self, settings: &AppSettings) -> f64 {
        let tts = settings.tts_server(match self {
            Self::Timeout(backend) | Self::MaxChars(backend) | Self::Pause(backend) => backend,
        });
        match self {
            Self::Timeout(_) => tts.timeout,
            Self::MaxChars(_) => tts.max_chars as f64,
            Self::Pause(_) => tts.pause,
        }
    }

    fn set(self, settings: &mut AppSettings, value: f64) {
        let tts = settings.tts_server_mut(match self {
            Self::Timeout(backend) | Self::MaxChars(backend) | Self::Pause(backend) => backend,
        });
        match self {
            Self::Timeout(_) => tts.timeout = value,
            Self::MaxChars(_) => tts.max_chars = value.max(0.0) as usize,
            Self::Pause(_) => tts.pause = value,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CheckTarget {
    Llm,
    Jev,
    Tts(TtsBackendSetting),
}

pub(super) enum Message {
    PageSelected(usize),
    TextChanged(TextField, String),
    ApiKeyChanged(ApiKey, String),
    NumberChanged(NumberField, f64),
    ExportAudioChanged(bool),
    SpeakerBackendChanged(SpeakerBackend),
    TtsBackendChanged(TtsBackendSetting),
    Start,
    Stop,
    CloseRequested,
    WorkerEvent(Event),
    FlushLog,
    CheckConnection(CheckTarget),
    CheckFinished(CheckTarget, String),
    LoadApiKey(ApiKey),
    SaveApiKey(ApiKey),
    DeleteApiKey(ApiKey),
    ApiKeyLoaded(ApiKey, Result<Option<String>, String>),
    ApiKeySaved(ApiKey, Result<(), String>),
    ApiKeyDeleted(ApiKey, Result<(), String>),
    FetchVoices,
    VoiceListLoaded(Result<Vec<String>, String>),
    ApplyVoices,
}

pub(super) struct RunModel {
    pub(super) cancel: Option<CancelFlag>,
    pub(super) stopping: bool,
    pub(super) progress: Progress,
    log_lines: VecDeque<String>,
    pub(super) log_text: String,
    pub(super) log_revision: u64,
    pub(super) characters: Vec<Character>,
    pub(super) voices: Vec<(String, String)>,
    pub(super) complete: bool,
}

impl Default for RunModel {
    fn default() -> Self {
        Self {
            cancel: None,
            stopping: false,
            progress: Progress::default(),
            log_lines: VecDeque::new(),
            log_text: String::new(),
            log_revision: 0,
            characters: Vec::new(),
            voices: Vec::new(),
            complete: false,
        }
    }
}

impl RunModel {
    pub(super) fn clear(&mut self) {
        self.progress = Progress::default();
        self.log_lines.clear();
        self.log_text.clear();
        self.log_revision = self.log_revision.wrapping_add(1);
        self.characters.clear();
        self.voices.clear();
        self.complete = false;
        self.stopping = false;
    }

    pub(super) fn push_log(&mut self, line: String) {
        self.log_lines.push_back(line);
        while self.log_lines.len() > MAX_LOG_LINES {
            self.log_lines.pop_front();
        }
    }

    pub(super) fn flush_log(&mut self) {
        let text = self.log_lines.iter().map(String::as_str).collect::<Vec<_>>().join("\n");
        if self.log_text != text {
            self.log_text = text;
            self.log_revision = self.log_revision.wrapping_add(1);
        }
    }
}

pub(super) struct AppModel {
    pub(super) settings: AppSettings,
    settings_path: Option<PathBuf>,
    pub(super) credential_errors: [Option<String>; 3],
    pub(super) page: usize,
    pub(super) run: RunModel,
    pub(super) notice: Option<(u64, String)>,
    notice_id: u64,
    pub(super) checking: Option<CheckTarget>,
    pub(super) voice_fetching: bool,
    pub(super) voice_list_text: String,
    pub(super) fetched_voices: Vec<String>,
    pub(super) close_requested: bool,
}

impl AppModel {
    pub(super) fn load() -> Self {
        let settings_path = settings::default_path();
        let mut settings = settings_path.as_deref().map(settings::load).unwrap_or_default();
        let credentials = Credentials::new();
        let credential_errors = credentials.load(&mut settings);
        Self {
            settings,
            settings_path,
            credential_errors,
            page: 0,
            run: RunModel::default(),
            notice: None,
            notice_id: 0,
            checking: None,
            voice_fetching: false,
            voice_list_text: String::new(),
            fetched_voices: Vec::new(),
            close_requested: false,
        }
    }

    pub(super) fn notify(&mut self, message: impl Into<String>) {
        self.notice_id = self.notice_id.wrapping_add(1);
        self.notice = Some((self.notice_id, message.into()));
    }

    pub(super) fn persist_effect(&self) -> Effect<Message> {
        let Some(path) = self.settings_path.clone() else {
            return Effect::none();
        };
        let settings = self.settings.clone();
        Effect::perform(move || {
            if let Err(error) = settings::save(&path, &settings) {
                eprintln!("設定を保存できませんでした({}): {error}", path.display());
            }
        })
    }

    pub(super) fn finish_run(&mut self, outcome: Outcome) {
        self.run.cancel = None;
        self.run.stopping = false;
        match outcome {
            Outcome::Completed => {
                self.run.complete = true;
                self.notify("完了しました");
                self.run.push_log("完了しました".into());
            }
            Outcome::PartiallyFailed(sources) => {
                self.run.complete = true;
                let names: Vec<String> = sources
                    .iter()
                    .map(|source| {
                        std::path::Path::new(source)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| source.clone())
                    })
                    .collect();
                let message = format!("完了しました。音声合成に失敗した文がある章: {}", names.join(", "));
                self.notify(&message);
                self.run.push_log(message);
            }
            Outcome::Cancelled => {
                self.notify("中止しました");
                self.run.push_log("中止しました".into());
            }
            Outcome::Failed(error) => {
                self.notify(&error);
                self.run.push_log(error);
            }
        }
        self.run.flush_log();
    }
}

impl AppModel {
    pub(super) fn apply_text(&mut self, field: TextField, value: String) -> Effect<Message> {
        field.set(&mut self.settings, value);
        self.persist_effect()
    }

    pub(super) fn apply_number(&mut self, field: NumberField, value: f64) -> Effect<Message> {
        field.set(&mut self.settings, value);
        self.persist_effect()
    }

    pub(super) fn apply_api_key(&mut self, key: ApiKey, value: String) {
        key.set(&mut self.settings, value);
    }

    pub(super) fn set_loaded_api_key(&mut self, key: ApiKey, value: String) {
        key.set(&mut self.settings, value);
        self.credential_errors[key.index()] = None;
    }

    pub(super) fn set_credential_error(&mut self, key: ApiKey, message: Option<String>) {
        self.credential_errors[key.index()] = message;
    }
}
