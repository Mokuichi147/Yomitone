//! 接続先ごとの設定画面。

use naui::{NumberInput, Result, Stack, TextInput, Ui};
use yomitone::adapters::wyoming::WyomingClient;
use yomitone::application::ports::SpeechSynthesizer;
use yomitone::setup::{self, DEFAULT_LLM_HOST, DEFAULT_LLM_MODEL, DEFAULT_LLM_PORT, MAX_TTS_PAUSE};

use super::model::{AppModel, CheckTarget, Message, NumberField, TextField};
use super::mvu::Dispatch;
use super::parts::{
    check_section, heading, note, number_field, page, secret_field, text_field, title, Form, SecretField,
};
use crate::credentials::ApiKey;
use crate::settings::{AppSettings, SpeakerBackend, TtsBackendSetting};

#[derive(Default)]
pub(crate) struct SettingsViews {
    texts: Vec<(TextField, TextInput)>,
    numbers: Vec<(NumberField, NumberInput)>,
    secrets: Vec<SecretField>,
    checks: Vec<(CheckTarget, naui::Button)>,
    voice_list: Option<VoiceListView>,
}

impl SettingsViews {
    pub(crate) fn render(&self, model: &AppModel) {
        for (field, input) in &self.texts {
            input.set_text(field.value(&model.settings));
        }
        for (field, input) in &self.numbers {
            input.set_value(field.value(&model.settings));
        }
        for secret in &self.secrets {
            secret.render(model);
        }
        for (target, button) in &self.checks {
            let checking = model.checking == Some(*target);
            button.set_enabled(model.checking.is_none());
            button.set_text(if checking { "確認中…" } else { "接続を確認" });
        }
        if let Some(voices) = &self.voice_list {
            voices.fetch.set_enabled(!model.voice_fetching);
            voices.fetch.set_text(if model.voice_fetching { "取得中…" } else { "音声一覧を取得" });
            voices.apply.set_enabled(!model.fetched_voices.is_empty());
            voices.result.set_text(&model.voice_list_text);
        }
    }
}

struct VoiceListView {
    fetch: naui::Button,
    apply: naui::Button,
    result: naui::Label,
}

fn tracked_text(
    ui: &Ui,
    dispatch: &Dispatch<Message>,
    views: &mut SettingsViews,
    field: TextField,
    placeholder: &str,
) -> Result<TextInput> {
    let input = text_field(ui, dispatch, field, placeholder)?;
    views.texts.push((field, input.clone()));
    Ok(input)
}

fn tracked_secret(
    ui: &Ui,
    dispatch: &Dispatch<Message>,
    views: &mut SettingsViews,
    key: ApiKey,
    placeholder: &str,
) -> Result<SecretField> {
    let field = secret_field(ui, dispatch, key, placeholder)?;
    views.secrets.push(field.clone());
    Ok(field)
}

fn tracked_check(
    ui: &Ui,
    dispatch: &Dispatch<Message>,
    views: &mut SettingsViews,
    page: &Stack,
    description: &str,
    target: CheckTarget,
) -> Result<()> {
    let button = check_section(ui, dispatch, page, description, target)?;
    views.checks.push((target, button));
    Ok(())
}

pub(crate) fn build(ui: &Ui, dispatch: &Dispatch<Message>) -> Result<(Vec<Stack>, SettingsViews)> {
    let mut views = SettingsViews::default();
    let llm = llm(ui, dispatch, &mut views)?;
    let jev = jev(ui, dispatch, &mut views)?;
    let openai = tts(ui, dispatch, &mut views, TtsBackendSetting::Openai)?;
    let wyoming = tts(ui, dispatch, &mut views, TtsBackendSetting::Wyoming)?;
    Ok((vec![llm, jev, openai, wyoming], views))
}

fn llm(ui: &Ui, dispatch: &Dispatch<Message>, views: &mut SettingsViews) -> Result<Stack> {
    let page = page(ui)?;
    page.append(&title(ui, "LLM")?);
    page.append(&note(
        ui,
        "登場人物の抽出と、「読み上げ」で話者推測に LLM を選んだときの推測に使う、OpenAI API 互換のサーバーです。\
         JSON Schema による構造化出力に対応したモデルが必要です。",
    )?);
    let form = Form::new(ui)?;
    let host = tracked_text(ui, dispatch, views, TextField::LlmHost, DEFAULT_LLM_HOST)?;
    form.add(ui, "ホスト", &host)?;
    let port = tracked_text(ui, dispatch, views, TextField::LlmPort, DEFAULT_LLM_PORT)?;
    form.add(ui, "ポート", &port)?;
    form.note(ui, "http://<ホスト>:<ポート>/v1 に接続します。")?;
    let model = tracked_text(ui, dispatch, views, TextField::LlmModel, DEFAULT_LLM_MODEL)?;
    form.add(ui, "モデル名", &model)?;
    let api_key = tracked_secret(ui, dispatch, views, ApiKey::Llm, "未入力時は OPENAI_API_KEY")?;
    form.add(ui, "API キー（任意）", &api_key.root)?;
    page.append(form.widget());
    tracked_check(
        ui,
        dispatch,
        views,
        &page,
        "サーバーのモデル一覧に、指定したモデルがあるか確認します。",
        CheckTarget::Llm,
    )?;
    Ok(page)
}

