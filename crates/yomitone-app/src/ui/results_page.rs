//! 「結果」画面のウィジェットと描画。

use std::cell::RefCell;

use naui::{Length, Result, Sizing, Stack, Table, TableColumn, TableRow, Ui};
use yomitone::domain::character::Character;

use super::parts::{heading, note, page, title};

pub(crate) struct ResultsView {
    characters: Table,
    voices: Table,
    shown_characters: RefCell<Vec<Character>>,
    shown_voices: RefCell<Vec<(String, String)>>,
}

impl ResultsView {
    pub(crate) fn render(&self, characters: &[Character], voices: &[(String, String)]) {
        if *self.shown_characters.borrow() != characters {
            let rows: Vec<TableRow> = characters
                .iter()
                .map(|character| {
                    let gender = character.gender.map(|gender| gender.label()).unwrap_or("不明");
                    TableRow::new([
                        character.name.clone(),
                        gender.to_string(),
                        character.aliases.join("、"),
                        character.portrait.clone(),
                    ])
                })
                .collect();
            self.characters.set_rows(&rows);
            *self.shown_characters.borrow_mut() = characters.to_vec();
        }
        if *self.shown_voices.borrow() != voices {
            let rows: Vec<TableRow> =
                voices.iter().map(|(name, voice)| TableRow::new([name.as_str(), voice.as_str()])).collect();
            self.voices.set_rows(&rows);
            *self.shown_voices.borrow_mut() = voices.to_vec();
        }
    }
}

pub(crate) fn build(ui: &Ui) -> Result<(Stack, ResultsView)> {
    let page = page(ui)?;
    page.append(&title(ui, "結果")?);
    page.append(&note(ui, "最後に読み上げた内容です。登場人物の一覧は章ごとに更新し、後の章に引き継ぎます。")?);

    page.append(&heading(ui, "登場人物")?);
    let characters = ui.table()?;
    characters.set_columns(&[
        TableColumn::new("名前").width(160.0),
        TableColumn::new("性別").width(70.0),
        TableColumn::new("別名").width(180.0),
        TableColumn::new("特徴"),
    ]);
    characters.set_sizing(Sizing::new().width(Length::Fill).height(Length::Fixed(280.0)));
    page.append(&characters);

    page.append(&heading(ui, "音声の割り当て")?);
    page.append(&note(ui, "音声を書き出したときだけ表示します。")?);
    let voices = ui.table()?;
    voices.set_columns(&[TableColumn::new("名前").width(200.0), TableColumn::new("音声")]);
    voices.set_sizing(Sizing::new().width(Length::Fill).height(Length::Fixed(220.0)));
    page.append(&voices);

    Ok((
        page,
        ResultsView {
            characters,
            voices,
            shown_characters: RefCell::new(Vec::new()),
            shown_voices: RefCell::new(Vec::new()),
        },
    ))
}
