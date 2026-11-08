//! 画面の組み方をそろえるための部品。

use std::cell::Cell;

use naui::{
    Align, FilePickerMode, Grid, GridCell, Label, Length, Orientation, Padding, PasswordInput, Result, Scroll,
    ScrollPolicy, Sizing, Stack, TextColor, TextInput, TextStyle, Toast, Track, Ui, Widget,
};

use super::model::{AppModel, CheckTarget, Message, TextField};
use super::mvu::Dispatch;
use crate::credentials::{self, ApiKey};

/// 左端をそろえて上から積む画面の土台。
pub(crate) fn page(ui: &Ui) -> Result<Stack> {
    let page = ui.stack(Orientation::Vertical)?;
    page.set_spacing(12.0);
    page.set_padding(Padding::all(20.0));
    page.set_align(Align::Start);
    page.set_sizing(Sizing::fill_width());
    Ok(page)
}

/// 横幅はウィンドウに合わせ、縦だけをスクロールする。
pub(crate) fn scrollable(ui: &Ui, page: &dyn Widget) -> Result<Scroll> {
    let scroll = ui.scroll()?;
    scroll.set_policy(ScrollPolicy::Never, ScrollPolicy::Auto);
    scroll.set_child(page);
    scroll.set_sizing(Sizing::fill());
    Ok(scroll)
}

pub(crate) fn title(ui: &Ui, text: &str) -> Result<Label> {
    let label = ui.label(text)?;
    label.set_style(TextStyle::Title);
    Ok(label)
}

pub(crate) fn heading(ui: &Ui, text: &str) -> Result<Label> {
    let label = ui.label(text)?;
    label.set_style(TextStyle::Heading);
    Ok(label)
}

/// 説明文。小さく淡い文字にして、窓を狭めても折り返す。
pub(crate) fn note(ui: &Ui, text: &str) -> Result<Label> {
    let label = ui.label(text)?;
    label.set_style(TextStyle::Caption);
    label.set_color(TextColor::Secondary);
    label.set_wrap(true);
    label.set_sizing(Sizing::fill_width());
    Ok(label)
}

pub(crate) fn feedback(toast: &Toast, message: &str) {
    toast.set_message(message);
    toast.set_timeout(6.0);
    toast.show();
}

/// 左に項目名、右に入力欄を並べる表。
pub(crate) struct Form {
    grid: Grid,
    rows: Cell<usize>,
}

impl Form {
    pub(crate) fn new(ui: &Ui) -> Result<Self> {
        let grid = ui.grid()?;
        grid.set_spacing(12.0, 8.0);
        grid.set_column_track(0, Track::Fixed(170.0));
        grid.set_column_track(1, Track::FILL);
        grid.set_sizing(Sizing::fill_width());
        Ok(Self { grid, rows: Cell::new(0) })
    }

    pub(crate) fn widget(&self) -> &Grid {
        &self.grid
    }

    fn next_row(&self) -> usize {
        let row = self.rows.get();
        self.rows.set(row + 1);
        row
    }

    pub(crate) fn add(&self, ui: &Ui, label: &str, field: &dyn Widget) -> Result<()> {
        let row = self.next_row();
        self.grid.attach(&ui.label(label)?, GridCell::new(0, row));
        self.grid.attach(field, GridCell::new(1, row));
        Ok(())
    }

    pub(crate) fn note(&self, ui: &Ui, text: &str) -> Result<()> {
        let row = self.next_row();
        self.grid.attach(&note(ui, text)?, GridCell::new(1, row));
        Ok(())
    }
}

pub(crate) fn text_field(
    ui: &Ui,
    dispatch: &Dispatch<Message>,
    field: TextField,
    placeholder: &str,
) -> Result<TextInput> {
    let input = ui.text_input("")?;
    input.set_placeholder(placeholder);
    input.set_sizing(Sizing::fill_width());
    input.on_change(dispatch.with(move |text: &str| Message::TextChanged(field, text.to_owned())));
    Ok(input)
}

