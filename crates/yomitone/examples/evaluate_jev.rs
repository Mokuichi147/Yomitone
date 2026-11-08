//! 実サーバーで話者推測の精度・速度・失敗を測る。実行方法は evaluations/jev/README.md を参照。
use std::cell::RefCell;
use std::error::Error;
use std::fs;
use std::rc::Rc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{json, Value};
use yomitone::adapters::jev::JevSpeakerEstimator;
use yomitone::adapters::llm::OpenAiChatModel;
use yomitone::application::ports::{
    CharacterExtractor, EstimationError, ExtractionError, Presenter, SpeakerEstimator, SpeakerGuess,
};
use yomitone::application::speaker_estimation::SpeakerEstimation;
use yomitone::domain::character::{Character, Gender};
use yomitone::domain::novel::{Novel, Sentence};

#[derive(Deserialize)]
struct Person {
    name: String,
    portrait: String,
    #[serde(default)]
    aliases: Vec<String>,
    gender: Option<String>,
}

#[derive(Deserialize)]
struct Line {
    text: String,
    expected: Option<String>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    category: String,
    characters: Vec<Person>,
    #[serde(default)]
    lines: Vec<Line>,
    /// 本文は再配布せず、利用者の手元のファイルと行番号だけを参照する。
    source: Option<String>,
    #[serde(default)]
    labels: std::collections::BTreeMap<usize, String>,
    #[serde(default)]
    prefix_repeat: usize,
}

struct TimedEstimator {
    inner: Box<dyn SpeakerEstimator>,
    pending: Rc<RefCell<Vec<Value>>>,
}

impl SpeakerEstimator for TimedEstimator {
    fn estimate_speaker(
        &mut self,
        candidates: &[Character],
        previous: &[Sentence],
        target: &Sentence,
        following: &[Sentence],
    ) -> Result<Option<SpeakerGuess>, EstimationError> {
        let start = Instant::now();
        let result = self.inner.estimate_speaker(candidates, previous, target, following);
        self.pending.borrow_mut().push(json!({
            "seconds": start.elapsed().as_secs_f64(),
            "previous_lines": previous.len(),
            "previous_chars": previous.iter().map(|s| s.text.chars().count()).sum::<usize>(),
            "following_lines": following.len(),
            "error": result.as_ref().err().map(ToString::to_string),
        }));
        result
    }
}

#[derive(Default)]
struct Recorder {
    pending: Rc<RefCell<Vec<Value>>>,
    events: RefCell<Vec<Value>>,
}

impl Recorder {
    fn record(&self, sentence: &Sentence, confidence: Option<f64>, fallback: bool, error: Option<&str>) {
        let calls = std::mem::take(&mut *self.pending.borrow_mut());
        let seconds: f64 = calls.iter().filter_map(|c| c["seconds"].as_f64()).sum();
        self.events.borrow_mut().push(json!({
            "actual": sentence.speaker.as_ref().map(|c| &c.name),
            "confidence": confidence,
            "fallback": fallback,
            "error": error,
            "seconds": seconds,
            "calls": calls,
        }));
    }
}

impl Presenter for Recorder {
    fn speaker_estimated(&self, sentence: &Sentence, confidence: Option<f64>) {
        self.record(sentence, confidence, false, None);
    }