fn jev(ui: &Ui, dispatch: &Dispatch<Message>, views: &mut SettingsViews) -> Result<Stack> {
    let page = page(ui)?;
    page.append(&title(ui, SpeakerBackend::Jev.page_title())?);
    page.append(&note(
        ui,
        "「読み上げ」で話者推測に Jev を選んだときに使う、TypeSafe System One API 互換のモデルです。\
         登場人物の抽出には LLM を使います。",
    )?);
    let form = Form::new(ui)?;
    let model =
        tracked_text(ui, dispatch, views, TextField::JevModel, "未入力時は TYPESAFE_DEFAULT_MODEL または jev-latest")?;
    form.add(ui, "モデル名", &model)?;
    let base_url = tracked_text(
        ui,
        dispatch,
        views,
        TextField::JevBaseUrl,
        "未入力時は TYPESAFE_BASE_URL または https://api.typesafe.ai",
    )?;
    form.add(ui, "URL", &base_url)?;
    let api_key = tracked_secret(ui, dispatch, views, ApiKey::Jev, "未入力時は TYPESAFE_API_KEY")?;
    form.add(ui, "API キー（任意）", &api_key.root)?;
    page.append(form.widget());
    tracked_check(
        ui,
        dispatch,
        views,
        &page,
        "サーバーのモデル一覧に、指定したモデルがあるか確認します。",
        CheckTarget::Jev,
    )?;
    Ok(page)
}

pub(super) type VoiceList = std::result::Result<Vec<String>, String>;

pub(super) fn fetch_wyoming_voices(settings: &AppSettings) -> VoiceList {
    let options = settings.wyoming_tts.options(TtsBackendSetting::Wyoming.backend());
    let client = WyomingClient::new(options.url(), options.timeout, Some(&options.language))
        .map_err(|error| format!("Wyomingサーバー({})から音声一覧を取得できませんでした: {error}", options.url()))?;
    Ok(client.list_voices())
}

fn wyoming_voice_list(ui: &Ui, dispatch: &Dispatch<Message>, views: &mut SettingsViews, page: &Stack) -> Result<()> {
    page.append(&note(ui, "サーバーから、言語に一致する音声の一覧を取得できます。")?);
    let buttons = ui.stack(naui::Orientation::Horizontal)?;
    buttons.set_spacing(8.0);
    let fetch = ui.button("音声一覧を取得")?;
    let apply = ui.button("使用する音声に設定")?;
    apply.set_enabled(false);
    fetch.on_click(dispatch.to(|| Message::FetchVoices));
    apply.on_click(dispatch.to(|| Message::ApplyVoices));
    buttons.append(&fetch);
    buttons.append(&apply);
    page.append(&buttons);
    let result = ui.label("")?;
    result.set_wrap(true);
    result.set_selectable(true);
    result.set_sizing(naui::Sizing::fill_width());
    page.append(&result);
    views.voice_list = Some(VoiceListView { fetch, apply, result });
    Ok(())
}

