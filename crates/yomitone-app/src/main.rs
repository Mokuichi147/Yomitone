// Windows でアプリを開いたときに、コンソールの窓が一緒に開かないようにする
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> naui::Result<()> {
    yomitone_app::run()
}
