use susync::SuspendFuture;
use susync_macros::suspend;

fn test_fn_empty(_a: i32, func: impl FnOnce()) {
    func();
}

#[tokio::test]
async fn macro_empty_args() {
    let fut: SuspendFuture<()> = suspend!(test_fn_empty(42, || {
        println!()
    }));
    fut.await.expect("result must be Ok");
}

fn test_multiple(a: i32, func: impl FnOnce(i32, f32)) {
    func(a, 69.0);
}

#[tokio::test]
async fn macro_multiple_args() {
    let fut: SuspendFuture<(i32, f32)> = suspend!(test_multiple(42, |integer, float| {
        println!("args: ({}, {})", integer, float);
    }));

    let res = fut.await.expect("result must be Ok");
    assert_eq!(res, (42, 69.0));
}

#[tokio::test]
async fn macro_name_collision() {
    let _fut: SuspendFuture<(i32, f32)> = suspend!(test_multiple(42, |handle, float| {
        println!("args: ({}, {})", handle, float);
    }));
}

#[tokio::test]
async fn macro_ignore_arg() {
    let fut: SuspendFuture<i32> = suspend!(test_multiple(42, |integer, _| {
        println!("args: ({}, _)", integer);
    }));
    let res = fut.await.expect("result must be OK");
    assert_eq!(res, 42);
}

fn test_return(a: i32, func: impl FnOnce(i32) -> i32) {
    assert_eq!(func(a), a);
}

#[tokio::test]
async fn macro_assert_return() {
    let fut: SuspendFuture<i32> = suspend!(test_return(42, |integer| {
        println!("args: {}", integer);
        integer
    }));
    let res = fut.await.expect("result must be OK");
    assert_eq!(res, 42);
}

#[derive(Clone, Debug)]
struct RefArgMock {
    member: i32,
}

fn test_ref(a: i32, func: impl FnOnce(i32, &RefArgMock)) -> i32 {
    func(a, &RefArgMock { member: a });
    a
}

// hack to 
#[derive(Debug)]
struct ArgMock(RefArgMock);

impl From<&RefArgMock> for ArgMock {
    fn from(value: &RefArgMock) -> Self {
        ArgMock(value.clone())
    }
}

#[tokio::test]
async fn macro_ref_args() {
    let fut: SuspendFuture<(i32, ArgMock)> = suspend!(test_ref(42, |integer, reference| {
        println!("args: ({}, {:?})", integer, reference);
    }));
    let (integer, ArgMock(reference)) = fut.await.expect("result must be OK");
    assert_eq!(integer, 42);
    assert_eq!(reference.member, 42);
}
