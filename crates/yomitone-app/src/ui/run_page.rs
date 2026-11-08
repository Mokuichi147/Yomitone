//! 「読み上げ」画面のウィジェットと描画。

use std::cell::Cell;

use naui::{
    Button, ComboBox, Grid, GridCell, Label, Orientation, Padding, ProgressBar, Result, Scroll, ScrollPolicy, Sizing,
    TextColor, TextInput, Toggle, Track, Ui,
};

use super::model::{AppModel, Message, TextField};
use super::mvu::Dispatch;
use super::parts::{folder_field, heading, note, title, Form};
use crate::settings::{SpeakerBackend, TtsBackendSetting};

pub(crate) struct RunPageView {
    input_folder: TextInput,
    speaker: ComboBox,
    export: Toggle,
    tts: ComboBox,
    output_folder: TextInput,
    output_picker: naui::FilePicker,
    start: Button,
    stop: Button,
    bar: ProgressBar,
    status: Label,
    log: LogView,
}

struct LogView {
    label: Label,
    scroll: Scroll,
    shown_revision: Cell<u64>,
}

impl RunPageView {
    pub(crate) fn render(&self, model: &AppModel) {
        let settings = &model.settings;
        let run = &model.run;
        self.input_folder.set_text(&settings.input_folder);
        self.speaker.set_selected(match settings.speaker_backend {
            SpeakerBackend::Llm => 0,
            SpeakerBackend::Jev => 1,
        });
        self.export.set_on(settings.export_audio);
        self.tts.set_selected(match settings.tts_backend {
            TtsBackendSetting::Openai => 0,
            TtsBackendSetting::Wyoming => 1,
        });
        self.output_folder.set_text(&settings.output_folder);
        self.tts.set_enabled(settings.export_audio);
        self.output_folder.set_enabled(settings.export_audio);
        self.output_picker.set_enabled(settings.export_audio);
        let running = run.cancel.is_some();
        self.start.set_enabled(!running && !model.close_requested);
        self.stop.set_enabled(running && !run.stopping);
        self.bar.set_value(if run.complete { 1.0 } else { run.progress.fraction() });
        if running {
            if run.stopping {
                self.status.set_text("中止しています(今の文を終えたところで止まります)");
                self.status.set_color(TextColor::Secondary);
            } else {
                self.status.set_text(&run.progress.status());
                self.status.set_color(TextColor::Default);
            }
        } else {
            self.status.set_text("");
            self.status.set_color(TextColor::Default);
        }
        if self.log.shown_revision.get() != run.log_revision {
            self.log.label.set_text(&run.log_text);
            self.log.scroll.scroll_to_end();
            self.log.shown_revision.set(run.log_revision);
        }
    }
}

pub(crate) fn build(ui: &Ui, dispatch: &Dispatch<Message>) -> Result<(Grid, RunPageView)> {
    let page = ui.grid()?;
    page.set_spacing(0.0, 12.0);
    page.set_padding(Padding::all(20.0));
    page.set_column_track(0, Track::FILL);
    page.set_sizing(Sizing::fill());
    let rows = Cell::new(0);
    let append = |widget: &dyn naui::Widget| {
        let row = rows.get();
        page.attach(widget, GridCell::new(0, row));
        rows.set(row + 1);
        row
    };

    append(&title(ui, "読み上げ")?);
    append(&note(
        ui,
        "章ごとに分けた UTF-8 の .txt ファイルを置いたフォルダを選び、開始を押します。\
         ファイル名の順に処理し、行全体が「」で囲まれた行を誰のセリフか推測します。\
         接続先などは左の「設定」で変えられます。",
    )?);

    let form = Form::new(ui)?;
    let (input_row, input_folder, _) = folder_field(ui, dispatch, TextField::InputFolder)?;
    form.add(ui, "入力フォルダ", &input_row)?;

    let speakers = [SpeakerBackend::Llm, SpeakerBackend::Jev];
    let speaker = ui.combo_box()?;
    speaker.set_items(&speakers.map(SpeakerBackend::page_title));
    speaker.on_select(
        dispatch.map(move |index: usize| Message::SpeakerBackendChanged(speakers[index.min(speakers.len() - 1)])),
    );
    form.add(ui, "話者推測", &speaker)?;
    form.note(ui, "セリフの話者を推測するのに使います。登場人物の抽出には常に LLM を使います。")?;

    let export = ui.toggle("音声合成して WAV を書き出す")?;
    export.on_toggle(dispatch.map(Message::ExportAudioChanged));
    form.add(ui, "音声", &export)?;
    let backends = [TtsBackendSetting::Openai, TtsBackendSetting::Wyoming];
    let tts = ui.combo_box()?;
    tts.set_items(&backends.map(TtsBackendSetting::page_title));
    tts.on_select(
        dispatch.map(move |index: usize| Message::TtsBackendChanged(backends[index.min(backends.len() - 1)])),
    );
    form.add(ui, "TTS", &tts)?;
    let (output_row, output_folder, output_picker) = folder_field(ui, dispatch, TextField::OutputFolder)?;
    form.add(ui, "出力フォルダ", &output_row)?;
    form.note(ui, "章ごとに <入力ファイル名>.wav を書き出します。")?;
    append(form.widget());

    let buttons = ui.stack(Orientation::Horizontal)?;
    buttons.set_spacing(8.0);
    let start = ui.button("開始")?;
    start.on_click(dispatch.to(|| Message::Start));
    let stop = ui.button("中止")?;
    stop.on_click(dispatch.to(|| Message::Stop));
    buttons.append(&start);
    buttons.append(&stop);
    append(&buttons);

    let bar = ui.progress_bar()?;
    bar.set_sizing(Sizing::fill_width());
    append(&bar);
    let status = ui.label("")?;
    status.set_wrap(true);
    status.set_sizing(Sizing::fill_width());
    append(&status);

    append(&heading(ui, "経過")?);
    let log_label = ui.label("")?;
    log_label.set_wrap(true);
    log_label.set_selectable(true);
    log_label.set_sizing(Sizing::fill_width());
    let log_scroll = ui.scroll()?;
    log_scroll.set_policy(ScrollPolicy::Never, ScrollPolicy::Auto);
    log_scroll.set_child(&log_label);
    log_scroll.set_sizing(Sizing::fill().min_height(160.0));
    let log_row = append(&log_scroll);
    page.set_row_track(log_row, Track::FILL);

    let view = RunPageView {
        input_folder,
        speaker,
        export,
        tts,
        output_folder,
        output_picker,
        start,
        stop,
        bar,
        status,
        log: LogView { label: log_label, scroll: log_scroll, shown_revision: Cell::new(u64::MAX) },
    };
    Ok((page, view))
}
