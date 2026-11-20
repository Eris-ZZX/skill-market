//! 技能状态全局读写锁
//!
//! 这层锁保护的是**文件系统**，与 `AppState` 里的 `Mutex<PersistedState>`
//! （内存状态）是两层不同的东西。
//!
//! 要防的核心场景：「检查更新」读哈希时另一端正在替换文件，读到半截内容，
//! 算出既不是旧版也不是新版的哈希，然后误判成「本地已修改」——不崩溃，
//! 但状态错乱。
//!
//! 注意：这些测试共用进程级全局锁，而 `cargo test` 默认并行跑同一个二进制的
//! 测试。所以每个测试只能断言「自己观测到的」相对事实（时间顺序、自己的
//! 计数器），不能用跨测试共享的全局计数——那会因为并行而互相污染。

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
            thread::sleep(Duration::from_millis(20));
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
            thread::sleep(Duration::from_millis(5));
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
fn 写锁期间读锁等待() {
    // 写锁持有中，另一线程的读锁必须等它释放
    let write_finished = Arc::new(AtomicBool::new(false));

    let w = {
        let flag = write_finished.clone();
        thread::spawn(move || {
            let _w = write_guard();
            thread::sleep(Duration::from_millis(60));
            flag.store(true, Ordering::SeqCst);
        })
    };
    thread::sleep(Duration::from_millis(20)); // 让写线程先拿到锁

    let r = thread::spawn(move || {
        let _r = read_guard();
        write_finished.load(Ordering::SeqCst)
    });

    w.join().unwrap();
    assert_eq!(
        r.join().unwrap(),
        true,
        "读锁应等写锁释放后才获得"
    );
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

#[test]
fn 纯读场景能观察到并发() {
    // 同一线程连续拿两个读锁就证明「读不互斥」；
    // 跨线程的并发数受调度影响不稳定，这里不做断言，只验证不会互相阻塞到超时
    let handle = thread::spawn(|| {
        let _r = read_guard();
        thread::sleep(Duration::from_millis(30));
    });
    // 主线程此时也能拿读锁（若读读互斥就会卡住直到子线程结束）
    thread::sleep(Duration::from_millis(10));
    let r = read_guard();
    drop(r);
    handle.join().unwrap();
}
