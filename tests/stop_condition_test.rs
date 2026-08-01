use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use wisp::crawl::{
    pages_by_callback, FnStopCondition, MaxErrors, MaxItems, MaxPages, MaxPagesByCallback,
    NeverStop, StopCondition, StopContext, Timeout,
};

fn ctx(pages: usize, items: usize, errors: usize, elapsed_secs: u64) -> StopContext {
    StopContext {
        pages,
        items,
        errors,
        in_flight: 0,
        elapsed: Duration::from_secs(elapsed_secs),
        queue_size: 10,
        callback_pages: HashMap::new(),
    }
}

#[test]
fn test_max_pages_triggered() {
    let cond = MaxPages(50);
    assert!(!cond.should_stop(&ctx(49, 0, 0, 0)));
    assert!(cond.should_stop(&ctx(50, 0, 0, 0)));
    assert!(cond.should_stop(&ctx(51, 0, 0, 0)));
}

#[test]
fn test_max_items_triggered() {
    let cond = MaxItems(10);
    assert!(!cond.should_stop(&ctx(0, 9, 0, 0)));
    assert!(cond.should_stop(&ctx(0, 10, 0, 0)));
}

#[test]
fn test_max_pages_by_callback_triggered() {
    let cond = MaxPagesByCallback::new("detail", 20);
    let mut c = ctx(50, 0, 0, 0);
    c.callback_pages.insert("detail".into(), 19);
    assert!(!cond.should_stop(&c));
    c.callback_pages.insert("detail".into(), 20);
    assert!(cond.should_stop(&c));
}

#[test]
fn test_uses_callback_pages_flag() {
    assert!(!MaxPages(10).uses_callback_pages());
    assert!(!MaxItems(10).uses_callback_pages());
    assert!(MaxPagesByCallback::new("detail", 10).uses_callback_pages());
    assert!(FnStopCondition(|_: &StopContext| true).uses_callback_pages());
    assert!(MaxPages(10)
        .and(MaxPagesByCallback::new("detail", 10))
        .uses_callback_pages());
    assert!(!MaxPages(10).and(MaxItems(5)).uses_callback_pages());
    assert!(MaxPagesByCallback::new("detail", 10)
        .not()
        .uses_callback_pages());
}

#[test]
fn test_pages_by_callback_helper() {
    let cond = pages_by_callback("detail", 3);
    let mut c = ctx(0, 0, 0, 0);
    c.callback_pages.insert("detail".into(), 2);
    assert!(!cond.should_stop(&c));
    c.callback_pages.insert("detail".into(), 3);
    assert!(cond.should_stop(&c));
}

#[test]
fn test_max_errors_triggered() {
    let cond = MaxErrors(5);
    assert!(!cond.should_stop(&ctx(0, 0, 4, 0)));
    assert!(cond.should_stop(&ctx(0, 0, 5, 0)));
}

#[test]
fn test_timeout_triggered() {
    let cond = Timeout(Duration::from_secs(60));
    assert!(!cond.should_stop(&ctx(0, 0, 0, 59)));
    assert!(cond.should_stop(&ctx(0, 0, 0, 60)));
}

#[test]
fn test_never_stop() {
    let cond = NeverStop;
    assert!(!cond.should_stop(&ctx(1000, 1000, 1000, 3600)));
}

#[test]
fn test_fn_stop_condition() {
    let cond = FnStopCondition(|c: &StopContext| c.pages > 3);
    assert!(!cond.should_stop(&ctx(3, 0, 0, 0)));
    assert!(cond.should_stop(&ctx(4, 0, 0, 0)));
}

#[test]
fn test_and_combinator() {
    // pages >= 10 AND items >= 5
    let cond: Arc<dyn StopCondition> = MaxPages(10).and(MaxItems(5));
    assert!(!cond.should_stop(&ctx(9, 5, 0, 0))); // pages 不够
    assert!(!cond.should_stop(&ctx(10, 4, 0, 0))); // items 不够
    assert!(cond.should_stop(&ctx(10, 5, 0, 0))); // 都满足
}

#[test]
fn test_or_combinator() {
    // pages >= 10 OR items >= 5
    let cond: Arc<dyn StopCondition> = MaxPages(10).or(MaxItems(5));
    assert!(!cond.should_stop(&ctx(9, 4, 0, 0)));
    assert!(cond.should_stop(&ctx(10, 4, 0, 0))); // pages 满足
    assert!(cond.should_stop(&ctx(9, 5, 0, 0))); // items 满足
}

#[test]
fn test_not_combinator() {
    // NOT pages >= 10 → pages < 10 时停
    let cond: Arc<dyn StopCondition> = MaxPages(10).not();
    assert!(cond.should_stop(&ctx(9, 0, 0, 0)));
    assert!(!cond.should_stop(&ctx(10, 0, 0, 0)));
}

#[test]
fn test_complex_combination() {
    // pages >= 50 AND timeout（elapsed >= 3600s）
    let cond: Arc<dyn StopCondition> = MaxPages(50).and(Timeout(Duration::from_secs(3600)));
    assert!(!cond.should_stop(&ctx(49, 0, 0, 3600)));
    assert!(!cond.should_stop(&ctx(50, 0, 0, 3599)));
    assert!(cond.should_stop(&ctx(50, 0, 0, 3600)));
}
