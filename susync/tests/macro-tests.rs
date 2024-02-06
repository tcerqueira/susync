use susync::SuspendFuture;
use susync::sus;

// ********************** HELPER FUNCTIONS **********************
// **************************************************************

fn test_fn_empty(_a: i32, func: impl FnOnce()) {
    func();
}

fn test_multiple(a: i32, func: impl FnOnce(i32, f32)) {
    func(a, 69.0);
}

fn test_return(a: i32, func: impl FnOnce(i32) -> i32) {
    assert_eq!(func(a), a);
}

fn test_multiple_closures(a: impl FnOnce(i32, i32), b: impl FnOnce(i32)) {
    a(42, 69);
    b(42);
}

fn test_closure_with_str(func: impl FnOnce(i32, &str)) {
    func(42, "test");
}

#[derive(Clone, Debug)]
struct RefArgMock {
    member: i32,
}

fn test_ref(a: i32, func: impl FnOnce(i32, &RefArgMock)) -> i32 {
    func(a, &RefArgMock { member: a });
    a
}

fn test_single_ref(a: i32, func: impl FnOnce(&RefArgMock)) -> i32 {
    func(&RefArgMock { member: a });
    a
}

// **************************** TESTS ***************************
// **************************************************************

#[tokio::test]
async fn macro_empty_args() {
    let fut: SuspendFuture<()> = sus!(test_fn_empty(42, || {
        println!()
    }));
    fut.await.expect("result must be Ok");
}

#[tokio::test]
async fn macro_multiple_args() {
    let fut: SuspendFuture<(i32, f32)> = sus!(test_multiple(42, |integer, float| {
        println!("args: ({}, {})", integer, float);
    }));

    let res = fut.await.expect("result must be Ok");
    assert_eq!(res, (42, 69.0));
}

#[tokio::test]
async fn macro_name_collision() {
    let _fut: SuspendFuture<(i32, f32)> = sus!(test_multiple(42, |handle, float| {
        println!("args: ({}, {})", handle, float);
    }));
}

#[tokio::test]
async fn macro_ignore_arg() {
    let fut: SuspendFuture<i32> = sus!(test_multiple(42, |integer, _| {
        println!("args: ({}, _)", integer);
    }));
    let res = fut.await.expect("result must be OK");
    assert_eq!(res, 42);
}

#[tokio::test]
async fn macro_ignore_all_args() {
    let fut = sus!(test_multiple(42, |_, _| {
        println!("args: (_, _)");
    }));
    fut.await.expect("result must be OK");
}

#[tokio::test]
async fn macro_assert_return() {
    let fut: SuspendFuture<i32> = sus!(test_return(42, |integer| {
        println!("args: {}", integer);
        integer
    }));
    let res = fut.await.expect("result must be OK");
    assert_eq!(res, 42);
}

#[tokio::test]
async fn macro_closure_no_braces() {
    let fut = sus!(test_return(42, |x| x));
    let res = fut.await.expect("result must be OK");
    assert_eq!(res, 42);
}

#[tokio::test]
async fn macro_closure_no_body() {
    let fut = sus!(test_multiple(42, |a, b| {}));
    let res = fut.await.expect("result must be OK");
    assert_eq!(res, (42, 69.0));
}

#[tokio::test]
async fn macro_move_closure() {
    let fut = sus!(test_multiple(42, move |_, _| {
        println!("args: (_, _)");
    }));
    fut.await.expect("result must be OK");
}

#[tokio::test]
async fn macro_last_closure() {
    let fut = sus!(test_multiple_closures(|_a, _b| {}, |x| {
        println!("args: {x}");
    }));
    let res = fut.await.expect("result must be Ok");
    assert_eq!(res, 42);
}

#[tokio::test]
async fn macro_str_arg() {
    let fut = sus!(test_closure_with_str(|a, b| {
        println!("args: ({a}, {b})");
    }));
    let res = fut.await.expect("result must be Ok");
    assert_eq!(res, (42, "test".to_owned()));
}

#[tokio::test]
async fn macro_ref_args() {
    let fut = sus!(test_ref(42, |integer, reference| {
        println!("args: ({}, {:?})", integer, reference);
    }));
    let (integer, reference) = fut.await.expect("result must be OK");
    assert_eq!(integer, 42);
    assert_eq!(reference.member, 42);
}

#[tokio::test]
async fn macro_ref_single_arg() {
    let fut = sus!(test_single_ref(42, |reference| {
        println!("args: {:?}", reference);
    }));
    let reference = fut.await.expect("result must be OK");
    assert_eq!(reference.member, 42);
}
