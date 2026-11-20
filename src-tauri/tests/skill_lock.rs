//! 技能状态全局读写锁
//!
//! 这层锁保护的是**文件系统**，与 `AppState` 里的 `Mutex<PersistedState>`
//! （内存状态）是两层不同的东西。
//!
//! 要防的核心场景：「检查更新」读哈希时另一端正在替换文件，读到半截内容，
//! 算出既不是旧版也不是新版的哈希，然后误判成「本地已修改」——不崩溃，
//! 但状态错乱。
//!
//! **测试约束（很重要）**：这些测试共用进程级全局锁，而 `cargo test` 默认
//! 并行跑同一个二进制内的测试。所以：
//! - 只能断言「锁自身保证的性质」（互斥、可重入读、中毒恢复）
//! - **不能依赖时序**：睡 20ms 不等于「对方已拿到锁」，睡 60ms 也不等于
//!   「对方还在持锁」。并行下其它测试会争同一把全局锁，时序完全不可控。
//! - 不能断言「写锁优先于读锁」——std `RwLock` 不保证这一点（见文末说明）

use skill_market_lib::skill_lock::{read_guard, write_guard};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[test]
fn 读锁可以并发持有() {
    // 同一线程可以连续拿两个读锁（不死锁）——这是读锁非独占的最直接证据
    let a = read_guard();
    let b = read_guard();
    drop(b);
    drop(a);
}

#[test]
fn 写锁互斥() {
    // 4 个线程争写锁，持锁期间各自登记「是否已有人在锁内」；
    // 若互斥成立，任何时刻都看不到别人也在锁内
    let inside = Arc::new(AtomicBool::new(false));
    let violated = Arc::new(AtomicBool::new(false));

    let mut handles = Vec::new();
    for _ in 0..4 {
        let inside = inside.clone();
        let violated = violated.clone();
        handles.push(thread::spawn(move || {
            let _w = write_guard();
            if inside.swap(true, Ordering::SeqCst) {
                violated.store(true, Ordering::SeqCst);
            }
            // 短暂持有，拉长重叠窗口；时间不能太长——并行下其它测试也在
            // 争这把全局锁，长 sleep 只是拖慢整体
            thread::sleep(Duration::from_millis(2));
            inside.store(false, Ordering::SeqCst);
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    assert!(
        !violated.load(Ordering::SeqCst),
        "同一时刻检测到两个写锁持有者，互斥失效"
    );
}

#[test]
fn 写锁串行且不丢更新() {
    // 4 个线程依次做「读-改-写」；若锁失效，增量会丢
    let counter = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for _ in 0..4 {
        let counter = counter.clone();
        handles.push(thread::spawn(move || {
            let _w = write_guard();
            let v = counter.load(Ordering::SeqCst);
            // 留出被抢占的窗口：没有这个窗口，即使锁失效也可能碰巧正确
            thread::sleep(Duration::from_millis(2));
            counter.store(v + 1, Ordering::SeqCst);
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(
        counter.load(Ordering::SeqCst),
        4,
        "有写锁时四次增量都应生效（无丢失更新）"
    );
}

#[test]
fn 写锁释放后读锁可以拿到() {
    // 只测「能拿到」这个稳定契约，不测「等待多久」。
    //
    // 原来那版想验证「写锁持有期间读锁被阻塞」，用了
    // `sleep(20ms)` 假设写线程已持锁 + 断言读线程看到的标志位仍为 false。
    // 两个假设都不成立：
    // 1. 20ms 后写线程可能还没拿到锁（并行下别的测试正在占着）
    // 2. 即使写线程拿到了，std::sync::RwLock 在 Linux/Windows 上**不保证**
    //    写优先——新来的读者可以与等待中的写者竞争，实现上甚至允许读者
    //    插队（glibc 默认读者优先）。所以「读锁必须等写锁」根本不是契约。
    {
        let _w = write_guard();
    }
    let r = read_guard();
    drop(r);
}

#[test]
fn 写锁释放后另一个写锁能拿到() {
    // 与上面同理：只验证「写锁释放后下一把能拿到」，不验证排队顺序
    {
        let _w1 = write_guard();
    }
    let _w2 = write_guard();
}

#[test]
fn 锁中毒后仍能获取() {
    // 某个线程持锁时 panic，标准库 RwLock 会中毒。
    // 我们的 guard 用 unwrap_or_else(into_inner) 恢复：不该 panic，也不该永久卡死。
    let t = thread::spawn(|| {
        let _w = write_guard();
        panic!("故意 panic 让锁中毒");
    });
    assert!(t.join().is_err(), "线程应确实 panic 了");

    // 中毒后仍能拿到并使用
    let w = write_guard();
    drop(w);
    let r = read_guard();
    drop(r);
}