fn tts(ui: &Ui, dispatch: &Dispatch<Message>, views: &mut SettingsViews, backend: TtsBackendSetting) -> Result<Stack> {
    let page = page(ui)?;
    page.append(&title(ui, backend.page_title())?);
    page.append(&note(
        ui,
        match backend {
            TtsBackendSetting::Openai => {
                "「読み上げ」で TTS に OpenAI 互換 API を選んだときに使う、/v1/audio/speech に対応した TTS サーバーです。"
            }
            TtsBackendSetting::Wyoming => {
                "「読み上げ」で TTS に Wyoming を選んだときに使う、Wyoming プロトコルに対応した TTS サーバーです。"
            }
        },
    )?);

    page.append(&heading(ui, "接続")?);
    let connection = Form::new(ui)?;
    let placeholder = format!("未入力時は {}", backend.backend().default_url());
    let url = tracked_text(ui, dispatch, views, TextField::TtsUrl(backend), &placeholder)?;
    connection.add(ui, "URL", &url)?;
    match backend {
        TtsBackendSetting::Openai => {
            let model = tracked_text(ui, dispatch, views, TextField::TtsModel, setup::DEFAULT_OPENAI_TTS_MODEL)?;
            connection.add(ui, "モデル名", &model)?;
            let api_key = tracked_secret(ui, dispatch, views, ApiKey::Tts, "未入力時は OPENAI_API_KEY")?;
            connection.add(ui, "API キー（任意）", &api_key.root)?;
        }
        TtsBackendSetting::Wyoming => {
            let language = tracked_text(ui, dispatch, views, TextField::TtsLanguage, "空にすると絞り込みません")?;
            connection.add(ui, "言語", &language)?;
            connection.note(
                ui,
                "サーバーの音声一覧から使う音声を、この言語で絞り込みます(ja と ja-JP は一致とみなします)。",
            )?;
        }
    }
    let timeout_field = NumberField::Timeout(backend);
    let timeout = number_field(ui, timeout_field.value(&AppSettings::default()), (Some(1.0), None), 1.0, 0)?;
    timeout.on_change(dispatch.map(move |value| Message::NumberChanged(timeout_field, value)));
    views.numbers.push((timeout_field, timeout.clone()));
    connection.add(ui, "タイムアウト(秒)", &timeout)?;
    page.append(connection.widget());

    page.append(&heading(ui, "声の割り当て")?);
    page.append(&note(
        ui,
        "ナレーターと登場人物に使う音声です。登場人物には、ナレーター以外で使用数が最も少ない音声を割り当て、\
         後の章でも同じ人物には同じ音声を使います。複数の音声はカンマで区切ります。",
    )?);
    let assignment = Form::new(ui)?;
    let voices = tracked_text(
        ui,
        dispatch,
        views,
        TextField::TtsVoices(backend),
        match backend {
            TtsBackendSetting::Openai => "必須",
            TtsBackendSetting::Wyoming => "空にするとサーバーの一覧(言語に一致する音声)",
        },
    )?;
    assignment.add(ui, "使用する音声", &voices)?;
    let narrator =
        tracked_text(ui, dispatch, views, TextField::TtsNarratorVoice(backend), "未入力時は使用する音声の先頭")?;
    assignment.add(ui, "ナレーター", &narrator)?;
    let male = tracked_text(ui, dispatch, views, TextField::TtsMaleVoices(backend), "")?;
    assignment.add(ui, "男性に優先する音声", &male)?;
    let female = tracked_text(ui, dispatch, views, TextField::TtsFemaleVoices(backend), "")?;
    assignment.add(ui, "女性に優先する音声", &female)?;
    assignment.note(ui, "性別が分かっている登場人物には、この中から優先して割り当てます。")?;
    page.append(assignment.widget());
    if backend == TtsBackendSetting::Wyoming {
        wyoming_voice_list(ui, dispatch, views, &page)?;
    }

    page.append(&heading(ui, "合成")?);
    let synthesis = Form::new(ui)?;
    let max_chars_field = NumberField::MaxChars(backend);
    let max_chars = number_field(ui, max_chars_field.value(&AppSettings::default()), (Some(0.0), None), 10.0, 0)?;
    max_chars.on_change(dispatch.map(move |value| Message::NumberChanged(max_chars_field, value)));
    views.numbers.push((max_chars_field, max_chars.clone()));
    synthesis.add(ui, "1回の最大文字数", &max_chars)?;
    synthesis.note(ui, "超える行は文末・読点・文字数の順で分割して合成します。0 で分割しません。")?;
    let pause_field = NumberField::Pause(backend);
    let pause = number_field(ui, pause_field.value(&AppSettings::default()), (Some(0.0), Some(MAX_TTS_PAUSE)), 0.1, 1)?;
    pause.on_change(dispatch.map(move |value| Message::NumberChanged(pause_field, value)));
    views.numbers.push((pause_field, pause.clone()));
    synthesis.add(ui, "行間の無音(秒)", &pause)?;
    page.append(synthesis.widget());

    let description = match backend {
        TtsBackendSetting::Openai => {
            "使用する音声のそれぞれで短い文を合成し、URL・モデル名・API キー・音声名が正しいかを確かめます。"
        }
        TtsBackendSetting::Wyoming => {
            "サーバーの音声一覧と照らして音声名を確かめ、ナレーターの音声で短い文を合成してみます。"
        }
    };
    tracked_check(ui, dispatch, views, &page, description, CheckTarget::Tts(backend))?;
    Ok(page)
}
