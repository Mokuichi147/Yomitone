//! API キーは設定ファイルから分け、OS の認証ストアだけに保存する。

use keyring::{Entry, Error};

use crate::settings::AppSettings;

#[cfg(target_os = "macos")]
const SERVICE: &str = "io.github.mokuichi147.yomitone.credentials.v2";
#[cfg(not(target_os = "macos"))]
const SERVICE: &str = "io.github.mokuichi147.yomitone";
const LEGACY_SERVICE: &str = "io.github.mokuichi147.yomitone";
pub(crate) const SUPPORTED: bool = cfg!(any(target_os = "macos", target_os = "windows"));

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApiKey {
    Llm,
    Jev,
    Tts,
}

impl ApiKey {
    const ALL: [Self; 3] = [Self::Llm, Self::Jev, Self::Tts];

    pub(crate) const fn index(self) -> usize {
        self as usize
    }

    fn account(self) -> &'static str {
        match self {
            Self::Llm => "llm-api-key",
            Self::Jev => "jev-api-key",
            Self::Tts => "tts-api-key",
        }
    }

    pub(crate) fn value(self, settings: &AppSettings) -> &str {
        match self {
            Self::Llm => &settings.llm_api_key,
            Self::Jev => &settings.jev_api_key,
            Self::Tts => &settings.openai_tts.api_key,
        }
    }

    pub(crate) fn set(self, settings: &mut AppSettings, value: String) {
        match self {
            Self::Llm => settings.llm_api_key = value,
            Self::Jev => settings.jev_api_key = value,
            Self::Tts => settings.openai_tts.api_key = value,
        }
    }
}

/// エントリーを保持し、テストでは OS に触れずにモックへ置き換えられるようにする。
pub(crate) struct Credentials {
    entries: [keyring::Result<Entry>; 3],
    legacy_entries: Option<[keyring::Result<Entry>; 3]>,
}

impl Credentials {
    pub(crate) fn new() -> Self {
        Self {
            entries: ApiKey::ALL.map(|key| Entry::new(SERVICE, key.account())),
            legacy_entries: cfg!(target_os = "macos")
                .then(|| ApiKey::ALL.map(|key| Entry::new(LEGACY_SERVICE, key.account()))),
        }
    }

    fn entry(&self, key: ApiKey) -> Result<&Entry, String> {
        if !SUPPORTED && !cfg!(test) {
            return Err("この OS では API キーの保存に対応していません。".into());
        }
        self.entries[key.index()].as_ref().map_err(error_message)
    }

    pub(crate) fn read(&self, key: ApiKey) -> Result<Option<String>, String> {
        keychain_access(true, || self.read_inner(key))?
    }

    fn read_inner(&self, key: ApiKey) -> Result<Option<String>, String> {
        let entry = self.entry(key)?;
        if let Some(value) = read_entry(entry)? {
            return Ok(Some(value));
        }
        let Some(legacy) = &self.legacy_entries else {
            return Ok(None);
        };
        let value = read_entry(legacy[key.index()].as_ref().map_err(error_message)?)?;
        if let Some(value) = &value {
            // 旧項目のアクセス権は旧ビルドの署名に結び付いている。
            // 読み取りに成功したら、現在の署名で作った項目へコピーする。
            // 移行が失敗しても元の項目は残す。
            entry
                .set_password(value)
                .map_err(|error| format!("API キーを移行できませんでした。{}", error_message(&error)))?;
        }
        Ok(value)
    }

    pub(crate) fn load(&self, settings: &mut AppSettings) -> [Option<String>; 3] {
        if !SUPPORTED && !cfg!(test) {
            return std::array::from_fn(|_| None);
        }
        // 自動読み込みでパスワード入力やアクセス許可の画面を開かない。
        // 許可が必要なキーは、設定画面から明示的に読み込む。
        keychain_access(false, || {
            ApiKey::ALL.map(|key| match self.read_inner(key) {
                Ok(Some(value)) => {
                    key.set(settings, value);
                    None
                }
                Ok(None) => None,
                Err(error) => Some(format!("保存済みの API キーを読み込めませんでした。{error}")),
            })
        })
        .unwrap_or_else(|error| std::array::from_fn(|_| Some(error.clone())))
    }