    fn speaker_estimation_failed(&self, sentence: &Sentence, error: Option<&str>) {
        self.record(sentence, None, true, error);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    if !(9..=10).contains(&args.len()) {
        return Err(
            "引数: <ケースJSON> <結果JSON> <URL> <モデル> <jev|llm> <後続行数> <反復数> <fixed|extract> [履歴行数]"
                .into(),
        );
    }
    let cases: Vec<Case> = serde_json::from_str(&fs::read_to_string(&args[1])?)?;
    let base_url = &args[3];
    let model = &args[4];
    let backend = &args[5];
    let following: usize = args[6].parse()?;
    let rounds: usize = args[7].parse()?;
    let previous: usize = args.get(9).map(|s| s.parse()).transpose()?.unwrap_or(100);
    if !matches!(backend.as_str(), "jev" | "llm") || !matches!(args[8].as_str(), "fixed" | "extract") || rounds == 0 {
        return Err("バックエンド・抽出方法・反復数が不正です".into());
    }
    // 比較用の既存 LLM アダプターはホストとポートを別々に受け取る。
    let address = base_url.strip_prefix("http://").ok_or("検証用URLには http:// を指定してください")?;
    let (host, port) = address.trim_end_matches('/').rsplit_once(':').ok_or("ポートが必要です")?;
    // 認証を要求しないローカルサーバー向けのダミー。実際のキーがあれば環境変数から使う。
    let key = std::env::var("TYPESAFE_API_KEY").unwrap_or_else(|_| "local-evaluation".into());
    let create_jev = || JevSpeakerEstimator::new(Some(key.clone()), Some(model.clone()), Some(base_url.clone()));
    let diagnostic = create_jev()?.list_models();
    let mut results = json!({
        "started_unix_seconds": SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        "base_url": base_url, "model": model, "backend": backend,
        "previous_count": previous, "following_count": following,
        "rounds": rounds, "characters_mode": args[8],
        "fixture": args[1],
        "model_list": diagnostic.as_ref().ok(),
        "model_list_error": diagnostic.as_ref().err(),
        "cases": [],
    });
    let mut total = 0;
    let mut correct = 0;
    let start = Instant::now();
    for round in 1..=rounds {
        for case in &cases {
            let mut lines: Vec<Line> = (0..case.prefix_repeat).map(|i| Line {
                text: format!("記録{i}。駅前の時計台には午後の日が当たり、広場には買い物帰りの人々が行き交っていた。遠くでは路面電車がゆっくりと通り過ぎ、街路樹の影が歩道に伸びていた。"),
                expected: None,
            }).collect();
            if let Some(path) = &case.source {
                lines.extend(
                    fs::read_to_string(path)?
                        .lines()
                        .enumerate()
                        .map(|(i, text)| Line { text: text.to_string(), expected: case.labels.get(&(i + 1)).cloned() }),
                );
            }
            lines.extend(case.lines.iter().map(|l| Line { text: l.text.clone(), expected: l.expected.clone() }));
            let mut novel = Novel::from_lines(lines.iter().map(|l| &l.text));
            let mut characters: Vec<Character> = case
                .characters
                .iter()
                .map(|c| {
                    Character::new(&c.name, &c.portrait)
                        .with_aliases(c.aliases.clone())
                        .with_gender(c.gender.as_deref().and_then(Gender::from_label))
                })
                .collect();
            let mut extraction = Value::Null;
            let mut extraction_fatal = None;
            if args[8] == "extract" {
                let start = Instant::now();
                let result = OpenAiChatModel::new(host, port, model).extract_characters(&novel, &[]);
                extraction = json!({"seconds": start.elapsed().as_secs_f64(), "error": result.as_ref().err().map(ToString::to_string)});
                if let Err(ExtractionError::Fatal(error)) = &result {
                    extraction_fatal = Some(error.clone());
                }
                characters = result.unwrap_or_default();
            }
            let character_info: Vec<Value> = characters
                .iter()
                .map(|c| {
                    json!({
                        "name": c.name, "aliases": c.aliases, "portrait": c.portrait,
                    })
                })
                .collect();
            novel.characters = characters.clone();
            let recorder = Rc::new(Recorder::default());
            let inner: Box<dyn SpeakerEstimator> = match backend.as_str() {
                "jev" => Box::new(create_jev()?),
                _ => Box::new(OpenAiChatModel::new(host, port, model)),
            };
            let estimator = TimedEstimator { inner, pending: recorder.pending.clone() };
            let mut estimation = SpeakerEstimation::new(Box::new(estimator), recorder.clone());
            estimation.previous_count = previous;
            estimation.following_count = following;
            let start_case = Instant::now();
            let error = match extraction_fatal {
                Some(error) => Some(error),
                None => estimation.run(&mut novel).err().map(|e| e.to_string()),
            };
            let mut events = recorder.events.borrow_mut();
            let mut event_index = 0;
            for (i, sentence) in novel.sentences.iter().enumerate() {
                if !sentence.is_dialogue() {
                    continue;
                }
                // 致命的エラーで途中終了した文も不正解に数え、分母が縮まらないようにする。
                if event_index >= events.len() {
                    events.push(json!({"actual": null, "confidence": null, "fallback": true,
                        "error": error, "seconds": null, "calls": []}));
                }
                let event = &mut events[event_index];
                let expected = lines[i].expected.as_deref();
                let actual = event["actual"].as_str().unwrap_or_default();
                let canonical =
                    characters.iter().find(|c| c.name == actual).map(|c| c.aliases.as_slice()).unwrap_or(&[]);
                let matched = expected.map(|e| {
                    actual == e
                        || canonical.iter().any(|a| a == e)
                        || case.characters.iter().any(|c| c.name == e && c.aliases.iter().any(|a| a == actual))
                });
                event["line"] = json!(i + 1);
                event["expected"] = json!(expected);
                event["correct"] = json!(matched);
                if let Some(matched) = matched {
                    total += 1;
                    correct += usize::from(matched);
                }
                event_index += 1;
            }
            let record = json!({
                "id": case.id, "category": case.category, "round": round,
                "source": case.source, "characters": character_info, "extraction": extraction,
                "seconds": start_case.elapsed().as_secs_f64(), "fatal_error": error,
                "events": *events,
            });
            println!(
                "反復{round} {}: {}セリフ、累計 {correct}/{total} 正解、{:.2}秒",
                case.id,
                events.len(),
                record["seconds"].as_f64().unwrap()
            );
            results["cases"].as_array_mut().unwrap().push(record);
            results["elapsed_seconds"] = json!(start.elapsed().as_secs_f64());
            results["scored"] = json!(total);
            results["correct"] = json!(correct);
            fs::write(&args[2], serde_json::to_string_pretty(&results)?)?;
        }
    }
    Ok(())
}
