// Windows 下隐藏控制台窗口（release 构建）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    skill_market_lib::run()
}