    pub(crate) fn save(&self, key: ApiKey, value: &str) -> Result<(), String> {
        let value = value.trim();
        if value.is_empty() {
            return Err("API キーを入力してください。".into());
        }
        keychain_access(true, || {
            self.entry(key)?
                .set_password(value)
                .map_err(|error| format!("API キーを保存できませんでした。{}", error_message(&error)))
        })?
    }

    pub(crate) fn delete(&self, key: ApiKey) -> Result<(), String> {
        keychain_access(true, || {
            // 旧項目を先に削除し、次回の読み込みで復活しないようにする。
            if let Some(legacy) = &self.legacy_entries {
                delete_entry(legacy[key.index()].as_ref().map_err(error_message)?)?;
            }
            delete_entry(self.entry(key)?)
        })?
    }
}

fn read_entry(entry: &Entry) -> Result<Option<String>, String> {
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(Error::NoEntry) => Ok(None),
        Err(error) => Err(error_message(&error)),
    }
}

fn delete_entry(entry: &Entry) -> Result<(), String> {
    match entry.delete_credential() {
        Ok(()) => match read_entry(entry)? {
            // 一部のストアでは削除失敗が返らないため、消えたことも確かめる。
            None => Ok(()),
            Some(_) => Err("API キーを削除できませんでした。認証ストアにキーが残っています。".into()),
        },
        Err(Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("API キーを削除できませんでした。{}", error_message(&error))),
    }
}

#[cfg(target_os = "macos")]
fn keychain_access<T>(allow_prompt: bool, operation: impl FnOnce() -> T) -> Result<T, String> {
    use security_framework::os::macos::keychain::SecKeychain;
    use std::sync::Mutex;

    // この設定はプロセス単位なので、自動読み込みと明示的な操作が重ならないようにする。
    static INTERACTION_LOCK: Mutex<()> = Mutex::new(());
    let _lock = INTERACTION_LOCK.lock().map_err(|_| "キーチェーンを読み込めませんでした。".to_string())?;
    let allowed =
        SecKeychain::user_interaction_allowed().map_err(|_| "キーチェーンを読み込めませんでした。".to_string())?;
    let _interaction = if !allow_prompt && allowed {
        Some(SecKeychain::disable_user_interaction().map_err(|_| "キーチェーンを読み込めませんでした。".to_string())?)
    } else {
        None
    };
    Ok(operation())
}

#[cfg(not(target_os = "macos"))]
fn keychain_access<T>(_allow_prompt: bool, operation: impl FnOnce() -> T) -> Result<T, String> {
    Ok(operation())
}

fn error_message(error: &Error) -> String {
    // 認証情報が Debug 出力に含まれるエラーもあるため、エラーの内容をそのまま表示しない。
    match error {
        Error::NoStorageAccess(_) => "認証ストアへのアクセスが拒否されたか、ロックされています。",
        Error::Ambiguous(_) => "同じ名前の認証情報が複数あります。OS の認証ストアを確認してください。",
        Error::BadEncoding(_) => "保存済みのキーを文字列として読み取れません。",
        Error::TooLong(_, _) => "API キーが認証ストアの長さ制限を超えています。",
        _ => "OS の認証ストアにアクセスできませんでした。",
    }
    .into()
}

#[cfg(test)]
mod tests {
    use keyring::mock::MockCredential;

    use super::*;

    fn mock_store() -> Credentials {
        Credentials {
            entries: std::array::from_fn(|_| Ok(Entry::new_with_credential(Box::<MockCredential>::default()))),
            legacy_entries: None,
        }
    }

