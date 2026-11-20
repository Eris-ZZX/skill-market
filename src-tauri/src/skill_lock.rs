//! 技能文件系统的全局读写锁
//!
//! `AppState` 里的 `Mutex<PersistedState>` 只保护内存状态，管不住文件系统。
//! 没有这层锁时，「检查更新」可能在另一端正替换文件时读到半截内容，
//! 算出一个既不是旧版也不是新版的哈希，然后误判成「已修改」——
//! 不崩溃，但状态错乱，比崩溃更难查。
//!
//! 锁是全局的而非按技能粒度：本机技能数量是几十的量级，细粒度锁的
//! 收益抵不上它带来的复杂度和死锁面。
//!
//! **使用约定（很重要）**：
//! - 只包**同步**的文件操作区段，绝不跨 `await` 持锁
//! - 同一个调用链里只取一次锁（标准库 `RwLock` 不可重入）

use std::sync::{OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

fn lock() -> &'static RwLock<()> {
    static LOCK: OnceLock<RwLock<()>> = OnceLock::new();
    LOCK.get_or_init(|| RwLock::new(()))
}

/// 只读技能目录时用（算哈希、扫描、列快照）
///
/// 锁中毒时取回内部值继续：某个线程 panic 了不该让整个应用永久卡死，
/// 技能文件本身也不会因为内存里的 panic 而损坏。
pub fn read_guard() -> RwLockReadGuard<'static, ()> {
    lock().read().unwrap_or_else(|e| e.into_inner())
}

/// 改动技能目录或 state.json 时用
pub fn write_guard() -> RwLockWriteGuard<'static, ()> {
    lock().write().unwrap_or_else(|e| e.into_inner())
}