#[derive(Clone)]
pub(crate) struct SecretField {
    pub(crate) root: Stack,
    pub(crate) input: PasswordInput,
    pub(crate) load: naui::Button,
    pub(crate) save: naui::Button,
    pub(crate) delete: naui::Button,
    key: ApiKey,
}

impl SecretField {
    pub(crate) fn render(&self, model: &AppModel) {
        self.input.set_text(self.key.value(&model.settings));
        self.load.set_visible(model.credential_errors[self.key.index()].is_some());
        self.save.set_enabled(credentials::SUPPORTED);
        self.delete.set_enabled(credentials::SUPPORTED);
    }
}

/// API キー欄と、OS の認証ストアへの読み書きボタン。
pub(crate) fn secret_field(
    ui: &Ui,
    dispatch: &Dispatch<Message>,
    key: ApiKey,
    placeholder: &str,
) -> Result<SecretField> {
    let row = ui.stack(Orientation::Horizontal)?;
    row.set_spacing(8.0);
    row.set_sizing(Sizing::fill_width());
    let input = ui.password_input()?;
    input.set_placeholder(placeholder);
    input.set_sizing(Sizing::fill_width());
    input.on_change(dispatch.with(move |text: &str| Message::ApiKeyChanged(key, text.to_owned())));

    let load = ui.button("読み込む")?;
    load.set_visible(false);
    load.on_click(dispatch.to(move || Message::LoadApiKey(key)));
    let save = ui.button("保存")?;
    save.set_enabled(credentials::SUPPORTED);
    save.on_click(dispatch.to(move || Message::SaveApiKey(key)));
    let delete = ui.button("削除")?;
    delete.set_enabled(credentials::SUPPORTED);
    delete.on_click(dispatch.to(move || Message::DeleteApiKey(key)));
    row.append(&input);
    row.append(&load);
    row.append(&save);
    row.append(&delete);
    Ok(SecretField { root: row, input, load, save, delete, key })
}

/// フォルダのパス入力と選択ダイアログ。
pub(crate) fn folder_field(
    ui: &Ui,
    dispatch: &Dispatch<Message>,
    field: TextField,
) -> Result<(Stack, TextInput, naui::FilePicker)> {
    let row = ui.stack(Orientation::Horizontal)?;
    row.set_spacing(8.0);
    row.set_sizing(Sizing::fill_width());
    let input = text_field(ui, dispatch, field, "フォルダのパス")?;
    let picker = ui.file_picker("選択…")?;
    picker.set_mode(FilePickerMode::Folder);
    let dispatch = dispatch.clone();
    picker.on_select(move |entries| {
        if let Some(path) = entries.first().and_then(|entry| entry.path()) {
            dispatch.send(Message::TextChanged(field, path.display().to_string()));
        }
    });
    row.append(&input);
    row.append(&picker);
    Ok((row, input, picker))
}

pub(crate) fn number_field(
    ui: &Ui,
    value: f64,
    range: (Option<f64>, Option<f64>),
    step: f64,
    decimals: u32,
) -> Result<naui::NumberInput> {
    let input = ui.number_input(0.0)?;
    input.set_decimals(decimals);
    input.set_step(step);
    input.set_range(range.0, range.1);
    input.set_value(value);
    input.set_sizing(Sizing::new().width(Length::Fixed(140.0)));
    Ok(input)
}

pub(crate) fn check_section(
    ui: &Ui,
    dispatch: &Dispatch<Message>,
    page: &Stack,
    description: &str,
    target: CheckTarget,
) -> Result<naui::Button> {
    page.append(&heading(ui, "接続の確認")?);
    page.append(&note(ui, description)?);
    let button = ui.button("接続を確認")?;
    button.on_click(dispatch.to(move || Message::CheckConnection(target)));
    page.append(&button);
    Ok(button)
}