    fn fail_next(store: &Credentials, key: ApiKey) {
        let mock = store.entry(key).unwrap().get_credential().downcast_ref::<MockCredential>().unwrap();
        mock.set_error(Error::NoStorageAccess(Box::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied))));
    }

    #[test]
    fn saved_keys_are_restored_independently_and_never_serialized() {
        let store = mock_store();
        for (key, value) in [(ApiKey::Llm, "llm-secret"), (ApiKey::Jev, "jev-secret"), (ApiKey::Tts, "tts-secret")] {
            store.save(key, value).unwrap();
        }
        let mut settings = AppSettings::default();
        assert_eq!(store.load(&mut settings), [None, None, None]);
        assert_eq!(settings.llm_api_key, "llm-secret");
        assert_eq!(settings.jev_api_key, "jev-secret");
        assert_eq!(settings.openai_tts.api_key, "tts-secret");
        let json = serde_json::to_string(&settings).unwrap();
        for secret in ["llm-secret", "jev-secret", "tts-secret"] {
            assert!(!json.contains(secret));
        }
        store.delete(ApiKey::Jev).unwrap();
        store.delete(ApiKey::Jev).unwrap();
        let mut reloaded = AppSettings::default();
        assert_eq!(store.load(&mut reloaded), [None, None, None]);
        assert_eq!(reloaded.llm_api_key, "llm-secret");
        assert!(reloaded.jev_api_key.is_empty());
        assert_eq!(reloaded.openai_tts.api_key, "tts-secret");
    }

    #[test]
    fn empty_input_and_storage_failures_preserve_saved_keys() {
        let store = mock_store();
        store.save(ApiKey::Llm, " old-secret ").unwrap();
        assert!(store.save(ApiKey::Llm, " \t").is_err());
        fail_next(&store, ApiKey::Llm);
        assert!(store.save(ApiKey::Llm, "new-secret").is_err());
        fail_next(&store, ApiKey::Llm);
        assert!(store.delete(ApiKey::Llm).is_err());
        assert_eq!(store.read(ApiKey::Llm).unwrap().as_deref(), Some("old-secret"));
    }

    #[test]
    fn legacy_key_is_copied_once_and_deletion_removes_both_versions() {
        let mut store = mock_store();
        store.legacy_entries = Some(mock_store().entries);
        let legacy = store.legacy_entries.as_ref().unwrap()[ApiKey::Llm.index()].as_ref().unwrap();
        legacy.set_password("legacy-secret").unwrap();
        assert_eq!(store.read(ApiKey::Llm).unwrap().as_deref(), Some("legacy-secret"));
        // 移行後は、旧項目が読めなくても新項目だけで読み込める。
        let mock = legacy.get_credential().downcast_ref::<MockCredential>().unwrap();
        mock.set_error(Error::NoStorageAccess(Box::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied))));
        assert_eq!(store.read(ApiKey::Llm).unwrap().as_deref(), Some("legacy-secret"));
        assert!(legacy.get_password().is_err());
        store.delete(ApiKey::Llm).unwrap();
        assert_eq!(store.read(ApiKey::Llm).unwrap(), None);
    }

    #[test]
    fn one_load_failure_does_not_prevent_other_keys_from_loading() {
        let store = mock_store();
        store.save(ApiKey::Llm, "llm-secret").unwrap();
        store.save(ApiKey::Jev, "jev-secret").unwrap();
        fail_next(&store, ApiKey::Llm);
        let mut settings = AppSettings::default();
        let errors = store.load(&mut settings);
        assert!(errors[0].is_some());
        assert_eq!(errors[1..], [None, None]);
        assert!(settings.llm_api_key.is_empty());
        assert_eq!(settings.jev_api_key, "jev-secret");
        assert!(settings.openai_tts.api_key.is_empty());
        // 自動読み込みに失敗しても、明示的な読み込みで再試行できる。
        assert_eq!(store.read(ApiKey::Llm).unwrap().as_deref(), Some("llm-secret"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn automatic_access_suppresses_dialogs_and_restores_interaction_after_failure() {
        use security_framework::os::macos::keychain::SecKeychain;

        let original = keychain_access(true, SecKeychain::user_interaction_allowed).unwrap().unwrap();
        let outcome = keychain_access(false, || {
            assert!(!SecKeychain::user_interaction_allowed().unwrap());
            Err::<(), _>("読み込み失敗")
        })
        .unwrap();
        assert!(outcome.is_err());
        keychain_access(true, || {
            assert_eq!(SecKeychain::user_interaction_allowed().unwrap(), original);
        })
        .unwrap();
    }
}
